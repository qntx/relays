//! Directory projection tests.

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

mod common;

use std::collections::BTreeMap;

use relays_core::curation::Curation;
use relays_core::directory::{self, Observed};
use relays_core::nip11::RelayInfo;
use relays_core::observe::{Check, Fault, Outcome};
use relays_core::url::RelayUrl;

const NOW: u64 = 8_640_000;

fn online(at: u64) -> Check {
    Check {
        at,
        outcome: Outcome::Online,
        http_status: None,
        location: None,
        rtt_open: Some(120),
        rtt_read: Some(80),
        auth: false,
    }
}

fn build(
    curation: &Curation,
    checks: &[(RelayUrl, Vec<Check>)],
    info: Option<(&RelayUrl, &RelayInfo)>,
) -> serde_json::Value {
    let checks_map: BTreeMap<RelayUrl, Vec<Check>> = checks.iter().cloned().collect();
    let observed: BTreeMap<RelayUrl, Observed<'_>> = curation
        .urls()
        .into_iter()
        .map(|url| {
            (
                url.clone(),
                Observed {
                    checks: checks_map.get(url).map_or(&[], Vec::as_slice),
                    info: info
                        .filter(|(u, _)| u.as_str() == url.as_str())
                        .map(|(_, i)| i),
                },
            )
        })
        .collect();
    serde_json::to_value(directory::build(NOW, curation, &observed)).unwrap()
}

#[test]
fn directory_projection() {
    let curation = Curation::parse(common::BASE).unwrap();
    let a = RelayUrl::parse("wss://a.example/").unwrap();
    let b = RelayUrl::parse("wss://b.example/x").unwrap();
    let checks = vec![online(NOW - 100)];
    let info = RelayInfo {
        name: Some("B Relay".to_owned()),
        description: Some("The B relay.".to_owned()),
        icon: Some("https://b.example/i.png".to_owned()),
        nips: vec![1, 11],
        auth_required: true,
        ..RelayInfo::default()
    };
    let doc = build(&curation, &[(a, checks)], Some((&b, &info)));

    assert_eq!(doc["version"], 1);
    assert_eq!(doc["generated_at"], NOW);
    assert_eq!(doc["collections"].as_array().unwrap().len(), 2);
    assert_eq!(doc["collections"][0]["id"], "featured");
    assert_eq!(doc["collections"][0]["kind"], "curated");
    assert_eq!(doc["collections"][0]["name"]["en"], "Featured");
    assert_eq!(doc["collections"][0]["name"]["zh-Hans"], "精选");
    assert_eq!(
        doc["collections"][1]["description"],
        serde_json::Value::Null
    );

    let relays = doc["relays"].as_array().unwrap();
    assert_eq!(relays.len(), 2);
    // Sorted by URL.
    assert_eq!(relays[0]["url"], "wss://a.example/");
    assert_eq!(relays[0]["name"], "Relay A"); // curation wins
    assert_eq!(relays[0]["status"], "online");
    assert_eq!(relays[0]["description"]["en"], "The A relay.");
    assert_eq!(relays[0]["languages"], serde_json::json!(["en"]));

    assert_eq!(relays[1]["url"], "wss://b.example/x");
    assert_eq!(relays[1]["name"], "B Relay"); // NIP-11 fallback
    assert_eq!(relays[1]["status"], "unverified"); // never checked
    assert_eq!(relays[1]["description"]["und"], "The B relay.");
    assert_eq!(relays[1]["icon"], "https://b.example/i.png");
    assert_eq!(relays[1]["nips"], serde_json::json!([1, 11]));
    assert_eq!(relays[1]["requirements"]["auth"], true);
    assert_eq!(relays[1]["score"], 0);
    // Absent optional fields are omitted.
    assert!(relays[1].get("uptime").is_none());
    assert!(relays[1].get("rtt").is_none());
    assert!(relays[1].get("icon").is_some());
    assert!(relays[0].get("icon").is_none());
}

#[test]
fn directory_snapshot() {
    let curation = Curation::parse(common::BASE).unwrap();
    let a = RelayUrl::parse("wss://a.example/").unwrap();
    let b = RelayUrl::parse("wss://b.example/x").unwrap();
    let checks_a = vec![online(NOW - 100)];
    let mut offline = online(NOW - 50);
    offline.outcome = Outcome::Offline(Fault::Moved);
    offline.http_status = Some(301);
    offline.location = Some("wss://new.example/".to_owned());
    let doc = build(&curation, &[(a, checks_a), (b, vec![offline])], None);
    insta::assert_json_snapshot!(doc, {
        ".generated_at" => "<ts>",
    });
}

#[test]
fn catalog_validates_against_schema() {
    let curation = Curation::parse(common::BASE).unwrap();
    let a = RelayUrl::parse("wss://a.example/").unwrap();
    let doc = build(&curation, &[(a, vec![online(NOW - 10)])], None);
    let schema = serde_json::to_value(directory::schema()).unwrap();
    assert!(jsonschema::is_valid(&schema, &doc), "{doc}");
}

#[test]
fn status_counts() {
    let curation = Curation::parse(common::BASE).unwrap();
    let doc = build(&curation, &[], None);
    let relays = doc["relays"].as_array().unwrap();
    assert!(relays.iter().all(|r| r["status"] == "unverified"));
}
