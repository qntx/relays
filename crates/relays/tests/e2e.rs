//! End-to-end single-round test.

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

use common::{Mode, RelayReply, spawn};
use relays::probe::Prober;
use relays::probe::resolve::AddressPolicy;
use relays::publish::Publisher;
use relays::schedule;
use relays::store::Store;
use relays_core::curation::{Collection, Curation, Localized};

#[tokio::test]
async fn one_cycle_publishes_all_artefacts() {
    let url =
        relays_core::url::RelayUrl::parse(&spawn(Mode::WebSocket(RelayReply::Eose)).await).unwrap();
    let curation = Curation {
        collections: vec![Collection {
            id: "featured".to_owned(),
            name: Localized::new(BTreeMap::from([("en".to_owned(), "Featured".to_owned())])),
            description: None,
            relays: vec![url.clone()],
        }],
        relays: BTreeMap::from([(
            url.clone(),
            relays_core::curation::RelayOverride {
                name: Some("Mock Relay".to_owned()),
                ..Default::default()
            },
        )]),
    };

    let store = Store::open_memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = format!("file://{}", dir.path().display());
    let mut publisher = Publisher::new(&root).unwrap();

    let prober = Prober::new(AddressPolicy::AllowLoopback).unwrap();
    let status = schedule::run_once(&curation, &store, &prober, &mut publisher)
        .await
        .unwrap();

    for name in [
        "catalog.json",
        "index.json",
        "schema.json",
        "status.json",
        "report.json",
    ] {
        assert!(dir.path().join("v1").join(name).exists(), "{name} missing");
    }

    let catalog: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("v1/catalog.json")).unwrap())
            .unwrap();
    assert_eq!(catalog["version"], 1);
    let relays = catalog["relays"].as_array().unwrap();
    assert_eq!(relays.len(), 1);
    assert_eq!(relays[0]["url"], url.as_str());
    assert_eq!(relays[0]["status"], "online");
    assert_eq!(relays[0]["name"], "Mock Relay");
    // The single online check spans the few seconds to the round's
    // post-check timestamp, so uptime is 1 and the score 100.
    assert_eq!(relays[0]["uptime"], 1.0);
    assert_eq!(relays[0]["score"], 100);
    assert_eq!(catalog["collections"][0]["id"], "featured");
    assert!(jsonschema::is_valid(
        &serde_json::from_slice(&std::fs::read(dir.path().join("v1/schema.json")).unwrap())
            .unwrap(),
        &catalog
    ));

    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("v1/report.json")).unwrap()).unwrap();
    assert_eq!(report["relays"].as_array().unwrap().len(), 0);

    assert_eq!(status.relays, 1);
    assert_eq!(status.online, 1);
    assert_eq!(status.offline, 0);
    assert_eq!(status.unverified, 0);
}
