//! Relay directory monitor: probing, storage, scheduling and publication.

#![expect(
    unused_crate_dependencies,
    reason = "dependencies are shared between the lib and bin targets"
)]

use std::time::{SystemTime, UNIX_EPOCH};

pub mod probe;
pub mod publish;
pub mod schedule;
pub mod store;

/// Current time as unix seconds.
#[must_use]
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
