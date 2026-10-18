# syntax=docker/dockerfile:1.9
#
# Multi-stage build for the `relays` directory monitor.
#
# * Stage 1 (`chef`)    -- pin cargo-chef, the dependency-graph caching tool.
# * Stage 2 (`planner`) -- emit recipe.json describing the dep graph only.
# * Stage 3 (`builder`) -- cook deps from the recipe, then compile the bin.
# * Stage 4 (`runtime`) -- distroless runtime image, non-root.
#
# Multi-arch: the base images are multi-arch out of the box.
# The runtime image is distroless; TLS roots are compiled in via
# webpki-roots so no CA bundle is needed.

ARG RUST_VERSION=1.98

FROM rust:${RUST_VERSION}-slim-bookworm AS chef
ENV CARGO_TERM_COLOR=always
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      pkg-config build-essential ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked --version ^0.1
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
# Cook deps once; rebuilds reuse this layer until the dep graph changes.
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --locked -p relays && strip target/release/relays
# distroless has no shell; create the data volume directory here and copy it
# into the runtime image owned by the nonroot user (65532) so a fresh named
# volume is writable.
RUN mkdir -p /data

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
LABEL org.opencontainers.image.title="relays" \
      org.opencontainers.image.description="Nostr relay directory monitor" \
      org.opencontainers.image.source="https://github.com/qntx/relays" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"

COPY --from=builder /src/target/release/relays /relays
COPY relays.toml /etc/relays/relays.toml
COPY --from=builder --chown=65532:65532 /data /data

ENV RELAYS_CURATION=/etc/relays/relays.toml \
    RELAYS_DB=/data/relays.db \
    RUST_LOG=info

VOLUME ["/data"]
USER nonroot:nonroot

ENTRYPOINT ["/relays"]
CMD ["run"]
