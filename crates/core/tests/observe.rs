//! Observation type wire-format tests: reason codes and JSON shape.

#![expect(
    unused_crate_dependencies,
    reason = "test targets share the crate dependency set"
)]
#![allow(
    clippy::panic,
    clippy::tests_outside_test_module,
    clippy::unwrap_used,
    reason = "test binary: panic-based assertions and free #[test] functions are idiomatic"
)]

use std::str::FromStr;

use relays_core::observe::{Check, Fault, Obstacle, Outcome};

#[test]
fn fault_codes_round_trip() {
    let codes = [
        (Fault::DnsNxdomain, "dns-nxdomain"),
        (Fault::PrivateAddress, "private-address"),
        (Fault::Refused, "refused"),
        (Fault::Timeout, "timeout"),
        (Fault::Tls, "tls"),
        (Fault::Moved, "moved"),
        (Fault::NotWebsocket, "not-websocket"),
        (Fault::ServerError, "server-error"),
        (Fault::Protocol, "protocol"),
        (Fault::Network, "network"),
    ];
    for (fault, code) in codes {
        assert_eq!(serde_json::to_value(fault).unwrap(), code);
        assert_eq!(Fault::from_str(code), Ok(fault));
        assert_eq!(fault.to_string(), code);
        assert_eq!(Outcome::Offline(fault).reason(), Some(code));
    }
    assert!(Fault::from_str("bogus").is_err());
}

#[test]
fn obstacle_codes_round_trip() {
    let codes = [
        (Obstacle::Blocked, "blocked"),
        (Obstacle::DnsTransient, "dns-transient"),
    ];
    for (obstacle, code) in codes {
        assert_eq!(serde_json::to_value(obstacle).unwrap(), code);
        assert_eq!(Obstacle::from_str(code), Ok(obstacle));
        assert_eq!(obstacle.to_string(), code);
        assert_eq!(Outcome::Unverified(obstacle).reason(), Some(code));
    }
}

#[test]
fn check_json_shape() {
    let online = Check {
        at: 42,
        outcome: Outcome::Online,
        http_status: None,
        location: None,
        rtt_open: Some(100),
        rtt_read: Some(50),
        auth: false,
    };
    assert_eq!(
        serde_json::to_value(&online).unwrap(),
        serde_json::json!({
            "at": 42,
            "outcome": "online",
            "rtt_open": 100,
            "rtt_read": 50,
            "auth": false,
        })
    );

    let offline = Check {
        outcome: Outcome::Offline(Fault::Moved),
        http_status: Some(301),
        location: Some("wss://new.example/".to_owned()),
        ..online
    };
    assert_eq!(
        serde_json::to_value(&offline).unwrap(),
        serde_json::json!({
            "at": 42,
            "outcome": "offline",
            "reason": "moved",
            "http_status": 301,
            "location": "wss://new.example/",
            "rtt_open": 100,
            "rtt_read": 50,
            "auth": false,
        })
    );

    let unverified = Check {
        outcome: Outcome::Unverified(Obstacle::Blocked),
        http_status: Some(403),
        location: None,
        rtt_open: None,
        rtt_read: None,
        ..online
    };
    assert_eq!(
        serde_json::to_value(&unverified).unwrap(),
        serde_json::json!({
            "at": 42,
            "outcome": "unverified",
            "reason": "blocked",
            "http_status": 403,
            "auth": false,
        })
    );
}
