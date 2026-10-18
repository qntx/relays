# Multi-stage build: the runtime image is distroless; TLS roots are compiled
# in via webpki-roots so no CA bundle is needed.
FROM rust:slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/core crates/core
COPY crates/relays crates/relays
RUN cargo build --release --locked -p relays \
    && cp target/release/relays /usr/local/bin/relays

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /usr/local/bin/relays /usr/local/bin/relays
COPY relays.toml /etc/relays/relays.toml
ENV RELAYS_CURATION=/etc/relays/relays.toml \
    RELAYS_DB=/data/relays.db
VOLUME ["/data"]
ENTRYPOINT ["/usr/local/bin/relays"]
CMD ["run"]
