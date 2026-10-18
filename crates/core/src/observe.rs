//! Observation types produced by probes and stored in SQLite.

use serde::{Deserialize, Serialize};
use strum::{Display, EnumString, IntoStaticStr};

/// The outcome of a single check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", content = "reason", rename_all = "lowercase")]
pub enum Outcome {
    /// The relay accepted the WebSocket upgrade.
    Online,
    /// The check failed for a reason attributable to the relay.
    Offline(Fault),
    /// The check could not complete for a reason attributable to the
    /// monitor's environment (for example a WAF blocking the exit IP).
    Unverified(Obstacle),
}

/// Failure reasons attributable to the relay.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    EnumString,
    Display,
    IntoStaticStr,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum Fault {
    /// NXDOMAIN or no A/AAAA records.
    DnsNxdomain,
    /// The host resolved to an address rejected by the address policy.
    PrivateAddress,
    /// The TCP connection was refused or reset.
    Refused,
    /// A probe phase timed out.
    Timeout,
    /// The certificate was invalid or the TLS handshake failed.
    Tls,
    /// The upgrade response was a redirect (`Location` is recorded).
    Moved,
    /// The upgrade response was not a WebSocket upgrade: 2xx, or a 4xx other
    /// than 401/403/429.
    NotWebsocket,
    /// The upgrade response was a 5xx.
    ServerError,
    /// The WebSocket handshake violated the protocol.
    Protocol,
    /// Any other IO error.
    Network,
}

/// Reasons a check could not be completed from the monitor's side.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    EnumString,
    Display,
    IntoStaticStr,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum Obstacle {
    /// The upgrade response was 401, 403 or 429, usually a WAF blocking the
    /// datacenter exit.
    Blocked,
    /// SERVFAIL, resolver timeout or another transient DNS failure.
    DnsTransient,
}

impl Outcome {
    /// The outcome column value: `online`, `offline` or `unverified`.
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline(_) => "offline",
            Self::Unverified(_) => "unverified",
        }
    }

    /// The reason code, when the outcome is not `online`.
    #[must_use]
    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::Online => None,
            Self::Offline(fault) => Some(fault.into()),
            Self::Unverified(obstacle) => Some(obstacle.into()),
        }
    }
}

/// A single recorded check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// When the check ran, unix seconds.
    pub at: u64,
    /// What happened, flattened to `outcome` plus an optional `reason`.
    #[serde(flatten)]
    pub outcome: Outcome,
    /// HTTP status of the upgrade response, for `moved`, `not-websocket`,
    /// `server-error` and `blocked`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// `Location` header of a `moved` response, kept as a fix suggestion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Milliseconds from start of DNS resolution to the 101 response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt_open: Option<u32>,
    /// Milliseconds from sending the `REQ` to the first `EVENT` or `EOSE`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt_read: Option<u32>,
    /// An `AUTH` message or `CLOSED auth-required:` was observed.
    pub auth: bool,
}
