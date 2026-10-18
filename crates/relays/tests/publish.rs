//! Publisher tests.

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

use relays::publish::Publisher;

#[tokio::test]
async fn file_store_writes_and_skips_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let root = format!("file://{}", dir.path().display());
    let mut publisher = Publisher::new(&root).unwrap();

    assert!(publisher.put("v1/a.json", b"{}".to_vec()).await.unwrap());
    assert!(dir.path().join("v1/a.json").exists());
    // Identical content is not rewritten.
    assert!(!publisher.put("v1/a.json", b"{}".to_vec()).await.unwrap());
    // Changed content is rewritten.
    assert!(
        publisher
            .put("v1/a.json", b"{\"x\":1}".to_vec())
            .await
            .unwrap()
    );
    assert_eq!(
        std::fs::read(dir.path().join("v1/a.json")).unwrap(),
        b"{\"x\":1}".to_vec()
    );
    // New keys always upload.
    assert!(publisher.put("v1/b.json", b"{}".to_vec()).await.unwrap());
}

#[tokio::test]
async fn unknown_scheme_fails() {
    assert!(Publisher::new("ftp://x/").is_err());
}
