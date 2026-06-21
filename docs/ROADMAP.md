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

## v6 — Intelligence & agentic analysis layer (✅ shipped)

The v3 agent was passive and deterministic: two detectors on hardcoded targets.
v6 makes it genuinely agentic while preserving the determinism-first principle
— the LLM gains autonomy over *what to investigate* and *how to answer*, but
every finding still originates from a pure-Rust detector.

- **Richer intelligence** — three new detectors (`outlier` via MAD robust z-score,
  `seasonality` via autocorrelation, `correlation` via Pearson r) and a
  generalized `cross_source_gap`. Scan targets moved to config (`[[agent.scan]]`)
  so coverage widens without code changes; empty list = the v3 defaults.
- **Agent tool belt** — `list_datasets` / `query_dataset` / `run_detector`
  wrapped behind a uniform `Tool` trait with OpenAI-compatible schemas
  (`crates/agent/src/tools.rs`). Both the periodic scan and the agent loop call
  through it.
- **Agentic investigation loop** — `LlmClient::step` drives a multi-step
  conversation (tool call → execute → reason → finalize), bounded by `max_steps`
  so a misbehaving model can't loop. Heuristic clients opt out via the default
  impl (`crates/agent/src/loop_mod.rs`).
- **Natural-language Q&A** — `POST /v1/ask`. Rich mode runs the agent loop;
  heuristic mode does keyword→dataset matching so the endpoint is useful with no
  LLM key. Dashboard + Python example updated.
- **Proactive alerting** — `AlertDispatcher` pushes qualifying insights
  (severity ≥ threshold, deduped by id) to webhook sinks. `WebhookSink` is
  behind the `alerts` feature; `GET /v1/alerts` exposes the dispatch log
  (`crates/agent/src/alerts.rs`).
- New `Error::Agent` variant; `InsightSeverity` now implements `Display`.

Feature gating: new detectors + tool belt + agent loop + `/ask` endpoint ship
unconditional (no new deps, heuristic baseline intact). `HttpLlmClient::step`
extends the `llm` feature. `WebhookSink` adds the `alerts` feature
(`alerts = ["dep:reqwest"]`).

## Remaining (future)

- ISD/info.gov.hk HTML scraping + news.gov.hk RSS (press connector v2).
- More `data.gov.hk` resources (each must be probe-verified first).
- Persisting insights to the Postgres tier (currently in-process).
- Deploy manifests (k8s/Helm), OTLP collector config, production hardening.
- Auth via OAuth/JWT (current is static key).
- Generalize `ToolBelt` / `AgentSupervisor` to `Arc<dyn RecordStore>` so the
  agent works against Redis/Postgres backends (currently `Arc<MemoryStore>`).
