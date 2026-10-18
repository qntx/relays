# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - Unreleased

**Breaking**: the repository is rebuilt as a self-contained monitoring
directory. The old catalog pipeline, health-state model, GitHub
Pages site and `data` branch publication are removed without a compatibility
path.

### Added

- **New curation format** in `relays.toml`: `[[collection]]` entries with
  localized names/descriptions and `[[relay]]` editorial overrides (name,
  description, languages, nsfw). Every URL must be canonical `wss://`.
- **`relays-core`**: IO-free domain crate — canonical URLs,
  curation validation, observation types, NIP-11 lenient extraction,
  deterministic derivation (time-weighted uptime, RTT medians, requirements,
  score), and `v1` artefact models with `directory::schema()`.
- **`relays` binary**: fixed-upstream DNS resolution (1.1.1.1 and
  9.9.9.9 over UDP+TCP, never `resolv.conf`), public-address policy, TCP/TLS/
  WebSocket probing with reason-coded outcomes, NIP-11 fetching, SQLite
  storage, exponential backoff scheduling, and `object_store` publication
  (`file://` and `s3://`) with content-hash skips.
- **`v1/` artefacts**: `catalog.json`, `index.json`, `schema.json`,
  `status.json`, `report.json`, published with JSON content type and shared
  cache headers.
- **CLI**: `validate`, `probe <url>`, `run [--once]`,
  `report <source>`; flags mirror the `RELAYS_*` environment variables.
- **Container image**: multi-stage Dockerfile on
  `gcr.io/distroless/cc-debian12:nonroot`, published to `ghcr.io/qntx/relays`
  on every `v*` tag (or manually via `workflow_dispatch`).
- **`report.yml`** workflow: weekly `relays report` run that opens or updates
  the single "Relay report" issue.
- **Licensing**: the repository is dual-licensed `MIT OR Apache-2.0`
  (`LICENSE-MIT` + `LICENSE-APACHE`), matching `Cargo.toml`.
- **TOML formatting**: `.taplo.toml` + a `taplo` CI job keep every TOML file
  consistently formatted (aligned `=`; one URL per line in `relays.toml`).

### Removed

- The generated relay list in `README.md` and the `relays readme`
  subcommand; the README is now prose only.
- `crates/cli`, `crates/probe` and the previous `crates/core` sources; the
  workspace is now `crates/core` + `crates/relays` only.
- The health state machine, `relays.json`/`health.json`/`api/` artefacts,
  `schema_version`, `nip66.json`, `badge.json` and the `dead` subcommand.
- The `web/` site, `CNAME`, `notes/`, and the `build`/`publish`/`pages`/
  `health-check`/`dead-relays`/`ci-docs` workflows.
- `ws://` and `.onion` entries plus the `regional-*`, `language` and `tor`
  collections; `operator`, `country`, `network`, `geohash`, `software`,
  `requirements`, `topics` and `added_at` fields.

[0.3.0]: https://github.com/qntx/relays/compare/v0.2.0...HEAD

## [0.2.0] - 2026-06-06

A ground-up refactor toward an ecosystem-native, production-ready relay catalog.
**Breaking**: the catalog and public JSON API are now `schema_version: 2`.

### Added

- **NIP-66 alignment** for the data model, plus a new `nip66.json` endpoint that
  emits unsigned `kind 30166` relay-discovery event templates.
- **Explicit health state machine** (`healthy`, `flaky`, `warn`, `dead`,
  `stale`, `blocked`, `skipped`, `unknown`) surfaced as `health.state` in
  `relays.json`, replacing consumer-side inference.
- **`monitor`** (probe cadence/checks, NIP-66 `kind 10166`) and **`summary`**
  (per-state relay counts) blocks in `relays.json`.
- **`badge.json`** shields.io endpoint and a live health badge.
- **JSON Schema** export under `schema/` (`relay`, `collection`, `health-entry`,
  `health-report`), generated from the Rust types via `schemars`.
- **`relays dead`** subcommand and a weekly **Dead Relay Report** workflow that
  opens/updates a removal-candidate tracking issue.
- `network` (clearnet/tor/i2p/loki), `geohash`, `requirements` and `topics`
  fields on catalog entries.

### Changed

- **Workspace split** into `relays-core` (IO-free domain/render), `relays-probe`
  (async probing) and `relays` (CLI).
- **Data layout**: `health.json` and `api/` are no longer committed to `main`.
  CI publishes them to the orphan `data` branch; GitHub Pages serves `web/`
  (main) + `api/` (data). This removes the high-frequency machine commits that
  previously dominated `main` history.
- The README catalog block no longer embeds per-relay health (now a badge + API
  link), so it only changes when the curated catalog changes.
- Persisted timestamps are truncated to whole seconds for readable diffs.

### Fixed

- **Pseudo-healthy relays**: a WAF/DNS-blocked relay that once succeeded no
  longer reports healthy — it is now `blocked`. A staleness guard reports
  `stale` when monitoring data is older than `STALE_AFTER_HOURS`.
- Probed-but-below-threshold relays are now `flaky`, distinct from the
  never-probed `unknown` state.

### Removed

- `country = "T1"` Tor hack (replaced by the `network` field).
- `*.json merge=union` git attribute (an anti-pattern for structured files).
- Top-level `paid` / `tags` / `status` JSON fields (replaced by
  `requirements` / `topics` / `health`).
