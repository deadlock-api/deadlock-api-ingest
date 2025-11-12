FROM rust:1.91-slim-trixie AS chef
RUN apt-get update \
    && apt-get install -y --no-install-recommends sccache ca-certificates gcc libssl-dev pkg-config cmake build-essential clang
RUN cargo install --locked cargo-chef
ENV RUSTC_WRAPPER=sccache SCCACHE_DIR=/sccache
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
WORKDIR /app
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    cargo build --release --bin deadlock-api-ingest

# We do not need the Rust toolchain to run the binary!
FROM debian:trixie-slim AS runtime
LABEL org.opencontainers.image.source="https://github.com/deadlock-api/deadlock-api-ingest" \
      org.opencontainers.image.description="Deadlock API ingest service" \
      org.opencontainers.image.licenses="MIT"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl-dev openssl libc6 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/deadlock-api-ingest /usr/local/bin
ENTRYPOINT ["/usr/local/bin/deadlock-api-ingest"]
