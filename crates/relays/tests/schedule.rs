//! Due-rule and backoff tests.

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

use std::collections::BTreeMap;

use relays::schedule;
use relays_core::observe::{Check, Fault, Obstacle, Outcome};
use relays_core::url::RelayUrl;

const HOUR: u64 = 3600;

fn url(raw: &str) -> RelayUrl {
    RelayUrl::parse(raw).unwrap()
}

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

#[test]
fn never_checked_is_due() {
    assert_eq!(schedule::next_due(&[]), 0);
}

#[test]
fn online_rechecks_in_55_minutes() {
    let history = [check(1000, Outcome::Online)];
    assert_eq!(schedule::next_due(&history), 1000 + 55 * 60);
    let history = [check(1000, Outcome::Unverified(Obstacle::Blocked))];
    assert_eq!(schedule::next_due(&history), 1000 + 55 * 60);
}

#[test]
fn offline_backs_off_exponentially() {
    let one = [check(1000, Outcome::Offline(Fault::Timeout))];
    assert_eq!(schedule::next_due(&one), 1000 + HOUR);
    let two = [
        check(500, Outcome::Offline(Fault::Timeout)),
        check(1000, Outcome::Offline(Fault::Timeout)),
    ];
    assert_eq!(schedule::next_due(&two), 1000 + 2 * HOUR);
    let three = [
        check(100, Outcome::Offline(Fault::Timeout)),
        check(500, Outcome::Offline(Fault::Timeout)),
        check(1000, Outcome::Offline(Fault::Timeout)),
    ];
    assert_eq!(schedule::next_due(&three), 1000 + 4 * HOUR);
    // An online or unverified check resets the streak.
    let reset = [
        check(100, Outcome::Offline(Fault::Timeout)),
        check(500, Outcome::Unverified(Obstacle::Blocked)),
        check(1000, Outcome::Offline(Fault::Timeout)),
    ];
    assert_eq!(schedule::next_due(&reset), 1000 + HOUR);
}

#[test]
fn backoff_is_capped_at_24_hours() {
    let history: Vec<Check> = (0..20)
        .map(|i| check(i * 100, Outcome::Offline(Fault::Timeout)))
        .collect();
    let last = history.last().unwrap().at;
    assert_eq!(schedule::next_due(&history), last + 24 * HOUR);
}

#[test]
fn due_filters_by_next_due() {
    let a = url("wss://a.example/");
    let b = url("wss://b.example/");
    let c = url("wss://c.example/");
    let mut history = BTreeMap::new();
    history.insert(a.clone(), vec![check(0, Outcome::Online)]); // due at 3300
    history.insert(b.clone(), vec![check(0, Outcome::Offline(Fault::Timeout))]); // due at 3600
    let urls = vec![a.clone(), b.clone(), c.clone()];
    let due = schedule::due(3300, &urls, &history);
    assert_eq!(due, vec![a.clone(), c.clone()]);
    let due = schedule::due(3600, &urls, &history);
    assert_eq!(due, vec![a, b, c]);
}
