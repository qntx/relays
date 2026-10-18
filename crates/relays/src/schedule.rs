//! Round scheduling: due rules, exponential backoff and the monitoring cycle.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use futures::StreamExt;
use relays_core::curation::Curation;
use relays_core::directory::{self, Directory, Observed};
use relays_core::nip11::RelayInfo;
use relays_core::observe::{Check, Outcome};
use relays_core::report::{Report, report_entry};
use relays_core::url::RelayUrl;
use serde::Serialize;
use tracing::{debug, info};

use crate::now_unix;
use crate::probe::Prober;
use crate::publish::Publisher;
use crate::store::Store;

const HOUR: u64 = 3600;
const DAY: u64 = 86_400;
/// Retention for raw observations.
pub const RETENTION: u64 = 35 * DAY;
/// Interval between NIP-11 fetch attempts per relay.
pub const NIP11_INTERVAL: u64 = 6 * HOUR;
/// How long the daemon sleeps between rounds.
pub const ROUND_INTERVAL: Duration = Duration::from_secs(3600);

/// The run-status artefact published at `v1/status.json`.
#[derive(Debug, Serialize)]
pub struct MonitorStatus {
    /// Generation time, unix seconds.
    pub generated_at: u64,
    /// Monitor software version.
    pub version: String,
    /// Monitored relay count.
    pub relays: usize,
    /// Relays currently online.
    pub online: usize,
    /// Relays currently offline.
    pub offline: usize,
    /// Relays currently unverified.
    pub unverified: usize,
    /// Wall-clock duration of the round, milliseconds.
    pub duration_ms: u64,
}
/// Recheck delay after an online or unverified check.
const RETRY_ONLINE: u64 = 55 * 60;
/// Backoff base after a failed check.
const RETRY_BASE: u64 = HOUR;
/// Backoff ceiling after repeated failures.
const RETRY_MAX: u64 = 24 * HOUR;
/// Number of hosts checked in parallel.
const CONCURRENCY: usize = 32;

/// Earliest unix second at which `history`'s relay becomes due.
///
/// Never-checked relays are due immediately, online/unverified outcomes
/// wait 55 minutes, and offline outcomes back off `2^(n-1)` hours capped at
/// 24 hours, where `n` is the current consecutive-failure streak.
#[must_use]
pub fn next_due(history: &[Check]) -> u64 {
    let Some(last) = history.last() else {
        return 0;
    };
    match last.outcome {
        Outcome::Online | Outcome::Unverified(_) => last.at.saturating_add(RETRY_ONLINE),
        Outcome::Offline(_) => last.at.saturating_add(backoff(history)),
    }
}

fn backoff(history: &[Check]) -> u64 {
    let streak = history
        .iter()
        .rev()
        .take_while(|c| matches!(c.outcome, Outcome::Offline(_)))
        .count();
    let shift = u32::try_from(streak.saturating_sub(1)).unwrap_or(u32::MAX);
    let multiplier = 1_u64.checked_shl(shift).unwrap_or(u64::MAX);
    RETRY_BASE.saturating_mul(multiplier).min(RETRY_MAX)
}

/// Relays due for a check at `now`, in the order of `urls`.
#[must_use]
pub fn due(now: u64, urls: &[RelayUrl], history: &BTreeMap<RelayUrl, Vec<Check>>) -> Vec<RelayUrl> {
    urls.iter()
        .filter(|url| next_due(history.get(*url).map_or(&[], Vec::as_slice)) <= now)
        .cloned()
        .collect()
}

/// Run one monitoring round.
///
/// Probes due relays (serial per host, 32 hosts in parallel), refreshes
/// due NIP-11 documents, prunes old rows, then derives and publishes the
/// `v1` artefacts against a fresh timestamp and reloaded window.
///
/// # Errors
///
/// Returns an error on storage or publication failure; individual relay
/// failures are recorded, not propagated.
pub async fn run_once(
    curation: &Curation,
    store: &Store,
    prober: &Prober,
    publisher: &mut Publisher,
) -> Result<MonitorStatus> {
    let started = Instant::now();
    let now = now_unix();
    let floor = now.saturating_sub(RETENTION);
    let urls: Vec<RelayUrl> = curation.urls().into_iter().cloned().collect();
    let history = store.checks_since(floor).await?;
    let attempts = store.nip11_attempts().await?;
    let due = due(now, &urls, &history);
    info!(due = due.len(), total = urls.len(), "checking due relays");
    run_checks(store, prober, &due, &attempts).await;
    store.prune(floor).await?;

    let finished = now_unix();
    let window = store.checks_since(floor).await?;
    let documents = store.nip11_documents().await?;
    let infos: BTreeMap<RelayUrl, RelayInfo> = documents
        .iter()
        .filter_map(|(url, raw)| {
            serde_json::from_str::<serde_json::Value>(raw)
                .ok()
                .map(|doc| (url.clone(), RelayInfo::extract(&doc)))
        })
        .collect();
    let observed: BTreeMap<RelayUrl, Observed<'_>> = urls
        .iter()
        .map(|url| {
            (
                url.clone(),
                Observed {
                    checks: window.get(url).map_or(&[], Vec::as_slice),
                    info: infos.get(url),
                },
            )
        })
        .collect();
    let directory = directory::build(finished, curation, &observed);
    let report = Report {
        generated_at: finished,
        relays: urls
            .iter()
            .filter_map(|url| {
                report_entry(finished, url, window.get(url).map_or(&[], Vec::as_slice))
            })
            .collect(),
    };
    let mut online = 0_usize;
    let mut offline = 0_usize;
    let mut unverified = 0_usize;
    for relay in &directory.relays {
        match relay.status {
            relays_core::derive::Status::Online => online += 1,
            relays_core::derive::Status::Offline => offline += 1,
            relays_core::derive::Status::Unverified => unverified += 1,
        }
    }
    let status = MonitorStatus {
        generated_at: finished,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        relays: directory.relays.len(),
        online,
        offline,
        unverified,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    };
    publish(publisher, &directory, &status, &report).await?;
    Ok(status)
}

async fn run_checks(
    store: &Store,
    prober: &Prober,
    due: &[RelayUrl],
    attempts: &BTreeMap<RelayUrl, u64>,
) {
    let mut groups: BTreeMap<String, Vec<RelayUrl>> = BTreeMap::new();
    for url in due {
        groups
            .entry(url.host().to_owned())
            .or_default()
            .push(url.clone());
    }
    futures::stream::iter(groups.into_values().map(|urls| {
        let store = store.clone();
        async move {
            for url in urls {
                check_and_store(&store, prober, &url, attempts).await;
            }
        }
    }))
    .buffer_unordered(CONCURRENCY)
    .collect::<()>()
    .await;
}

async fn check_and_store(
    store: &Store,
    prober: &Prober,
    url: &RelayUrl,
    attempts: &BTreeMap<RelayUrl, u64>,
) {
    let check = prober.check(url).await;
    if let Err(e) = store.insert_check(url, &check).await {
        tracing::warn!(url = %url, error = %e, "failed to store check");
    }
    fetch_nip11(store, prober, url, attempts).await;
}

async fn fetch_nip11(
    store: &Store,
    prober: &Prober,
    url: &RelayUrl,
    attempts: &BTreeMap<RelayUrl, u64>,
) {
    let now = now_unix();
    let due = attempts
        .get(url)
        .is_none_or(|at| now.saturating_sub(*at) >= NIP11_INTERVAL);
    if !due {
        return;
    }
    let document = match prober.nip11(url).await {
        Ok(document) => Some(document),
        Err(e) => {
            debug!(url = %url, error = %e, "NIP-11 fetch failed");
            None
        }
    };
    if let Err(e) = store.record_nip11(url, now, document.as_deref()).await {
        tracing::warn!(url = %url, error = %e, "failed to store NIP-11 record");
    }
}

async fn publish(
    publisher: &mut Publisher,
    directory: &Directory,
    status: &MonitorStatus,
    report: &Report,
) -> Result<()> {
    let body = serde_json::to_vec(directory).context("failed to serialise directory")?;
    publisher.put("v1/catalog.json", body.clone()).await?;
    publisher.put("v1/index.json", body).await?;
    let schema = serde_json::to_vec(&directory::schema()).context("failed to serialise schema")?;
    publisher.put("v1/schema.json", schema).await?;
    let status_body = serde_json::to_vec(status).context("failed to serialise status")?;
    publisher.put("v1/status.json", status_body).await?;
    let report_body = serde_json::to_vec(report).context("failed to serialise report")?;
    publisher.put("v1/report.json", report_body).await?;
    Ok(())
}
