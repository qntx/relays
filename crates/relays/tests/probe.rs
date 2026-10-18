//! Probe integration tests against an in-process mock relay.

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
    reason = "test binary: panic-based assertions and free test functions are idiomatic"
)]

mod common;

use std::net::SocketAddr;

use common::{Mode, RelayReply, spawn};
use relays::probe::Prober;
use relays::probe::resolve::AddressPolicy;
use relays::probe::ws;
use relays_core::observe::{Fault, Obstacle, Outcome};
use relays_core::url::RelayUrl;

fn url(raw: &str) -> RelayUrl {
    RelayUrl::parse(raw).unwrap()
}

fn prober() -> Prober {
    Prober::new(AddressPolicy::AllowLoopback).unwrap()
}

#[tokio::test]
async fn online_with_eose() {
    let check = prober()
        .check(&url(&spawn(Mode::WebSocket(RelayReply::Eose)).await))
        .await;
    assert_eq!(check.outcome, Outcome::Online);
    assert!(check.rtt_open.is_some());
    assert!(check.rtt_read.is_some());
    assert!(!check.auth);
}

#[tokio::test]
async fn auth_challenge_is_recorded() {
    let check = prober()
        .check(&url(
            &spawn(Mode::WebSocket(RelayReply::AuthChallenge)).await
        ))
        .await;
    assert_eq!(check.outcome, Outcome::Online);
    assert!(check.auth);
    assert!(check.rtt_read.is_some());
}

#[tokio::test]
async fn closed_auth_required_is_recorded() {
    let check = prober()
        .check(&url(&spawn(Mode::WebSocket(
            RelayReply::ClosedAuthRequired,
        ))
        .await))
        .await;
    assert_eq!(check.outcome, Outcome::Online);
    assert!(check.auth);
}

#[tokio::test]
async fn silent_relay_stays_online_without_rtt_read() {
    let check = prober()
        .check(&url(&spawn(Mode::WebSocket(RelayReply::Silent)).await))
        .await;
    assert_eq!(check.outcome, Outcome::Online);
    assert!(check.rtt_open.is_some());
    assert_eq!(check.rtt_read, None);
    assert!(!check.auth);
}

#[tokio::test]
async fn close_sends_close_message_and_frame() {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let check = prober()
        .check(&url(&spawn(Mode::WebSocketClose(tx)).await))
        .await;
    assert_eq!(check.outcome, Outcome::Online);
    let (close_msg, close_frame) = rx.await.unwrap();
    assert!(close_msg, "expected a [\"CLOSE\", <sub>] message");
    assert!(close_frame, "expected a WebSocket close frame");
}

#[tokio::test]
async fn connect_falls_back_to_next_address() {
    // A closed port first, then a reachable mock: the connection succeeds.
    let closed = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        addr
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let open = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    });
    let addrs: &[SocketAddr] = &[closed, open];
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    assert!(ws::connect(addrs, deadline).await.is_ok());
}

#[tokio::test]
async fn tls_fault_only_for_tls_errors() {
    // Garbage bytes fail the TLS handshake inside rustls.
    let prober = prober();
    let target = spawn(Mode::Garbage).await.replace("ws://", "wss://");
    let check = prober.check(&url(&target)).await;
    assert_eq!(check.outcome, Outcome::Offline(Fault::Tls));

    // An immediate close is an IO error, not a TLS failure.
    let target = spawn(Mode::Close).await.replace("ws://", "wss://");
    let check = prober.check(&url(&target)).await;
    assert!(
        matches!(check.outcome, Outcome::Offline(f) if f != Fault::Tls),
        "{:?}",
        check.outcome
    );
}

#[tokio::test]
async fn moved_is_recorded_with_location() {
    let check = prober()
        .check(&url(
            &spawn(Mode::Http(
                "HTTP/1.1 301 Moved Permanently\r\nLocation: wss://new.example/\r\nContent-Length: 0\r\n\r\n",
            ))
            .await,
        ))
        .await;
    assert_eq!(check.outcome, Outcome::Offline(Fault::Moved));
    assert_eq!(check.http_status, Some(301));
    assert_eq!(check.location.as_deref(), Some("wss://new.example/"));
}

#[tokio::test]
async fn waf_statuses_are_blocked() {
    for status in ["401 Unauthorized", "403 Forbidden", "429 Too Many Requests"] {
        let response = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n");
        let relay = spawn(Mode::Http(Box::leak(response.into_boxed_str()))).await;
        let check = prober().check(&url(&relay)).await;
        assert_eq!(
            check.outcome,
            Outcome::Unverified(Obstacle::Blocked),
            "{status}"
        );
        assert!(check.http_status.is_some());
    }
}

#[tokio::test]
async fn plain_http_is_not_websocket() {
    let check = prober()
        .check(&url(&spawn(Mode::Http(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
        ))
        .await))
        .await;
    assert_eq!(check.outcome, Outcome::Offline(Fault::NotWebsocket));
    assert_eq!(check.http_status, Some(200));
}

#[tokio::test]
async fn server_error_is_recorded() {
    let check = prober()
        .check(&url(&spawn(Mode::Http(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n",
        ))
        .await))
        .await;
    assert_eq!(check.outcome, Outcome::Offline(Fault::ServerError));
    assert_eq!(check.http_status, Some(500));
}

#[tokio::test]
async fn refused_connection() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let check = prober()
        .check(&url(&format!("ws://127.0.0.1:{port}/")))
        .await;
    assert_eq!(check.outcome, Outcome::Offline(Fault::Refused));
}

#[tokio::test]
async fn private_address_rejected_under_public_only() {
    let prober = Prober::new(AddressPolicy::PublicOnly).unwrap();
    for target in ["ws://127.0.0.1:8080/", "wss://[::1]:8080/"] {
        let check = prober.check(&url(target)).await;
        assert_eq!(
            check.outcome,
            Outcome::Offline(Fault::PrivateAddress),
            "{target}"
        );
    }
}

#[tokio::test]
async fn nip11_ip_literal_is_policy_checked() {
    // reqwest skips its DNS resolver for IP-literal hosts; the policy is
    // applied before any request is made.
    let prober = Prober::new(AddressPolicy::PublicOnly).unwrap();
    for target in ["wss://127.0.0.1:9/", "wss://[::1]:9/"] {
        let err = prober.nip11(&url(target)).await.unwrap_err();
        assert!(format!("{err:#}").contains("policy"), "{target}: {err:#}");
    }
}

#[tokio::test]
async fn open_times_out_on_dead_server() {
    let check = prober().check(&url(&spawn(Mode::Dead).await)).await;
    assert_eq!(check.outcome, Outcome::Offline(Fault::Timeout));
}

#[tokio::test]
async fn slow_upgrade_still_gets_a_full_read_budget() {
    // The 101 lands 6 s in, inside the open deadline; the relay then stays
    // silent, so the check must end online after the read timeout, not
    // offline against the open budget.
    let check = prober()
        .check(&url(&spawn(Mode::Delayed(
            std::time::Duration::from_secs(6),
            RelayReply::Silent,
        ))
        .await))
        .await;
    assert_eq!(check.outcome, Outcome::Online);
    assert!(check.rtt_open.is_some_and(|rtt| rtt >= 6000));
    assert_eq!(check.rtt_read, None);
}
