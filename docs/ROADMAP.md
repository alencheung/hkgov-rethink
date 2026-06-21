# Roadmap

The end goal — **100k concurrent users** served from **AI-agent-generated
insights** over consolidated HKGOV data. This roadmap tracks what shipped and
what remains. Each milestone is independently runnable.

## v1 — Foundation (✅ shipped)

- Cargo workspace, Rust 1.96, clippy/fmt/test green, CI.
- HKMA connector (retry, backoff, verified live).
- `moka` cache-first `RecordStore` + trait.
- Ingestion supervisor (per-dataset background refresh).
- axum API: `/health`, `/sources`, `/datasets/{source}/{dataset}[/records]`.
- Config + telemetry + graceful shutdown.

## v2 — More sources + resilience + shared cache (✅ shipped)

- `data.gov.hk` connector (v2 filter + historical archive — verified, see
  DATA_SOURCES.md).
- Press connector (HKMA press releases API, verified).
- LandsD/CSDI connector (open catalog via data.gov.hk archive — gov-only map
  API excluded).
- Per-source **rate limiting** (token bucket) + **circuit breaker** wrapping
  every connector.
- **Redis** `RecordStore` implementation (`--features redis`) — the multi-node
  cache enabler.
- `/health/sources` endpoint exposing circuit-breaker state.

## v3 — AI-agent analysis layer (✅ shipped)

- `crates/agent`: pluggable LLM client trait + **heuristic** client
  (deterministic, zero-config) + **HTTP** client (OpenAI-compatible, behind
  `llm` feature).
- Cross-source detectors: `series_jump` (numeric anomalies) and
  `cross_source_gap` (press dates vs data dates).
- Structured `Insight` records with verifiable evidence pointers.
- `/insights` endpoint serving insights; agent scheduler decoupled from
  serving.
- Live-verified: agent detects real HKMA market moves (HIBOR drops, HSI swings).

## v4 — Scale & hardening (✅ shipped)

- **Postgres** `RecordStore` (`--features pg`) for the persistent cold/historical
  tier.
- **API auth** (optional `X-API-Key` / `?api_key=`) + **API versioning**
  (`/v1` prefix; health kept at root for probes).
- **OpenTelemetry** trace export (`--features otel`).
- **Load-test harness** (k6) + capacity model with the
  single-node → 100k-concurrency scaling path.

## v5 — Public surface (✅ shipped)

- Insights dashboard (`dashboard/index.html`) — static, reads the live API,
  renders source health + insights with evidence.
- Python API example (`examples/query_api.py`).
- CONTRIBUTING guide with data-source verification rules and the feature matrix.

## Remaining (future)

- ISD/info.gov.hk HTML scraping + news.gov.hk RSS (press connector v2).
- More `data.gov.hk` resources (each must be probe-verified first).
- Persisting insights to the Postgres tier (currently in-process).
- Deploy manifests (k8s/Helm), OTLP collector config, production hardening.
- Auth via OAuth/JWT (current is static key).
