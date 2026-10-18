//! Curation file validation tests.

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

use relays_core::curation::Curation;

fn errors(source: &str) -> Vec<String> {
    match Curation::parse(source) {
        Ok(_) => panic!("expected validation errors"),
        Err(e) => e.0,
    }
}

#[test]
fn valid_file_parses() {
    let curation = Curation::parse(common::BASE).expect("valid curation");
    assert_eq!(curation.collections.len(), 2);
    assert_eq!(curation.collections[0].id, "featured");
    assert_eq!(curation.collections[0].relays.len(), 2);
    assert_eq!(curation.relays.len(), 1);
    assert_eq!(curation.urls().len(), 2);
    let entry = curation.relays.values().next().unwrap();
    assert_eq!(entry.name.as_deref(), Some("Relay A"));
    assert_eq!(entry.languages, ["en"]);
}

#[test]
fn syntax_error_aborts() {
    let errs = errors("not = [toml");
    assert_eq!(errs.len(), 1);
}

#[test]
fn unknown_top_level_key() {
    let errs = errors("schema_version = 2\n");
    assert_eq!(errs.len(), 1);
    assert!(
        errs[0].contains("unknown field `schema_version`"),
        "{errs:?}"
    );
}

#[test]
fn unknown_fields_fail() {
    let errs = errors(&format!(
        "{}\n\n[[collection]]\nid = \"extra\"\nname = {{ en = \"X\" }}\nrelays = [\"wss://c.example/\"]\ncountry = \"US\"\n",
        common::BASE
    ));
    assert!(errs.iter().any(|e| e.contains("unknown field `country`")));
    let errs = errors(&format!(
        "{}\n\n[[relay]]\nurl = \"wss://b.example/x\"\noperator = \"x\"\n",
        common::BASE
    ));
    assert!(errs.iter().any(|e| e.contains("unknown field `operator`")));
}

#[test]
fn collection_requires_id_name_relays() {
    // Missing required fields are structural errors: the first error stops.
    let errs = errors("[[collection]]\nname = { en = \"X\" }\n");
    assert_eq!(errs.len(), 1);
    assert!(errs[0].contains("missing field `id`"), "{errs:?}");
    let errs = errors("[[collection]]\nid = \"x\"\nrelays = [\"wss://a.example/\"]\n");
    assert!(errs.iter().any(|e| e.contains("missing field `name`")));
}

#[test]
fn name_must_contain_en() {
    let errs = errors(
        "[[collection]]\nid = \"x\"\nname = { zh-Hans = \"精选\" }\nrelays = [\"wss://a.example/\"]\n",
    );
    assert!(errs.iter().any(|e| e.contains("must contain `en`")));
}

#[test]
fn collection_relays_must_be_non_empty() {
    let errs = errors("[[collection]]\nid = \"x\"\nname = { en = \"X\" }\nrelays = []\n");
    assert!(errs.iter().any(|e| e.contains("must not be empty")));
}

#[test]
fn collection_urls_must_be_canonical_wss() {
    for (url, needle) in [
        ("ws://a.example/", "not a `wss://` URL"),
        ("wss://a.example", "not canonical"),
        ("wss://A.example/", "not canonical"),
        ("not a url", "not a valid relay URL"),
        ("ftp://a.example/", "not a valid relay URL"),
        ("wss://user:pw@a.example/", "must not contain userinfo"),
    ] {
        let errs = errors(&format!(
            "[[collection]]\nid = \"x\"\nname = {{ en = \"X\" }}\nrelays = [\"{url}\"]\n"
        ));
        assert!(errs.iter().any(|e| e.contains(needle)), "{url}: {errs:?}");
    }
}

#[test]
fn duplicate_collection_id_fails() {
    let errs = errors(&format!(
        "{}\n\n[[collection]]\nid = \"featured\"\nname = {{ en = \"X\" }}\nrelays = [\"wss://c.example/\"]\n",
        common::BASE
    ));
    assert!(errs.iter().any(|e| e.contains("defined twice")));
}

#[test]
fn duplicate_url_within_collection_fails() {
    let errs = errors(
        "[[collection]]\nid = \"x\"\nname = { en = \"X\" }\nrelays = [\"wss://a.example/\", \"wss://a.example/\"]\n",
    );
    assert!(errs.iter().any(|e| e.contains("listed twice")));
}

#[test]
fn duplicate_relay_entry_fails() {
    let errs = errors(&format!(
        "{}\n\n[[relay]]\nurl = \"wss://a.example/\"\nname = \"A\"\n\n[[relay]]\nurl = \"wss://a.example/\"\nname = \"B\"\n",
        common::BASE
    ));
    assert!(errs.iter().any(|e| e.contains("defined twice")));
}

#[test]
fn unreferenced_relay_entry_fails() {
    let errs = errors(&format!(
        "{}\n\n[[relay]]\nurl = \"wss://c.example/\"\nname = \"C\"\n",
        common::BASE
    ));
    assert!(
        errs.iter()
            .any(|e| e.contains("not referenced by any collection"))
    );
}

#[test]
fn relay_name_length_bounds() {
    let long = "x".repeat(41);
    let errs = errors(&format!(
        "{}\n\n[[relay]]\nurl = \"wss://b.example/x\"\nname = \"{long}\"\n",
        common::BASE
    ));
    assert!(
        errs.iter()
            .any(|e| e.contains("must be 1 to 40 characters"))
    );
    let errs = errors(&format!(
        "{}\n\n[[relay]]\nurl = \"wss://b.example/x\"\nname = \"\"\n",
        common::BASE
    ));
    assert!(
        errs.iter()
            .any(|e| e.contains("must be 1 to 40 characters"))
    );
}

#[test]
fn languages_must_be_iso639_1() {
    for (code, valid) in [("ja", true), ("jpn", false), ("JA", false), ("j1", false)] {
        let source = format!(
            "{}\n\n[[relay]]\nurl = \"wss://b.example/x\"\nlanguages = [\"{code}\"]\n",
            common::BASE
        );
        match (Curation::parse(&source), valid) {
            (Ok(_), true) => {}
            (Err(e), false) => {
                assert!(
                    e.0.iter().any(|m| m.contains("two-letter code")),
                    "{code}: {e}"
                );
            }
            (Ok(_), false) => panic!("{code} should be rejected"),
            (Err(e), true) => panic!("{code} should be accepted: {e}"),
        }
    }
}

#[test]
fn duplicate_language_fails() {
    let errs = errors(&format!(
        "{}\n\n[[relay]]\nurl = \"wss://b.example/x\"\nlanguages = [\"ja\", \"ja\"]\n",
        common::BASE
    ));
    assert!(errs.iter().any(|e| e.contains("listed twice")));
}

#[test]
fn und_tag_is_reserved() {
    let errs = errors(&format!(
        "{}\n\n[[relay]]\nurl = \"wss://b.example/x\"\ndescription = {{ und = \"x\", en = \"y\" }}\n",
        common::BASE
    ));
    assert!(errs.iter().any(|e| e.contains("reserved tag `und`")));
}

#[test]
fn id_rules() {
    for (id, valid) in [
        ("featured", true),
        ("regional-americas", true),
        ("Featured", false),
        ("regional_americas", false),
        ("-bad", false),
        ("bad-", false),
        ("dm-inbox", true),
        ("lang-x", false),
        ("lang-zh", false),
        ("role-search", false),
    ] {
        let source = format!(
            "[[collection]]\nid = \"{id}\"\nname = {{ en = \"X\" }}\nrelays = [\"wss://a.example/\"]\n"
        );
        match (Curation::parse(&source), valid) {
            (Ok(_), true) | (Err(_), false) => {}
            (Ok(_), false) => panic!("{id} should be rejected"),
            (Err(e), true) => panic!("{id} should be accepted: {e}"),
        }
    }
}

#[test]
fn errors_are_aggregated() {
    // Structural errors stop at the first one; semantic errors all report.
    let errs = errors(
        "schema_version = 2\n\n[[collection]]\nid = \"Bad\"\nname = { und = \"x\" }\nrelays = []\n",
    );
    assert_eq!(errs.len(), 1);
    let errs = errors("[[collection]]\nid = \"Bad\"\nname = { und = \"x\" }\nrelays = []\n");
    assert!(errs.len() >= 3, "{errs:?}");
}
