# relays — Nostr relay directory monitor

default:
    @just --list

# Format all code (Rust + TOML).
fmt:
    cargo fmt --all
    npx --yes @taplo/cli@0.7.0 fmt

# Run the full local gate (same checks as CI).
lint:
    npx --yes @taplo/cli@0.7.0 fmt --check
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features
    cargo deny check
    cargo run -q -p relays -- validate

# Strict clippy on the workspace.
clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run the test suite.
test:
    cargo test --workspace --all-features

# Probe every curated relay once and publish to a local file store.
run-once:
    cargo run -p relays -- run --once --store file:///tmp/relays-out
