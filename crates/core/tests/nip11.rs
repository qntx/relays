//! NIP-11 lenient extraction tests.

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

use relays_core::nip11::RelayInfo;
use serde_json::json;

#[test]
fn extracts_all_fields() {
    let doc = json!({
        "name": "My Relay",
        "description": "A fine relay.",
        "icon": "https://a.example/i.png",
        "supported_nips": [1, 2, 11, 42],
        "limitation": {
            "auth_required": true,
            "payment_required": true,
            "restricted_writes": true,
            "min_pow_difficulty": 8
        }
    });
    let info = RelayInfo::extract(&doc);
    assert_eq!(info.name.as_deref(), Some("My Relay"));
    assert_eq!(info.description.as_deref(), Some("A fine relay."));
    assert_eq!(info.icon.as_deref(), Some("https://a.example/i.png"));
    assert_eq!(info.nips, [1, 2, 11, 42]);
    assert!(info.auth_required);
    assert!(info.payment_required);
    assert!(info.restricted_writes);
    assert!(info.pow);
}

#[test]
fn bad_field_types_are_dropped() {
    let doc = json!({
        "name": 42,
        "description": {"nested": true},
        "icon": ["x"],
        "supported_nips": "11",
        "limitation": "yes",
        "pubkey": "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqjxdcs",
    });
    let info = RelayInfo::extract(&doc);
    assert_eq!(info, RelayInfo::default());
}

#[test]
fn npub_pubkey_is_ignored() {
    let doc = json!({
        "pubkey": "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqjxdcs",
        "name": "Relay",
    });
    let info = RelayInfo::extract(&doc);
    assert_eq!(info.name.as_deref(), Some("Relay"));
}

#[test]
fn long_text_is_sanitized() {
    let name = "  a\u{0} very   long\nname\t with padding  ";
    let doc = json!({"name": name});
    let info = RelayInfo::extract(&doc);
    assert_eq!(info.name.as_deref(), Some("a very long name with padding"));

    let long_name = "n".repeat(60);
    let doc = json!({"name": long_name});
    let info = RelayInfo::extract(&doc);
    assert_eq!(
        info.name.as_deref().map(str::chars).map(Iterator::count),
        Some(40)
    );
    assert!(info.name.as_deref().unwrap().ends_with('…'));

    let long_desc = "d".repeat(300);
    let doc = json!({"description": long_desc});
    let info = RelayInfo::extract(&doc);
    assert_eq!(
        info.description.as_deref().map(|s| s.chars().count()),
        Some(280)
    );
}

#[test]
fn empty_sanitized_fields_are_missing() {
    let doc = json!({"name": "   \u{7}\u{8}  "});
    let info = RelayInfo::extract(&doc);
    assert_eq!(info.name, None);
}

#[test]
fn non_https_icons_are_dropped() {
    for icon in [
        "http://a.example/i.png",
        "wss://a.example/",
        "not a url",
        "data:image/png,x",
    ] {
        let doc = json!({"icon": icon});
        assert_eq!(RelayInfo::extract(&doc).icon, None, "{icon}");
    }
    let doc = json!({"icon": "https://a.example/i.png"});
    assert_eq!(
        RelayInfo::extract(&doc).icon.as_deref(),
        Some("https://a.example/i.png")
    );
}

#[test]
fn nips_are_bounded_sorted_deduped() {
    let doc = json!({"supported_nips": [42, 1, 42, 65535, 65536, -1, 1.5, "11", null]});
    let info = RelayInfo::extract(&doc);
    assert_eq!(info.nips, [1, 42, 65535]);
}

#[test]
fn non_object_document_yields_defaults() {
    for doc in [json!(null), json!([]), json!("x"), json!(5)] {
        assert_eq!(RelayInfo::extract(&doc), RelayInfo::default());
    }
}

#[test]
fn zero_pow_is_not_pow() {
    let doc = json!({"limitation": {"min_pow_difficulty": 0}});
    assert!(!RelayInfo::extract(&doc).pow);
    let doc = json!({"limitation": {"min_pow_difficulty": "high"}});
    assert!(!RelayInfo::extract(&doc).pow);
}
