<!-- markdownlint-disable MD033 MD041 -->

# relays

Nostr relay directory: curated relays, probed and published as JSON for clients such as Meowl.

- `relays-core` — IO-free curation, derivation and artefact model
- `relays` — the monitor binary: fixed-upstream DNS, WebSocket/NIP-11 probing, SQLite, scheduled rounds and artefact publication

## Quick start

```bash
cargo run -p relays -- validate
cargo run -p relays -- run --once --store file:///tmp/relays-out
```

`relays --help` lists every flag with its `RELAYS_*` environment variable. Artefacts land under `v1/` (`catalog.json`, `index.json`, `schema.json`, `status.json`, `report.json`).

Changes are tracked in [CHANGELOG.md](CHANGELOG.md); contributor invariants and the local gate live in [AGENTS.md](AGENTS.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project shall be dual-licensed as above, without any additional terms or conditions.

---

<div align="center">

A **[QuantX](https://qntx.org)** open-source project.

<a href="https://qntx.org"><img alt="QuantX" width="369" src="https://raw.githubusercontent.com/qntx/.github/main/profile/qntx.svg" /></a>

Code is law. We write both.

</div>
