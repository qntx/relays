//! Lenient field extraction from NIP-11 relay information documents.

use serde::Serialize;
use serde_json::Value;
use url::Url;

const NAME_MAX_CHARS: usize = 40;
const DESCRIPTION_MAX_CHARS: usize = 280;

/// Relay metadata extracted from a NIP-11 document. Fields whose values do
/// not match the expected type are dropped individually.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the limitation flags are orthogonal NIP-11 fields"
)]
pub struct RelayInfo {
    /// Relay-reported name, sanitised to at most 40 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Relay-reported description, sanitised to at most 280 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Relay-reported icon; only `https` URLs are accepted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// `supported_nips` integers in range, deduplicated and sorted.
    pub nips: Vec<u16>,
    /// `limitation.auth_required`.
    pub auth_required: bool,
    /// `limitation.payment_required`.
    pub payment_required: bool,
    /// `limitation.restricted_writes`.
    pub restricted_writes: bool,
    /// `limitation.min_pow_difficulty` greater than zero.
    pub pow: bool,
}

impl RelayInfo {
    /// Extract the fields that are used by the directory from a parsed
    /// NIP-11 document.
    #[must_use]
    pub fn extract(document: &Value) -> Self {
        let limitation = document.get("limitation");
        let flag = |key: &str| {
            limitation
                .and_then(|l| l.get(key))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        Self {
            name: document
                .get("name")
                .and_then(Value::as_str)
                .and_then(|s| sanitize(s, NAME_MAX_CHARS)),
            description: document
                .get("description")
                .and_then(Value::as_str)
                .and_then(|s| sanitize(s, DESCRIPTION_MAX_CHARS)),
            icon: document
                .get("icon")
                .and_then(Value::as_str)
                .and_then(https_url),
            nips: nips(document.get("supported_nips")),
            auth_required: flag("auth_required"),
            payment_required: flag("payment_required"),
            restricted_writes: flag("restricted_writes"),
            pow: limitation
                .and_then(|l| l.get("min_pow_difficulty"))
                .and_then(Value::as_f64)
                .is_some_and(|n| n > 0.0),
        }
    }
}

fn nips(value: Option<&Value>) -> Vec<u16> {
    let mut nips: Vec<u16> = value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_u64().and_then(|n| u16::try_from(n).ok()))
                .collect()
        })
        .unwrap_or_default();
    nips.sort_unstable();
    nips.dedup();
    nips
}

fn https_url(text: &str) -> Option<String> {
    let text = sanitize(text, usize::MAX)?;
    let url = Url::parse(&text).ok()?;
    if url.scheme() == "https" {
        Some(text)
    } else {
        None
    }
}

/// Remove control characters, collapse whitespace runs to a single space,
/// trim; truncate to `max` characters ending in `…`. `None` when empty.
fn sanitize(text: &str, max: usize) -> Option<String> {
    let mut out = String::with_capacity(text.len().min(max));
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if ch.is_control() {
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(ch);
    }
    if out.is_empty() {
        return None;
    }
    if out.chars().count() > max {
        out = out.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
    }
    Some(out)
}
