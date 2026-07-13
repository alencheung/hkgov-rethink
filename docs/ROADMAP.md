# Roadmap

The end goal — **100k concurrent users** (a design target, not yet a verified
measurement — see [docs/CAPACITY.md](CAPACITY.md)) served from **AI-agent-generated
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
- **API auth** (optional `X-API-Key`; the `?api_key=` query fallback was later
  removed as security hardening) + **API versioning**
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

## v7 — Product layer (✅ shipped)

The first features from the PM strategy, turning the agent's findings into a
citable, quotable product surface.

- **Silence Index** — `GET /v1/silence-index?period=`. A deterministic 0–100
  "opacity" score: how much did HKGOV not explain this period? Built from the
  cross-source gaps + unattributed moves, so the same critique anyone levels at
  the score can be checked against the exact missing dates.
- **Unprecedentedness Score** — `GET /v1/unprecedentedness?...`. Percentile
  rank, normal-range band, 1-in-N return period, and "last exceeded" comparator
  for a value against its history.
- **Cite-It** — `GET /v1/insights/{id}/cite?format=`. A stable permalink +
  citation strings (BibTeX/RIS/APA/Chicago/Markdown) + a CI-reproducibility
  manifest (SHA-256 over the evidence) so a citation never false-claims
  reproducibility.
- New detectors: `year_over_year`, `proxy_divergence`, `benchmark_deviation`,
  `threshold_crossing`. Each is pure-Rust and deterministic.

## v8 — Product layer II (✅ shipped)

Turns the one-shot findings into a trackable, subscribable, bilingual product.

- **Insight Lifeline** — `GET /v1/insights?since=` + `GET /v1/insights/{id}/history`.
  Evolution tracking: what's new since a timestamp, and prior versions of an
  insight.
- **Signal Subscriptions** — `POST /v1/signals` / `POST /v1/signals/preview`.
  Author a detector watch in natural language; the preview reuses the
  scheduler's detector dispatch so "preview IS what will fire" (the D-006 fix).
- **Drill-In Investigations** — `POST /v1/investigations` + `/steps` + `/notes`.
  Saved, resumable, shareable case files from any insight.
- **Bilingual (zh-HK)** — deterministic zh-HK summary reframers; `?lang=zh-HK`.
- **Identity Tier** — `POST /v1/auth/request-token` / `/redeem` / `GET /me`.
  Email + magic-link; the principal for per-user state. Sessions now expire.
- **Market Players** — `GET /v1/market-players`. A curated directory of the
  named private-sector operators holding each department's licences.
- `threshold_crossing` detector wired into the scheduler; `trend_break`
  detector added (regime-change detection).

## Remaining (future)

- ISD/info.gov.hk HTML scraping + news.gov.hk RSS (press connector v2).
- More `data.gov.hk` resources (each must be probe-verified first).
- Persisting insights to the Postgres tier (currently in-process).
- Deploy manifests (k8s/Helm), OTLP collector config, production hardening.
- Auth via OAuth/JWT (current is static key).
- Generalize `ToolBelt` / `AgentSupervisor` to `Arc<dyn RecordStore>` so the
  agent works against Redis/Postgres backends (currently `Arc<MemoryStore>`).
- **Wire Redis/Postgres store backends into the binary.** `RedisStore`
  (`--features redis`) and `PgStore` (`--features pg`) are implemented behind
  feature flags and have tests, but `build_store` in `main.rs` only
  instantiates `MemoryStore` — selecting `redis` or `pg` produces a loud
  startup error. Connecting them also means addressing known issues:
  `RedisStore` serializes the whole dataset as one blob (re-key per record),
  and `PgStore` guards its client behind a single `Mutex` (would serialize
  under load). See [docs/CAPACITY.md](CAPACITY.md).
- **Validate the 100k concurrency target with a real load test against an LB
  tier.** The k6 harness exists but defaults to 500 VUs (a smoke test, not a
  ceiling test) and is not in CI. The 100k figure is a design target; verifying
  it requires the load-balancer tier (the v3 stage, not yet built) in front of
  N replicas, plus a k6 run scaled into the tens of thousands of VUs.
