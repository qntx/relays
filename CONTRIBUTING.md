# Contributing

Thanks for taking the time to improve relays.

## Prerequisites

- **Rust stable** via `rust-toolchain.toml`.
- **cargo-deny** for the supply-chain check (`cargo install cargo-deny`).
- **Node.js** — `npx` runs taplo; no other JS tooling is required.
- **Optional**: `just` (recipe shortcuts in `Justfile`), Docker (image build).

## Repository layout

```text
crates/core/        relays-core: IO-free curation, derivation and artefact model
crates/relays/      the `relays` binary: probe, SQLite, scheduling, publication, CLI
relays.toml         curation file — the single source of truth
Dockerfile          multi-stage image (distroless runtime)
.github/            CI, container image and weekly report workflows
```

## Architecture invariants

These are the rules reviewers enforce:

- `relays-core` performs **no I/O**: no async, no network, no filesystem —
  just parsing, validation, derivation and artefact projection.
- Each check opens **one probe connection** (TCP → optional TLS → WebSocket
  upgrade → one `REQ` → close). The outcome is one of:

  | outcome      | reason            | when                                                   |
  | ------------ | ----------------- | ------------------------------------------------------ |
  | `online`     | —                 | the upgrade returned 101                               |
  | `offline`    | `dns-nxdomain`    | NXDOMAIN or no A/AAAA records                          |
  | `offline`    | `private-address` | a resolved or literal address fails the address policy |
  | `offline`    | `refused`         | TCP refused, reset or aborted                          |
  | `offline`    | `timeout`         | the open deadline or a connect attempt elapsed         |
  | `offline`    | `tls`             | a `rustls::Error` during the TLS handshake             |
  | `offline`    | `moved`           | a 3xx upgrade response (`location` is recorded)        |
  | `offline`    | `not-websocket`   | a 2xx non-upgrade, or a 4xx other than 401/403/429     |
  | `offline`    | `server-error`    | a 5xx upgrade response                                 |
  | `offline`    | `protocol`        | a WebSocket handshake violation                        |
  | `offline`    | `network`         | any other IO error                                     |
  | `unverified` | `blocked`         | a 401/403/429 response, usually a WAF                  |
  | `unverified` | `dns-transient`   | SERVFAIL, resolver timeout or DNS transport failure    |

- DNS goes only to the fixed upstreams **1.1.1.1 and 9.9.9.9** over UDP+TCP;
  the host `resolv.conf` is never read.
- The **address policy** allows only globally routable unicast addresses.
  It is checked on every resolved IP before connecting and on IP-literal
  hosts before a NIP-11 request (reqwest skips its DNS resolver for
  literals, including bracketed IPv6). NIP-11 requests never follow
  redirects.
- Budgets (`crates/relays/src/probe/mod.rs`): DNS 5 s, open deadline 15 s
  (DNS → 101 response, with 5 s per TCP connect attempt), read check 10 s,
  close 1 s, NIP-11 10 s and 64 KiB body, WebSocket frames/messages
  256 KiB.
- One `Prober` shares a single `rustls::ClientConfig` (webpki roots) across
  WebSocket checks and the reqwest client; exactly one crypto provider
  (`ring`) may appear in the dependency graph.
- Relay-supplied text is sanitised (control characters stripped, whitespace
  collapsed) and capped: names 40, descriptions 280 characters.
- Scheduling: relays on the same host are checked serially, at most 32
  hosts in parallel. A relay is due 55 minutes after an online or
  unverified check; after `n` consecutive offline checks it backs off
  `2^(n-1)` hours, capped at 24 hours. NIP-11 is fetched at most every
  6 hours, independently of the WebSocket outcome. Raw checks are kept
  35 days.
- Artefacts are deterministic compact JSON; uploads whose SHA-256 matches
  the previous write are skipped. Only additive fields may appear in `v1`.
- No `unwrap`/`expect`/panics in non-test code; the workspace lint set runs
  under `-D warnings`.

## Public contract

The `v1/` artefacts — `catalog.json`, `index.json`, `schema.json`,
`status.json`, `report.json` — are consumed by clients (Meowl). Within `v1`
only **additive** changes are allowed: new optional fields and new
collection kinds are fine because clients ignore unknown fields and kinds;
anything else ships under a new path version. Derived fields mean:

- `status` — the latest check's outcome (`unverified` when never checked).
- `uptime` — 30-day time-weighted ratio of online time; each check's state
  lasts until the next check, and unverified periods count toward neither
  side.
- `rtt` — 24-hour medians of `open` (DNS → 101) and `read` (`REQ` → first
  `EVENT`/`EOSE`), in milliseconds.
- `requirements.auth` — NIP-11 `auth_required` or an `AUTH` /
  `auth-required:` observed in the last 24 hours; the other flags come from
  NIP-11 `limitation`.
- `description` — the curated language table, else the NIP-11 description
  under the `und` (undetermined) key.
- `score` — `round(100 × uptime)`, `0` without observations.

Three test groups guard the
shape: the directory insta snapshot (`crates/core/tests/snapshots/`), the
catalog-against-schema validation test, and the reason-code/JSON tests in
`crates/core/tests/observe.rs`.

## Curating relays

`relays.toml` is the single source of truth:

- `[[collection]]` — `id` (kebab-case, unique; the prefixes `role-` and
  `lang-` are reserved), `name` (language table, must contain `en`),
  optional `description` (language table, `en` when present), and `relays`
  — an ordered member list in editorial (display) order.
- `[[relay]]` — per-relay editorial overrides, only when a relay needs
  one: `url`, plus optional `name` (≤ 40 characters), `description`,
  `languages` (lowercase ISO 639-1) and `nsfw`. Every `[[relay]]` URL must
  be referenced by at least one collection.

Every URL must be canonical `wss://` — lowercase host, no default port, no
fragment, no userinfo. `relays validate` prints the canonical form. It
checks in two stages: structural TOML/type errors are reported with line
and column and stop at the first error; once the document parses, all
semantic violations are reported at once.

Acceptance criteria for new relays:

- `wss://` only; `ws://` and `.onion` URLs are not curated.
- The relay answers a WebSocket upgrade and a NIP-01 `REQ` from a
  datacenter (`relays probe <url>` shows the outcome and NIP-11 data).
- Permanent homes only; no shared-hosting or ephemeral tunnels.

Relay health is **not** edited by hand: the monitor publishes
`v1/report.json` and the weekly `report.yml` workflow renders it into the
single open "Relay report" issue.

## Local gate

Run before opening a pull request:

```bash
npx --yes @taplo/cli@0.7.0 fmt --check
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny check
cargo run -q -p relays -- validate
```

`just lint` runs the same sequence. CI runs taplo, fmt, clippy, tests and
cargo-deny, plus `relays validate` on `relays.toml`.

## Running locally

```bash
cargo run -p relays -- run --once --store file:///tmp/relays-out
cargo run -p relays -- probe wss://nos.lol/
```

Every flag has a `RELAYS_*` environment variable; `relays --help` lists
them. A full round takes about 20 s for ~100 relays.

Docker:

```bash
docker build -t relays . &&
docker run --rm -v /tmp/relays-data:/data -e RELAYS_STORE=file:///data/out relays run --once
```

The monitor resolves through fixed public upstreams, so fake-IP local
resolvers (for example OrbStack's `198.18.0.0/15`) cannot distort
classification. Networks that block direct connections to foreign IPs will
still show inflated `network`/`tls` counts — those numbers reflect the
probing environment, not the relays.

## Deployment

The image is published to `ghcr.io/qntx/relays` on every `main` push. The
container runs `relays run` on an hourly cadence; mount a volume at
`/data` for the SQLite database. Set `RELAYS_STORE` to the artefact
destination — `file:///path` or `s3://bucket` — and for S3-compatible
stores (R2) the standard `AWS_*` credential variables. Serve the bucket
through a custom domain with CORS `GET`/`HEAD` so web clients can read the
artefacts.

## Testing

- `relays-core` tests cover URL normalisation vectors, the two-stage
  curation validation, derivation boundaries (uptime windows, RTT medians,
  the seven-day report rule), directory snapshots and schema validation.
- `relays` tests run against an in-process mock relay
  (`crates/relays/tests/common/`): canned HTTP statuses, EOSE/AUTH/CLOSED replies, the
  close handshake, garbage-TLS vs immediate-close classification, address
  fallback and policy rejections — all under `AddressPolicy::AllowLoopback`.
- `slow_upgrade_still_gets_a_full_read_budget` uses real time (~16 s): a
  6 s upgrade delay must still get the full 10 s read budget. Paused time
  is not used because the mock's `sleep` registers its timer only after a
  real socket event, so the virtual clock can jump past it and the test
  becomes racy.

## Dependencies

Only depend on versions published at least 7 days ago. Keep
`cargo deny check` green and exactly one rustls crypto provider (`ring`)
in the graph — verify with `cargo tree -i ring` / `cargo tree -i aws-lc-rs`.

## Versioning and release

Bump `[workspace.package].version`, update `CHANGELOG.md`, merge, then tag
`v*.*.*` on main. The image workflow publishes `ghcr.io/qntx/relays` on
every `main` push.

## Commits

English Conventional Commits: `type(scope): subject` (`feat`, `fix`, `docs`,
`style`, `refactor`, `perf`, `test`, `chore`, `ci`, `build`, `revert`),
subject in imperative mood, no period, ≤ 50 chars; `BREAKING CHANGE:` footer
for breaking changes.

## Documentation policy

`docs/` is local-only and gitignored; never commit it. Public behaviour is
documented in this file and `relays --help`.

## License

Contributions are dual-licensed under MIT OR Apache-2.0.
