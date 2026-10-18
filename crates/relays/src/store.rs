//! SQLite observation store (WAL mode, `rusqlite_migration`).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow};
use relays_core::observe::{Check, Fault, Obstacle, Outcome};
use relays_core::url::RelayUrl;
use rusqlite::{Connection, params};
use rusqlite_migration::{M, Migrations};

static MIGRATION_LIST: &[M<'static>] = &[M::up(
    "CREATE TABLE checks (
        url         TEXT    NOT NULL,
        at          INTEGER NOT NULL,
        outcome     TEXT    NOT NULL CHECK (outcome IN ('online', 'offline', 'unverified')),
        reason      TEXT,
        http_status INTEGER,
        location    TEXT,
        rtt_open    INTEGER,
        rtt_read    INTEGER,
        auth        INTEGER NOT NULL,
        PRIMARY KEY (url, at)
    ) STRICT, WITHOUT ROWID;

    CREATE TABLE nip11 (
        url          TEXT    PRIMARY KEY,
        attempted_at INTEGER NOT NULL,
        fetched_at   INTEGER,
        document     TEXT
    ) STRICT;",
)];

/// The observation store.
#[derive(Debug, Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    /// Open (and migrate) a store at `path`, in WAL mode.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be opened or migrated.
    pub fn open(path: &Path) -> Result<Self> {
        Self::new(Connection::open(path).context("failed to open sqlite database")?)
    }

    /// Open an in-memory store; for tests.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be opened or migrated.
    pub fn open_memory() -> Result<Self> {
        Self::new(Connection::open_in_memory().context("failed to open sqlite database")?)
    }

    fn new(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Migrations::from_slice(MIGRATION_LIST)
            .to_latest(&mut conn)
            .context("failed to run migrations")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Record one check.
    ///
    /// # Errors
    ///
    /// Returns an error on database failure.
    pub async fn insert_check(&self, url: &RelayUrl, check: &Check) -> Result<()> {
        let url = url.as_str().to_owned();
        let check = check.clone();
        self.call(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO checks
                 (url, at, outcome, reason, http_status, location, rtt_open, rtt_read, auth)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    url,
                    sql_int(check.at),
                    check.outcome.kind(),
                    check.outcome.reason(),
                    check.http_status.map(i64::from),
                    check.location,
                    check.rtt_open.map(i64::from),
                    check.rtt_read.map(i64::from),
                    i64::from(check.auth),
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// All checks since `since` (unix seconds), grouped by URL, each list
    /// sorted by `at`.
    ///
    /// # Errors
    ///
    /// Returns an error on database failure.
    pub async fn checks_since(&self, since: u64) -> Result<BTreeMap<RelayUrl, Vec<Check>>> {
        self.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT url, at, outcome, reason, http_status, location, rtt_open, rtt_read, auth
                 FROM checks WHERE at >= ?1 ORDER BY url, at",
            )?;
            let rows = stmt.query_map(params![sql_int(since)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })?;
            let mut map: BTreeMap<RelayUrl, Vec<Check>> = BTreeMap::new();
            for row in rows {
                let (raw, at, kind, reason, status, location, open, read, auth) = row?;
                let url = RelayUrl::parse(&raw)
                    .map_err(|_| anyhow!("invalid relay URL in store: {raw}"))?;
                let check = Check {
                    at: sql_time(at),
                    outcome: outcome(&kind, reason.as_deref())
                        .map_err(|e| anyhow!("{e} in stored check for {raw}"))?,
                    http_status: sql_u16(status),
                    location,
                    rtt_open: sql_u32(open),
                    rtt_read: sql_u32(read),
                    auth: auth != 0,
                };
                map.entry(url).or_default().push(check);
            }
            Ok(map)
        })
        .await
    }

    /// Last NIP-11 fetch attempt time per relay.
    ///
    /// # Errors
    ///
    /// Returns an error on database failure.
    pub async fn nip11_attempts(&self) -> Result<BTreeMap<RelayUrl, u64>> {
        self.call(|conn| {
            let mut stmt = conn.prepare("SELECT url, attempted_at FROM nip11")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            let mut map = BTreeMap::new();
            for row in rows {
                let (raw, at) = row?;
                let url = RelayUrl::parse(&raw)
                    .map_err(|_| anyhow!("invalid relay URL in store: {raw}"))?;
                map.insert(url, sql_time(at));
            }
            Ok(map)
        })
        .await
    }

    /// Record a NIP-11 fetch attempt: `document` is the raw JSON on success
    /// (overwriting `fetched_at`), `None` on failure (only `attempted_at`
    /// is updated).
    ///
    /// # Errors
    ///
    /// Returns an error on database failure.
    pub async fn record_nip11(
        &self,
        url: &RelayUrl,
        attempted_at: u64,
        document: Option<&str>,
    ) -> Result<()> {
        let url = url.as_str().to_owned();
        let document = document.map(str::to_owned);
        self.call(move |conn| {
            if let Some(document) = document {
                conn.execute(
                    "INSERT INTO nip11 (url, attempted_at, fetched_at, document)
                     VALUES (?1, ?2, ?2, ?3)
                     ON CONFLICT(url) DO UPDATE SET
                        attempted_at = excluded.attempted_at,
                        fetched_at = excluded.fetched_at,
                        document = excluded.document",
                    params![url, sql_int(attempted_at), document],
                )?;
            } else {
                conn.execute(
                    "INSERT INTO nip11 (url, attempted_at) VALUES (?1, ?2)
                     ON CONFLICT(url) DO UPDATE SET attempted_at = excluded.attempted_at",
                    params![url, sql_int(attempted_at)],
                )?;
            }
            Ok(())
        })
        .await
    }

    /// Stored NIP-11 documents per relay.
    ///
    /// # Errors
    ///
    /// Returns an error on database failure.
    pub async fn nip11_documents(&self) -> Result<BTreeMap<RelayUrl, String>> {
        self.call(|conn| {
            let mut stmt =
                conn.prepare("SELECT url, document FROM nip11 WHERE document IS NOT NULL")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut map = BTreeMap::new();
            for row in rows {
                let (raw, document) = row?;
                let url = RelayUrl::parse(&raw)
                    .map_err(|_| anyhow!("invalid relay URL in store: {raw}"))?;
                map.insert(url, document);
            }
            Ok(map)
        })
        .await
    }

    /// Delete checks older than `before` (unix seconds).
    ///
    /// # Errors
    ///
    /// Returns an error on database failure.
    pub async fn prune(&self, before: u64) -> Result<usize> {
        self.call(move |conn| {
            Ok(conn.execute("DELETE FROM checks WHERE at < ?1", params![sql_int(before)])?)
        })
        .await
    }

    async fn call<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|_| anyhow!("sqlite connection lock poisoned"))?;
            f(&conn)
        })
        .await
        .context("sqlite task failed")?
    }
}

fn outcome(kind: &str, reason: Option<&str>) -> Result<Outcome> {
    Ok(match kind {
        "online" => Outcome::Online,
        "offline" => Outcome::Offline(
            reason
                .and_then(|r| r.parse::<Fault>().ok())
                .ok_or_else(|| anyhow!("missing or invalid offline reason"))?,
        ),
        "unverified" => Outcome::Unverified(
            reason
                .and_then(|r| r.parse::<Obstacle>().ok())
                .ok_or_else(|| anyhow!("missing or invalid unverified reason"))?,
        ),
        other => return Err(anyhow!("invalid outcome `{other}`")),
    })
}

fn sql_int(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn sql_time(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn sql_u16(value: Option<i64>) -> Option<u16> {
    value.and_then(|v| u16::try_from(v).ok())
}

fn sql_u32(value: Option<i64>) -> Option<u32> {
    value.and_then(|v| u32::try_from(v).ok())
}
