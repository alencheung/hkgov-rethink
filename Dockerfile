# syntax=docker/dockerfile:1.7
#
# Multi-stage build for hkgov-rethink. The final image is a minimal runtime
# carrying only the hkgov-api binary + the static dashboard.
#
# Build:  docker build -t hkgov-rethink .
# Run:    docker run --rm -p 8080:8080 -e HKGOV_AGENT__ENABLED=true hkgov-rethink

ARG RUST_VERSION=1.96

# ---------- builder ----------
FROM rust:${RUST_VERSION}-slim AS builder

# Deps for building (reqwest uses rustls, so no openssl needed, but pkg-config
# + ca-certificates keep TLS roots sane at runtime).
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependencies: copy manifests first, build a dummy, then real source.
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY dashboard/ ./dashboard/
COPY examples/ ./examples/
COPY docs/ ./docs/
COPY config.toml ./

# Release build of the API binary only. Default features keep it zero-dep.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release -p hkgov-api && \
    cp /build/target/release/hkgov-api /hkgov-api

# ---------- runtime ----------
FROM debian:bookworm-slim AS runtime

# CA certs so reqwest can hit https://api.hkma.gov.hk; curl for healthchecks.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /hkgov-api /usr/local/bin/hkgov-api
COPY config.toml /app/config.toml
COPY dashboard/index.html /app/dashboard/index.html

# Non-root user.
RUN useradd --create-home --uid 10001 hkgov
USER hkgov

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS http://localhost:8080/health || exit 1

# All config is env-overridable (HKGOV_ prefix + __ separator).
ENTRYPOINT ["hkgov-api"]
