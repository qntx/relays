//! Command-line interface for the relay directory monitor.

#![expect(
    unused_crate_dependencies,
    reason = "dependencies are shared between the lib and bin targets"
)]

use std::io::{Write as _, stdout};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use relays::probe::Prober;
use relays::probe::resolve::AddressPolicy;
use relays::publish::Publisher;
use relays::schedule;
use relays::store::Store;
use relays_core::curation::Curation;
use relays_core::nip11::RelayInfo;
use relays_core::report::Report;
use relays_core::url::RelayUrl;
use tracing::error;
use tracing_subscriber::EnvFilter;

/// Relay directory monitor.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate the curation file and report all errors.
    Validate {
        /// Path to the curation file.
        #[arg(long, env = "RELAYS_CURATION", default_value = "relays.toml")]
        curation: PathBuf,
    },
    /// Probe a single relay and print its check and NIP-11 info as JSON.
    Probe {
        /// Relay URL (`wss://` assumed when the scheme is missing).
        url: String,
    },
    /// Run the monitoring loop.
    Run {
        /// Path to the curation file.
        #[arg(long, env = "RELAYS_CURATION", default_value = "relays.toml")]
        curation: PathBuf,
        /// Path to the SQLite database.
        #[arg(long, env = "RELAYS_DB", default_value = "relays.db")]
        db: PathBuf,
        /// Artefact store URL: `file:///path` or `s3://bucket`.
        #[arg(long, env = "RELAYS_STORE")]
        store: String,
        /// Run a single round and exit.
        #[arg(long)]
        once: bool,
    },
    /// Render a remote report.json as a Markdown table.
    Report {
        /// Report source: an http(s) URL or a local file path.
        source: String,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let result = match Cli::parse().command {
        Command::Validate { curation } => validate(&curation),
        Command::Probe { url } => probe(&url).await,
        Command::Run {
            curation,
            db,
            store,
            once,
        } => run(&curation, &db, &store, once).await,
        Command::Report { source } => report(&source).await,
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn out(text: &str) -> Result<()> {
    let mut lock = stdout().lock();
    lock.write_all(text.as_bytes())?;
    if !text.ends_with('\n') {
        lock.write_all(b"\n")?;
    }
    Ok(())
}

fn load_curation(path: &Path) -> Result<Curation> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Curation::parse(&text).map_err(|e| anyhow::anyhow!("{e}"))
}

fn validate(curation: &Path) -> Result<()> {
    let curation = load_curation(curation)?;
    out(&format!(
        "ok: {} collections, {} relays",
        curation.collections.len(),
        curation.relays.len()
    ))
}

async fn probe(url: &str) -> Result<()> {
    let url = RelayUrl::parse(url).context("invalid relay URL")?;
    let prober = Prober::new(AddressPolicy::PublicOnly)?;
    let check = prober.check(&url).await;
    let mut fields = serde_json::Map::new();
    fields.insert("check".to_owned(), serde_json::to_value(&check)?);
    match prober.nip11(&url).await {
        Ok(doc) => {
            let info = serde_json::from_str::<serde_json::Value>(&doc)
                .ok()
                .map(|doc| RelayInfo::extract(&doc));
            fields.insert("nip11".to_owned(), serde_json::json!(info));
        }
        Err(e) => {
            fields.insert(
                "nip11_error".to_owned(),
                serde_json::json!(format!("{e:#}")),
            );
        }
    }
    out(&serde_json::to_string_pretty(&serde_json::Value::Object(
        fields,
    ))?)
}

async fn run(curation_path: &Path, db: &Path, store: &str, once: bool) -> Result<()> {
    let store_db = Store::open(db)?;
    let mut publisher = Publisher::new(store)?;
    let prober = Prober::new(AddressPolicy::PublicOnly)?;
    let mut curation = load_curation(curation_path)?;
    let mut ticks = tokio::time::interval(schedule::ROUND_INTERVAL);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let shutdown = shutdown();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = ticks.tick() => {}
            () = &mut shutdown => return Ok(()),
        }
        tokio::select! {
            result = schedule::run_once(&curation, &store_db, &prober, &mut publisher) => {
                match result {
                    Ok(status) => tracing::info!(
                        relays = status.relays,
                        online = status.online,
                        offline = status.offline,
                        unverified = status.unverified,
                        duration_ms = status.duration_ms,
                        "round complete"
                    ),
                    Err(e) if once => return Err(e),
                    Err(e) => error!("round failed: {e:#}"),
                }
            }
            () = &mut shutdown => return Ok(()),
        }
        if once {
            return Ok(());
        }
        match load_curation(curation_path) {
            Ok(next) => curation = next,
            Err(e) => error!("curation reload failed, keeping previous: {e:#}"),
        }
    }
}

#[cfg(unix)]
async fn shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
        }
        Err(_) => drop(tokio::signal::ctrl_c().await),
    }
}

#[cfg(not(unix))]
async fn shutdown() {
    drop(tokio::signal::ctrl_c().await);
}

async fn report(source: &str) -> Result<()> {
    let body = if source.starts_with("http://") || source.starts_with("https://") {
        let prober = Prober::new(AddressPolicy::PublicOnly)?;
        prober
            .http()
            .get(source)
            .send()
            .await
            .with_context(|| format!("failed to fetch {source}"))?
            .error_for_status()
            .with_context(|| format!("{source} returned an error"))?
            .text()
            .await
            .context("failed to read report body")?
    } else {
        std::fs::read_to_string(source).with_context(|| format!("failed to read {source}"))?
    };
    let report: Report = serde_json::from_str(&body).context("failed to parse report")?;
    if report.relays.is_empty() {
        return Ok(());
    }
    let mut text = String::from("| Relay | Reason | Offline for | HTTP | Suggested URL |\n");
    text.push_str("| --- | --- | --- | --- | --- |\n");
    for entry in &report.relays {
        let days = report.generated_at.saturating_sub(entry.offline_since) / 86_400;
        let status = entry
            .http_status
            .map_or_else(|| "-".to_owned(), |s| s.to_string());
        let location = entry.location.as_deref().unwrap_or("-");
        let row = format!(
            "| `{}` | {} | {} days | {} | {} |\n",
            entry.url, entry.reason, days, status, location,
        );
        text.push_str(&row);
    }
    out(&text)
}
