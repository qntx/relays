//! `v1` directory artefact types and projection.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Serialize;

use crate::curation::{Curation, Localized};
use crate::derive::{self, RelayState, Rtt, Status};
use crate::nip11::RelayInfo;
use crate::observe::Check;
use crate::url::RelayUrl;

/// A `v1` catalogue or index document.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Directory {
    /// Directory format version, always `1`.
    pub version: u8,
    /// Generation time, unix seconds.
    pub generated_at: u64,
    /// Curated collections in curation order.
    pub collections: Vec<DirectoryCollection>,
    /// Relay entries sorted by URL.
    pub relays: Vec<DirectoryRelay>,
}

/// A collection entry in a directory document.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DirectoryCollection {
    /// Collection identifier.
    pub id: String,
    /// Collection kind.
    #[serde(flatten)]
    pub kind: CollectionKind,
    /// Member URLs in editorial order.
    #[schemars(with = "Vec<String>")]
    pub relays: Vec<RelayUrl>,
}

/// Collection kind, serialised as `"kind": "curated"`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CollectionKind {
    /// A manually curated collection.
    Curated {
        /// Localized collection name.
        name: Localized,
        /// Localized collection description.
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<Localized>,
    },
}

/// Access requirements derived from NIP-11 limitations and observed AUTH.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the requirement flags are orthogonal schema fields"
)]
pub struct Requirements {
    /// Authentication required (NIP-11 `auth_required` or observed AUTH).
    pub auth: bool,
    /// Payment required.
    pub payment: bool,
    /// Proof of work required.
    pub pow: bool,
    /// Writes restricted to an allowlist.
    pub restricted_writes: bool,
}

/// A relay entry in a directory document.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DirectoryRelay {
    /// Canonical relay URL.
    #[schemars(with = "String")]
    pub url: RelayUrl,
    /// Display name: curation override, else NIP-11.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Localized description: curation override, else NIP-11 under `und`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Localized>,
    /// Icon URL (https only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Status from the latest check.
    pub status: Status,
    /// Time-weighted 30-day uptime ratio, three decimals.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime: Option<f64>,
    /// RTT medians over the last 24 hours.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt: Option<Rtt>,
    /// Supported NIPs reported by NIP-11.
    pub nips: Vec<u16>,
    /// Access requirements.
    pub requirements: Requirements,
    /// Declared language codes.
    pub languages: Vec<String>,
    /// Not-safe-for-work flag.
    pub nsfw: bool,
    /// `round(100 * uptime)`; `0` without observations.
    pub score: u8,
}

/// Observation inputs for [`build`].
#[derive(Debug)]
pub struct Observed<'a> {
    /// Stored checks for this relay.
    pub checks: &'a [Check],
    /// Extracted NIP-11 information, when a document is stored.
    pub info: Option<&'a RelayInfo>,
}

/// Project the directory document from curation and stored observations.
#[must_use]
pub fn build(
    now: u64,
    curation: &Curation,
    observed: &BTreeMap<RelayUrl, Observed<'_>>,
) -> Directory {
    let collections = curation
        .collections
        .iter()
        .map(|c| DirectoryCollection {
            id: c.id.clone(),
            kind: CollectionKind::Curated {
                name: c.name.clone(),
                description: c.description.clone(),
            },
            relays: c.relays.clone(),
        })
        .collect();
    let relays = curation
        .urls()
        .into_iter()
        .map(|url| relay(now, curation, observed, url))
        .collect();
    Directory {
        version: 1,
        generated_at: now,
        collections,
        relays,
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "uptime is in [0, 1], so the rounded score is always 0..=100"
)]
fn relay(
    now: u64,
    curation: &Curation,
    observed: &BTreeMap<RelayUrl, Observed<'_>>,
    url: &RelayUrl,
) -> DirectoryRelay {
    let empty: &[Check] = &[];
    let entry = observed.get(url);
    let checks = entry.map_or(empty, |o| o.checks);
    let info = entry.and_then(|o| o.info);
    let over = curation.relays.get(url);
    let RelayState {
        status,
        uptime,
        rtt,
        auth_observed,
    } = derive::derive(now, checks);
    let uptime = uptime.map(|u| (u * 1000.0).round() / 1000.0);
    DirectoryRelay {
        url: url.clone(),
        name: over
            .and_then(|o| o.name.clone())
            .or_else(|| info.and_then(|i| i.name.clone())),
        description: over.and_then(|o| o.description.clone()).or_else(|| {
            info.and_then(|i| i.description.clone())
                .map(|d| Localized::new(BTreeMap::from([("und".to_owned(), d)])))
        }),
        icon: info.and_then(|i| i.icon.clone()),
        status,
        uptime,
        rtt,
        nips: info.map_or_else(Vec::new, |i| i.nips.clone()),
        requirements: Requirements {
            auth: info.is_some_and(|i| i.auth_required) || auth_observed,
            payment: info.is_some_and(|i| i.payment_required),
            pow: info.is_some_and(|i| i.pow),
            restricted_writes: info.is_some_and(|i| i.restricted_writes),
        },
        languages: over.map_or_else(Vec::new, |o| o.languages.clone()),
        nsfw: over.is_some_and(|o| o.nsfw),
        score: uptime.map_or(0, |u| (100.0 * u).round() as u8),
    }
}

/// The JSON Schema for directory documents, published as `v1/schema.json`.
#[must_use]
pub fn schema() -> schemars::Schema {
    schemars::schema_for!(Directory)
}
