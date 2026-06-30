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
# NOTE: no `--mount=type=cache` here — Railway's Metal builder (and other
# non-BuildKit Docker engines) reject that flag. We rely on plain layer
# caching instead: rebuilding from a source change recompiles dependencies
# too, so expect a longer build (~10-15 min) on code changes.
RUN cargo build --release -p hkgov-api && \
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

# PR-004: probe /ready (not /health). /health is pure liveness (process up);
# /ready folds in circuit-breaker state + warm cache, so a container with open
# circuits or an empty cache is marked unhealthy and stops receiving traffic.
# curl -f fails the check on the 503 a degraded /ready returns.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS http://localhost:8080/ready || exit 1

# All config is env-overridable (HKGOV_ prefix + __ separator).
ENTRYPOINT ["hkgov-api"]
