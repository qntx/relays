//! Report entry derivation tests.

#![expect(
    unused_crate_dependencies,
    reason = "test targets share the crate dependency set"
)]
#![allow(
    clippy::cognitive_complexity,
    clippy::indexing_slicing,
    clippy::missing_const_for_fn,
    clippy::panic,
    clippy::shadow_unrelated,
    clippy::tests_outside_test_module,
    clippy::unwrap_used,
    reason = "test binary: indexed fixtures, panic-based assertions and free #[test] functions are idiomatic"
)]

use relays_core::observe::{Check, Fault, Obstacle, Outcome};
use relays_core::report::report_entry;
use relays_core::url::RelayUrl;

const DAY: u64 = 86_400;
const NOW: u64 = 100 * DAY;

fn check(at: u64, outcome: Outcome) -> Check {
    Check {
        at,
        outcome,
        http_status: None,
        location: None,
        rtt_open: None,
        rtt_read: None,
        auth: false,
    }
}

fn online(at: u64) -> Check {
    check(at, Outcome::Online)
}

fn offline(at: u64) -> Check {
    check(at, Outcome::Offline(Fault::Timeout))
}

#[test]
fn report_entry_boundary() {
    let url = RelayUrl::parse("wss://a.example/").unwrap();
    // Offline for 8 days since the last online check -> reported.
    let checks = [
        online(NOW - 8 * DAY),
        offline(NOW - 8 * DAY + 1),
        offline(NOW),
    ];
    let entry = report_entry(NOW, &url, &checks).unwrap();
    assert_eq!(entry.reason, Fault::Timeout);
    assert_eq!(entry.offline_since, NOW - 8 * DAY + 1);
    // 7 days: not yet reported (strictly more than seven days required).
    let checks = [online(NOW - 7 * DAY), offline(NOW)];
    assert!(report_entry(NOW, &url, &checks).is_none());
    // Never online, first check 8 days ago, all offline.
    let checks = [offline(NOW - 8 * DAY), offline(NOW - DAY)];
    let entry = report_entry(NOW, &url, &checks).unwrap();
    assert_eq!(entry.offline_since, NOW - 8 * DAY);
    // Latest check online -> no entry.
    let checks = [offline(NOW - 9 * DAY), online(NOW - 8 * DAY)];
    assert!(report_entry(NOW, &url, &checks).is_none());
    // Unverified checks are invisible to the streak.
    let checks = [
        online(NOW - 9 * DAY),
        offline(NOW - 8 * DAY),
        check(NOW - 100, Outcome::Unverified(Obstacle::Blocked)),
    ];
    assert!(report_entry(NOW, &url, &checks).is_some());
}

#[test]
fn report_entry_includes_http_details() {
    let url = RelayUrl::parse("wss://a.example/").unwrap();
    let mut moved = check(NOW, Outcome::Offline(Fault::Moved));
    moved.http_status = Some(301);
    moved.location = Some("wss://new.example/".to_owned());
    let checks = [online(NOW - 8 * DAY), moved];
    let entry = report_entry(NOW, &url, &checks).unwrap();
    assert_eq!(entry.http_status, Some(301));
    assert_eq!(entry.location.as_deref(), Some("wss://new.example/"));
}
