//! Pure derivation of relay state from stored checks.

use serde::Serialize;

use crate::observe::{Check, Outcome};

const DAY: u64 = 86_400;
const UPTIME_WINDOW: u64 = 30 * DAY;
const RTT_WINDOW: u64 = DAY;
const AUTH_WINDOW: u64 = DAY;

/// Relay status derived from the latest check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The latest check received a WebSocket upgrade.
    Online,
    /// The latest check failed with a relay-side reason.
    Offline,
    /// The relay was never checked or the latest check was inconclusive.
    Unverified,
}

/// Median round-trip times over the last 24 hours, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct Rtt {
    /// Median time from start of DNS resolution to the 101 response.
    pub open: u32,
    /// Median time from sending a `REQ` to the first `EVENT` or `EOSE`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read: Option<u32>,
}

/// The derived state of a single relay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelayState {
    /// Status from the latest check.
    pub status: Status,
    /// Time-weighted 30-day uptime ratio, excluding unverified periods.
    /// `None` when no online or offline check exists.
    pub uptime: Option<f64>,
    /// RTT medians over the last 24 hours.
    pub rtt: Option<Rtt>,
    /// An online check in the last 24 hours observed AUTH requirements.
    pub auth_observed: bool,
}

/// Derive the state of a relay from its checks, oldest first.
#[must_use]
pub fn derive(now: u64, checks: &[Check]) -> RelayState {
    RelayState {
        status: status(checks),
        uptime: uptime(now, checks),
        rtt: rtt(now, checks),
        auth_observed: auth_observed(now, checks),
    }
}

fn status(checks: &[Check]) -> Status {
    checks
        .last()
        .map_or(Status::Unverified, |c| match c.outcome {
            Outcome::Online => Status::Online,
            Outcome::Offline(_) => Status::Offline,
            Outcome::Unverified(_) => Status::Unverified,
        })
}

fn uptime(now: u64, checks: &[Check]) -> Option<f64> {
    let window = now.saturating_sub(UPTIME_WINDOW);
    let mut timeline: Vec<(u64, bool)> = Vec::with_capacity(checks.len() + 1);
    let mut before_window = None;
    for check in checks {
        let online = match check.outcome {
            Outcome::Online => true,
            Outcome::Offline(_) => false,
            Outcome::Unverified(_) => continue,
        };
        if check.at < window {
            before_window = Some(online);
        } else {
            timeline.push((check.at, online));
        }
    }
    timeline.sort_by_key(|(at, _)| *at);
    if let Some(online) = before_window {
        timeline.insert(0, (window, online));
    }
    if timeline.is_empty() {
        return None;
    }
    let mut total = 0_u64;
    let mut up = 0_u64;
    for (i, (at, online)) in timeline.iter().enumerate() {
        let end = timeline.get(i + 1).map_or(now, |(t, _)| *t);
        let span = end.saturating_sub(*at);
        total += span;
        if *online {
            up += span;
        }
    }
    if total == 0 {
        None
    } else {
        Some(up as f64 / total as f64)
    }
}

fn rtt(now: u64, checks: &[Check]) -> Option<Rtt> {
    let cutoff = now.saturating_sub(RTT_WINDOW);
    let mut open = Vec::new();
    let mut read = Vec::new();
    for check in checks {
        if !matches!(check.outcome, Outcome::Online) || check.at < cutoff {
            continue;
        }
        if let Some(ms) = check.rtt_open {
            open.push(ms);
        }
        if let Some(ms) = check.rtt_read {
            read.push(ms);
        }
    }
    if open.is_empty() {
        return None;
    }
    Some(Rtt {
        open: median(&mut open),
        read: (!read.is_empty()).then(|| median(&mut read)),
    })
}

fn median(values: &mut [u32]) -> u32 {
    values.sort_unstable();
    let mid = values.len() / 2;
    let high = values.get(mid).copied().unwrap_or(0);
    if values.len() % 2 == 1 {
        return high;
    }
    let low = values.get(mid.saturating_sub(1)).copied().unwrap_or(0);
    (low + high).div_ceil(2)
}

fn auth_observed(now: u64, checks: &[Check]) -> bool {
    let cutoff = now.saturating_sub(AUTH_WINDOW);
    checks
        .iter()
        .any(|c| c.at >= cutoff && matches!(c.outcome, Outcome::Online) && c.auth)
}
