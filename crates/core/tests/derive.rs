//! State derivation tests.

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

use relays_core::derive::{self, Status};
use relays_core::observe::{Check, Fault, Obstacle, Outcome};

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
fn status_from_latest_check() {
    assert_eq!(derive::derive(NOW, &[]).status, Status::Unverified);
    assert_eq!(derive::derive(NOW, &[online(NOW)]).status, Status::Online);
    assert_eq!(derive::derive(NOW, &[offline(NOW)]).status, Status::Offline);
    let checks = [
        online(NOW - 10),
        check(NOW - 5, Outcome::Unverified(Obstacle::Blocked)),
    ];
    assert_eq!(derive::derive(NOW, &checks).status, Status::Unverified);
}

#[test]
fn uptime_is_time_weighted() {
    // Online for the first half of the window, offline for the second.
    let checks = [online(70 * DAY), offline(85 * DAY)];
    let state = derive::derive(NOW, &checks);
    let uptime = state.uptime.unwrap();
    assert!((uptime - 0.5).abs() < 0.001, "{uptime}");
}

#[test]
fn uptime_excludes_unverified_records() {
    // Unverified records do not create state boundaries: the online state
    // at 70d persists until the offline record at 90d.
    let checks = [
        online(70 * DAY),
        check(80 * DAY, Outcome::Unverified(Obstacle::Blocked)),
        offline(90 * DAY),
    ];
    let state = derive::derive(NOW, &checks);
    let uptime = state.uptime.unwrap();
    // online span = 20d, offline span = 10d.
    assert!((uptime - (20.0 / 30.0)).abs() < 0.001, "{uptime}");
}

#[test]
fn pre_window_observation_carries_into_window() {
    // Only a pre-window online check: uptime over the whole 30-day window.
    let checks = [online(10 * DAY)];
    let state = derive::derive(NOW, &checks);
    assert!((state.uptime.unwrap() - 1.0).abs() < 0.001);
    let checks = [offline(10 * DAY)];
    assert_eq!(derive::derive(NOW, &checks).uptime, Some(0.0));
}

#[test]
fn sparse_samples_are_time_weighted_not_event_weighted() {
    // One online at window start, then nothing until a last-moment failure:
    // uptime is ~1.0 even though half the checks are offline.
    let checks = [online(70 * DAY), offline(NOW - 60)];
    let uptime = derive::derive(NOW, &checks).uptime.unwrap();
    assert!(uptime > 0.99, "{uptime}");
}

#[test]
fn no_observations_means_no_uptime() {
    assert_eq!(derive::derive(NOW, &[]).uptime, None);
    let checks = [check(NOW, Outcome::Unverified(Obstacle::Blocked))];
    assert_eq!(derive::derive(NOW, &checks).uptime, None);
}

#[test]
fn rtt_medians_use_online_records_in_24h() {
    let mut a = online(NOW - 100);
    a.rtt_open = Some(100);
    a.rtt_read = Some(50);
    let mut b = online(NOW - 50);
    b.rtt_open = Some(300);
    b.rtt_read = Some(70);
    let mut old = online(NOW - 2 * DAY);
    old.rtt_open = Some(9999);
    old.rtt_read = Some(9999);
    let mut fail = offline(NOW - 10);
    fail.rtt_open = Some(1);
    fail.rtt_read = Some(1);
    let state = derive::derive(NOW, &[old, a, fail, b]);
    let rtt = state.rtt.unwrap();
    assert_eq!(rtt.open, 200);
    assert_eq!(rtt.read, Some(60));
}

#[test]
fn auth_observed_within_24h() {
    let mut a = online(NOW - 100);
    a.auth = true;
    assert!(derive::derive(NOW, &[a]).auth_observed);
    // Older than 24h.
    let mut b = online(NOW - 2 * DAY);
    b.auth = true;
    assert!(!derive::derive(NOW, &[b]).auth_observed);
    // Offline checks do not count.
    let mut c = offline(NOW - 10);
    c.auth = true;
    assert!(!derive::derive(NOW, &[c]).auth_observed);
}
