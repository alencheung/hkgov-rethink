# hkgov-rethink

> Consolidated, AI-infused insights from Hong Kong Government public data —
> not just a chat over data, but agentic monitoring that surfaces what the HKGOV
> press room leaves unsaid.

`hkgov-rethink` is a Rust platform that ingests Hong Kong Government open data
(monetary, statistical, geospatial, and press), normalizes it onto one model,
serves it through a high-concurrency cache-first API, and runs an AI agent layer
that cross-references sources to detect anomalies, gaps, and narratives the
official press releases don't spell out.

## What's here (v1–v5 shipped)

| Milestone | Status |
|---|---|
| **v1** Foundation — HKMA connector, cache-first store, axum API | ✅ |
| **v2** data.gov.hk + press + LandsD connectors, rate limiting, circuit breakers, Redis store | ✅ |
| **v3** AI-agent layer — anomaly detection, cross-source gaps, `/insights`, heuristic + LLM clients | ✅ |
| **v4** Postgres store, API auth + `/v1` versioning, OpenTelemetry, k6 load harness | ✅ |
| **v5** Insights dashboard, examples, CONTRIBUTING | ✅ |

Full detail in [docs/ROADMAP.md](docs/ROADMAP.md).

## Quick start

```bash
cargo run --release -p hkgov-api
```

Boots on `0.0.0.0:8080`, warms its cache from HKGOV sources, and serves:

```bash
curl http://localhost:8080/health
curl http://localhost:8080/v1/sources
curl 'http://localhost:8080/v1/datasets/hkma/capital-market-statistics/records?limit=5'
curl 'http://localhost:8080/v1/insights?limit=5'
```

Enable the AI agent (heuristic mode, no API key needed):

```bash
HKGOV_AGENT__ENABLED=true cargo run -p hkgov-api
```

Then open [dashboard/index.html](dashboard/index.html) in a browser (point it at
`http://localhost:8080`) to see live source health + AI-generated insights.

## Feature flags

Optional backends/integrations, off by default so the build needs no external
services:

```bash
cargo build -p hkgov-store --features redis    # Redis cache tier
cargo build -p hkgov-store --features pg       # Postgres persistent tier
cargo build -p hkgov-common --features otel    # OpenTelemetry export
cargo build -p hkgov-api --features llm        # HTTP LLM client for insights
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full matrix.

## Configuration

[`config.toml`](config.toml) with env overrides using a `HKGOV_` prefix and
`__` separator, e.g. `HKGOV_API__BIND=0.0.0.0:9090`,
`HKGOV_AGENT__ENABLED=true`.

## Tests

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Live HKMA integration tests are behind `cargo test -p hkgov-connectors --features live`.

## Layout

```
crates/
  common/      config, normalized model, errors, telemetry (otel-able)
  connectors/  HKMA, data.gov.hk, press, LandsD + rate limiting/circuit breakers
  store/       cache-first RecordStore: moka (default) / redis / pg
  ingest/      per-dataset refresh scheduler
  agent/       LLM client trait + heuristic/http, anomaly detectors, insights
  api/         the public axum binary (auth, /v1 versioning, /insights)
dashboard/     static insights dashboard
docs/          ARCHITECTURE, DATA_SOURCES, ROADMAP, CAPACITY
loadtest/      k6 harness
examples/      Python API client
```

## Data sources

All public HKGOV open data — see [docs/DATA_SOURCES.md](docs/DATA_SOURCES.md)
for the verified endpoint table. The government-only
`api.portal.hkmapservice.gov.hk` is intentionally excluded; we use the open
LandsD/CSDI endpoints on data.gov.hk instead.

## Scaling to 100k

Single-node → fleet path with the numbers: [docs/CAPACITY.md](docs/CAPACITY.md).
The `RecordStore` trait absorbs each backing-store swap; no other code changes.

## License

MIT — see [LICENSE](LICENSE).
