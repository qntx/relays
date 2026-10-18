//! IO-free domain model for the relay directory: curation parsing, URL
//! canonicalisation, observation types, NIP-11 extraction, state derivation
//! and `v1` artefact projection. All timestamps are unix seconds (`u64`).

#![cfg_attr(
    test,
    expect(
        unused_crate_dependencies,
        reason = "dev-dependencies are used by the tests/ targets"
    )
)]

pub mod curation;
pub mod derive;
pub mod directory;
pub mod nip11;
pub mod observe;
pub mod report;
pub mod url;
