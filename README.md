<div align="center">

# hkgov-rethink

**Turn Hong Kong's fragmented open data into one queryable, monitorable, AI-ready surface.**

A unified, high-throughput layer over HKGOV public data — built so that any AI IDE,
MCP server, or A2A agent can pull normalized data, watch it for change, and cross-
check facts across sources.

[![CI](https://github.com/alencheung/hkgov-rethink/actions/workflows/ci.yml/badge.svg)](https://github.com/alencheung/hkgov-rethink/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.96%2B-orange.svg)](https://www.rust-lang.org)

</div>

---

## The problem this solves

Hong Kong publishes a huge amount of public data — but it is spread across
**independent portals that don't know each other exist**:

| Portal | What's there | What you *can't* do through it |
|---|---|---|
| [data.gov.hk](https://data.gov.hk/en/) | ~4,000 datasets, cross-departmental | No CKAN action API (`/api/3/action/*` 404s); no joins across datasets |
| [HKMA Open API](https://apidocs.hkma.gov.hk/) | Monetary & market statistics, press | Only HKMA data; can't be correlated with other bureaux |
| [info.gov.hk](https://www.info.gov.hk/gia/general/today.htm) | Press releases (1997→) | HTML only; no structured API; no link to the stats they announce |
| [CSDI](https://portal.csdi.gov.hk/) | ~1,100 geospatial datasets | Catalogs, not a normalized query layer |

So today, answering a simple question like **"does what the HKMA press release
said match the numbers the HKMA API published on the same day?"** means scraping
HTML, hitting three different API dialects, and writing the reconciliation by hand.

**`hkgov-rethink` does that reconciliation once, and serves the result as one
fast API and (on the roadmap) one AI-agent surface.**

## What it delivers that the public APIs can't

1. **One normalized model.** Every upstream — HKMA's `{header, result:{records}}`
   envelope, data.gov.hk's bare arrays, press HTML/RSS — is collapsed onto a
   single `NormalizedRecord` shape. Your consumer code speaks one dialect,
   regardless of source.
2. **Cross-source joins for validation.** Join monetary data against press
   releases to surface *"the release says X, the data says Y"* — the kind of
   fact-check the individual portals structurally cannot do.
3. **Always-warm cache.** A cache-first serving layer means reads never hit the
   upstream and never depend on a government portal being responsive.
4. **Built for AI consumers.** The API is the substrate for AI IDEs, MCP servers,
   and A2A agents to **get data, run analysis, monitor for change, detect
   anomalies, and correlate points across sources to validate facts and trends.**
5. **Resilient by construction.** Per-source rate limiting + circuit breakers so
   one degraded HKGOV endpoint can't take the rest down.

## Designed to be plugged into your AI tooling

The normalized read API is intentionally agent-friendly: stable record IDs, typed
cell values, paginated responses, and CORS/gzip/timeout wired in. That makes it a
natural backend for:

- **AI IDEs** (Cursor, Copilot-style assistants, custom LLM apps) that want a
  single trustworthy data source for HK government facts.
- **MCP (Model Context Protocol) servers** — expose `/datasets/.../records` as a
  tool so a model can query live HKGOV data without learning four APIs.
- **A2A (agent-to-agent) workflows** — agents that monitor for change, flag
  anomalies, and cross-reference press vs. data on your behalf.

> The MCP/A2A adapters and the anomaly-detection agent are on the roadmap
> ([docs/ROADMAP.md](docs/ROADMAP.md)) — the read API and the normalized store
> they need are here today. PRs welcome.

## Status — v0.2 (multi-source + resilience)

✅ **In the codebase now:**

- **4 live source families** (all verified against real endpoints — see
  [docs/DATA_SOURCES.md](docs/DATA_SOURCES.md)):
  - **HKMA** — `capital-market-statistics`, `daily-interbank-liquidity`
    (retry + exponential backoff).
  - **data.gov.hk** — v2 filter API (`money-lenders-licensees`, `hko-rainfall-warning`).
  - **Press** — HKMA press releases API (`hkma-press-releases`).
  - **LandsD / CSDI** — open dataset catalog via the data.gov.hk archive.
- **Resilience layer** — per-source token-bucket rate limiter + three-state
  circuit breaker wrapping every outbound call ([`crates/connectors/src/resilience.rs`](crates/connectors/src/resilience.rs)).
- **Cache-first store** — `moka` in-process cache behind a `RecordStore` trait,
  with a **Redis** implementation (behind the `redis` feature) for multi-node fleets.
- **Ingestion supervisor** — per-dataset background refresh tasks; one slow source
  never blocks another.
- **Read API** (`axum` + `tokio` + `tower`): `/health`, `/sources`,
  `/datasets/{source}/{dataset}`, `/datasets/{source}/{dataset}/records`, with
  timeout / gzip / CORS / tracing middleware.
- **Config** via `config.toml` + `HKGOV_` env overrides; structured `tracing`.

🗓️ **On the roadmap** ([docs/ROADMAP.md](docs/ROADMAP.md)):

- **AI-agent analysis layer** — pluggable LLM client, cross-source anomaly
  detection, insights served via `/insights`.
- **MCP server adapter** + **A2A protocol surface** so the platform is a first-
  class tool for agents.
- ISD/info.gov.hk HTML scraping + news.gov.hk RSS ingestion.
- Edge auth, rate limiting, OpenTelemetry, load-test harness toward the 100k-
  concurrency target.

## Quick start

```bash
# Requires Rust 1.96+ (rustup recommended).
cargo run --release -p hkgov-api
```

The server boots on `0.0.0.0:8080`, warms its cache from all configured sources
in the background, and serves:

```bash
curl http://localhost:8080/health
curl http://localhost:8080/sources
curl 'http://localhost:8080/datasets/hkma/capital-market-statistics/records?offset=0&limit=5'
curl 'http://localhost:8080/datasets/press/hkma-press-releases/records?limit=3'
```

### Multi-node (shared Redis cache)

```bash
cargo build --release --features hkgov-store/redis
# point the store at Redis via config/env (see config.toml)
```

### Configuration

Defaults live in [`config.toml`](config.toml); override anything with a `HKGOV_`
prefix and `__` as the nested separator:

```bash
HKGOV_API__BIND=0.0.0.0:9090
HKGOV_LOG__FORMAT=json          # plain | json
HKGOV_UPSTREAM__HKMA_API_KEY=…  # optional, for higher HKMA quota
```

## Tests & quality

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

CI runs fmt + clippy (with `-D warnings`) + build + test on every push.

## Project layout

```
crates/
  common/      config, normalized model, errors, telemetry
  connectors/  one client per upstream family + resilience layer + registry
  store/       cache-first RecordStore (moka; Redis behind feature flag)
  ingest/      per-dataset refresh scheduler
  api/         the public axum binary (the only thing deployed)
docs/          ARCHITECTURE, DATA_SOURCES (verified endpoints), ROADMAP
```

Data flows one way: **upstream → connectors → ingest → store → api → client.**
The API never calls a connector directly — it only reads from the store. That
decoupling is what lets the platform serve high concurrency without saturating
the (free, shared) HKGOV upstreams. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Contributing

This project is early and collaborative by design. The most valuable
contributions right now:

- **New connectors** — each HKGOV portal follows a small, consistent pattern
  (implement the `Connector` trait, register in `Registry`). Adding a source is
  self-contained. See [docs/DATA_SOURCES.md](docs/DATA_SOURCES.md) for verified
  endpoints.
- **The MCP server adapter** — turn the read API into an MCP tool so AI IDEs can
  query HKGOV data directly.
- **The agent layer** — cross-source anomaly/narrative detection over the
  normalized store.
- **Hardening** — Postgres store, OTel exporters, load-testing toward 100k.

Please keep the project polite to HKGOV endpoints: defaults are conservative
(`hkma_rate_per_sec=5`, bounded retries, gzip on). Don't raise rate limits
without coordination. Open an issue first for larger changes.

## License

MIT — see [LICENSE](LICENSE).
