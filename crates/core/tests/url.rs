//! Canonical URL tests, including the shared normalize vectors.

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

use relays_core::url::RelayUrl;

#[derive(serde::Deserialize)]
struct Vectors {
    cases: Vec<Case>,
}

#[derive(serde::Deserialize)]
struct Case {
    input: String,
    output: Option<String>,
    error: Option<String>,
}

#[test]
fn url_normalize_vectors() {
    let raw = include_str!("vectors/url-normalize.json");
    let vectors: Vectors = serde_json::from_str(raw).expect("valid vector file");
    for case in vectors.cases {
        match (case.output, case.error) {
            (Some(output), None) => {
                let url = RelayUrl::parse(&case.input)
                    .unwrap_or_else(|_| panic!("input {} should parse", case.input));
                assert_eq!(url.as_str(), output, "input {}", case.input);
            }
            (None, Some(_)) => {
                assert!(
                    RelayUrl::parse(&case.input).is_err(),
                    "input {} should fail",
                    case.input
                );
            }
            _ => panic!("case for {} must have output xor error", case.input),
        }
    }
}

#[test]
fn ftp_is_an_error() {
    assert!(RelayUrl::parse("ftp://a.example").is_err());
    assert!(RelayUrl::parse("ftp://x").is_err());
}

#[test]
fn bare_host_defaults_to_wss() {
    let url = RelayUrl::parse("example.com").unwrap();
    assert_eq!(url.as_str(), "wss://example.com/");
    assert!(url.is_secure());
    assert_eq!(url.port(), 443);
}

#[test]
fn accessors() {
    let url = RelayUrl::parse("wss://relay.example:8443/a/b?x=1").unwrap();
    assert_eq!(url.host(), "relay.example");
    assert_eq!(url.port(), 8443);
    assert!(url.is_secure());
    assert_eq!(url.to_string(), "wss://relay.example:8443/a/b?x=1");
}

#[test]
fn nip11_url_swaps_scheme() {
    let secure = RelayUrl::parse("wss://a.example/x").unwrap();
    assert_eq!(secure.nip11_url().as_str(), "https://a.example/x");
    let plain = RelayUrl::parse("ws://a.example:8080/x").unwrap();
    assert_eq!(plain.nip11_url().as_str(), "http://a.example:8080/x");
}

#[test]
fn ordering_is_lexicographic() {
    let a = RelayUrl::parse("wss://a.example/").unwrap();
    let b = RelayUrl::parse("wss://b.example/").unwrap();
    assert!(a < b);
}

#[test]
fn ip_returns_literals_including_ipv6() {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    let v4 = RelayUrl::parse("wss://127.0.0.1/").unwrap();
    assert_eq!(v4.ip(), Some(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    assert_eq!(v4.host(), "127.0.0.1");

    let v6 = RelayUrl::parse("wss://[::1]/").unwrap();
    assert_eq!(v6.ip(), Some(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    assert_eq!(v6.as_str(), "wss://[::1]/");

    let name = RelayUrl::parse("wss://nos.lol/").unwrap();
    assert_eq!(name.ip(), None);
}
