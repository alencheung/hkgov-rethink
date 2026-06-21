# hkgov-rethink

> Consolidated, AI-infused insights from Hong Kong Government public data —
> not just a chat over data, but agentic monitoring that surfaces what the HKGOV
> press room leaves unsaid.

`hkgov-rethink` is a Rust platform that ingests Hong Kong Government open data
(monetary, statistical, geospatial, and press), normalizes it onto one model,
serves it through a high-concurrency cache-first API, and — on the roadmap —
runs an AI agent layer that cross-references sources to detect anomalies, gaps,
and narratives the official press releases don't spell out.

## Why

Hong Kong publishes a large amount of public data across fragmented portals
([data.gov.hk](https://data.gov.hk/en/), the [HKMA open API
portal](https://apidocs.hkma.gov.hk/), [CSDI](https://portal.csdi.gov.hk/),
[info.gov.hk press releases](https://www.info.gov.hk/gia/general/today.htm)).
Each is useful alone; the value multiplies when they're joined and monitored
together. This project does the joining and the monitoring, and exposes the
result as a fast public API and (later) as AI-generated insight.

## Status — v1 (foundation)

This is the first milestone. It delivers the substrate the AI-agent layer will
sit on:

- ✅ Cargo workspace (5 crates), Rust 1.96, clippy-clean, CI-green.
- ✅ **HKMA connector** — typed client with retry + exponential backoff against
  `api.hkma.gov.hk/public/...`, verified live (capital-market-statistics,
  daily-interbank-liquidity).
- ✅ Cache-first store (`moka`, in-process, TTL'd) behind a `RecordStore` trait
  ready to swap for Redis/Postgres.
- ✅ Ingestion supervisor — per-dataset background refresh tasks.
- ✅ Async serving API (`axum` + `tokio` + `tower`): `/health`, `/sources`,
  `/datasets/:source/:dataset`, `/datasets/:source/:dataset/records`, with
  timeout / gzip / CORS / tracing middleware.
- ✅ Config via `config.toml` + `HKGOV_` env overrides; structured `tracing`.

Deferred to later milestones (tracked in [docs/ROADMAP.md](docs/ROADMAP.md)):

- The AI-agent analysis layer (LLM orchestration, anomaly/narrative detection).
- `data.gov.hk`, press/RSS, and CSDI/LandsD connectors (trait + stubs exist).
- Multi-node scaling (Redis cache, Postgres store, LB, load-test harness, OTel).

## Quick start

```bash
# Requires Rust 1.96+ (rustup recommended).
cargo run --release -p hkgov-api
```

The server boots on `0.0.0.0:8080`, warms its cache from HKMA in the
background, and serves:

```bash
curl http://localhost:8080/health
curl http://localhost:8080/sources
curl 'http://localhost:8080/datasets/hkma/capital-market-statistics/records?offset=0&limit=5'
```

Configuration is in [`config.toml`](config.toml); env overrides use a `HKGOV_`
prefix with `__` as the nested separator, e.g. `HKGOV_API__BIND=0.0.0.0:9090`.

## Run the tests

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

A live HKMA integration test is gated behind the `live` feature so CI stays
hermetic:

```bash
cargo test -p hkgov-connectors --features live -- --nocapture
```

## Layout

```
crates/
  common/      config, normalized model, errors, telemetry
  connectors/  one client per upstream family (v1: HKMA; others stubbed)
  store/       cache-first RecordStore (moka; trait for Redis/Postgres later)
  ingest/      per-dataset refresh scheduler
  api/         the public axum binary
docs/          ARCHITECTURE, DATA_SOURCES (verified endpoints), ROADMAP
```

## Data sources

See [docs/DATA_SOURCES.md](docs/DATA_SOURCES.md) for the verified endpoint table.
All sources are public HKGOV open data. The government-only
`api.portal.hkmapservice.gov.hk` is intentionally excluded; we use the open
LandsD tile APIs on data.gov.hk and CSDI instead.

## License

MIT — see [LICENSE](LICENSE).
