//! SQLite store tests.

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

use relays::store::Store;
use relays_core::observe::{Check, Fault, Obstacle, Outcome};
use relays_core::url::RelayUrl;

fn url(raw: &str) -> RelayUrl {
    RelayUrl::parse(raw).unwrap()
}

fn check(at: u64, outcome: Outcome) -> Check {
    Check {
        at,
        outcome,
        http_status: None,
        location: None,
        rtt_open: Some(10),
        rtt_read: Some(5),
        auth: false,
    }
}

#[tokio::test]
async fn insert_and_read_back() {
    let store = Store::open_memory().unwrap();
    let a = url("wss://a.example/");
    let mut moved = check(200, Outcome::Offline(Fault::Moved));
    moved.http_status = Some(301);
    moved.location = Some("wss://new.example/".to_owned());
    moved.auth = true;
    for c in [
        check(100, Outcome::Online),
        moved.clone(),
        check(300, Outcome::Unverified(Obstacle::Blocked)),
    ] {
        store.insert_check(&a, &c).await.unwrap();
    }
    let all = store.checks_since(0).await.unwrap();
    let checks = &all[&a];
    assert_eq!(checks.len(), 3);
    assert_eq!(checks[0].outcome, Outcome::Online);
    assert_eq!(checks[1].outcome, Outcome::Offline(Fault::Moved));
    assert_eq!(checks[1].http_status, Some(301));
    assert_eq!(checks[1].location.as_deref(), Some("wss://new.example/"));
    assert!(checks[1].auth);
    assert_eq!(checks[2].outcome, Outcome::Unverified(Obstacle::Blocked));
}

#[tokio::test]
async fn window_queries() {
    let store = Store::open_memory().unwrap();
    let a = url("wss://a.example/");
    let b = url("wss://b.example/");
    for at in [50, 150, 250] {
        store
            .insert_check(&a, &check(at, Outcome::Online))
            .await
            .unwrap();
    }
    store
        .insert_check(&b, &check(150, Outcome::Online))
        .await
        .unwrap();
    let since = store.checks_since(100).await.unwrap();
    assert_eq!(since[&a].len(), 2);
    assert_eq!(since[&b].len(), 1);
    let empty = store.checks_since(1000).await.unwrap();
    assert!(empty.is_empty());
}

#[tokio::test]
async fn prune_removes_old_rows() {
    let store = Store::open_memory().unwrap();
    let a = url("wss://a.example/");
    for at in [50, 150] {
        store
            .insert_check(&a, &check(at, Outcome::Online))
            .await
            .unwrap();
    }
    assert_eq!(store.prune(100).await.unwrap(), 1);
    let remaining = store.checks_since(0).await.unwrap();
    assert_eq!(remaining[&a].len(), 1);
    assert_eq!(remaining[&a][0].at, 150);
}

#[tokio::test]
async fn nip11_attempts_and_documents() {
    let store = Store::open_memory().unwrap();
    let a = url("wss://a.example/");
    assert!(!store.nip11_attempts().await.unwrap().contains_key(&a));
    // Failed attempt: only attempted_at is recorded.
    store.record_nip11(&a, 100, None).await.unwrap();
    assert_eq!(store.nip11_attempts().await.unwrap()[&a], 100);
    assert!(store.nip11_documents().await.unwrap().is_empty());
    // Successful fetch stores the document.
    store
        .record_nip11(&a, 200, Some(r#"{"name":"A"}"#))
        .await
        .unwrap();
    assert_eq!(store.nip11_attempts().await.unwrap()[&a], 200);
    assert_eq!(
        store.nip11_documents().await.unwrap()[&a],
        r#"{"name":"A"}"#
    );
    // Later failure keeps the stored document.
    store.record_nip11(&a, 300, None).await.unwrap();
    assert_eq!(store.nip11_attempts().await.unwrap()[&a], 300);
    assert_eq!(
        store.nip11_documents().await.unwrap()[&a],
        r#"{"name":"A"}"#
    );
}
