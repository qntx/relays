//! The weekly report of persistently failing curated relays.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::observe::{Check, Fault, Outcome};
use crate::url::RelayUrl;

const DAY: u64 = 86_400;
/// Relays offline for more than this many days appear in the report.
const REPORT_OFFLINE_DAYS: u64 = 7;

/// The weekly report document published at `v1/report.json`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Report {
    /// Generation time, unix seconds.
    pub generated_at: u64,
    /// Entries for relays offline more than seven days.
    pub relays: Vec<ReportEntry>,
}

/// A single report entry.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReportEntry {
    /// The failing relay.
    #[schemars(with = "String")]
    pub url: RelayUrl,
    /// Latest failure reason.
    pub reason: Fault,
    /// First check of the current offline streak, unix seconds.
    pub offline_since: u64,
    /// HTTP status of the latest failure, when recorded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// Redirect target of a `moved` failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// A report entry for a curated relay that has been offline for more than
/// seven days since its last online check (or since the first check if it
/// was never online). `None` otherwise.
#[must_use]
pub fn report_entry(now: u64, url: &RelayUrl, checks: &[Check]) -> Option<ReportEntry> {
    let relevant: Vec<&Check> = checks
        .iter()
        .filter(|c| !matches!(c.outcome, Outcome::Unverified(_)))
        .collect();
    let last = relevant.last()?;
    let Outcome::Offline(fault) = last.outcome else {
        return None;
    };
    let last_online = relevant
        .iter()
        .rposition(|c| matches!(c.outcome, Outcome::Online));
    let (reference, since) = last_online.map_or_else(
        || (relevant.first(), relevant.first()),
        |i| (relevant.get(i), relevant.get(i + 1)),
    );
    let (Some(reference), Some(since)) = (reference, since) else {
        return None;
    };
    if now.saturating_sub(reference.at) <= REPORT_OFFLINE_DAYS * DAY {
        return None;
    }
    Some(ReportEntry {
        url: url.clone(),
        reason: fault,
        offline_since: since.at,
        http_status: last.http_status,
        location: last.location.clone(),
    })
}
