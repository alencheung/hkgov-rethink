# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) for its
public API surface (the HTTP API + the `hkgov-py` client).

## [v6] — 2026-06-21 — Intelligence & agentic analysis layer

### Added
- **Richer detectors:** `outlier` (MAD robust z-score), `seasonality`
  (autocorrelation at monthly/quarterly lags), `correlation` (Pearson r flagging
  decoupling). `cross_source_gap` generalized to take `(source, dataset)`.
- **Config-driven scan:** `[[agent.scan]]` controls which detectors run on which
  datasets; empty list falls back to the v3 defaults.
- **Agent tool belt:** `list_datasets` / `query_dataset` / `run_detector` behind
  a uniform `Tool` trait with OpenAI-compatible schemas (`crates/agent/src/tools.rs`).
- **Agentic loop:** `LlmClient::step` drives multi-step reasoning, bounded by
  `max_steps`; heuristic client opts out via default impl (`loop_mod.rs`).
- **NL Q&A:** `POST /v1/ask` with heuristic keyword fallback (`qa.rs`).
- **Proactive alerting:** `AlertDispatcher` pushes severity-filtered, deduped
  insights to webhook sinks; `GET /v1/alerts`; `WebhookSink` behind the new
  `alerts` feature.
- `Error::Agent` variant; `InsightSeverity` implements `Display`.

### Fixed
- `Finding::into_insight` id now includes `dataset` (was `{kind}:{source}:{hash}`,
  which would collide as coverage widened).

### Tests
- 54 across default features; 55 with `--all-features`. Clippy + fmt clean across
  all feature combinations.

## [v5] — Public surface

### Added
- Insights dashboard (`dashboard/index.html`).
- Python API example (`examples/query_api.py`).
- CONTRIBUTING guide with data-source verification rules and feature matrix.

## [v4] — Scale & hardening

### Added
- Postgres `RecordStore` (`--features pg`).
- API auth (optional `X-API-Key` / `?api_key=`) + `/v1` versioning.
- OpenTelemetry trace export (`--features otel`).
- k6 load-test harness + capacity model.

## [v3] — AI-agent analysis layer

### Added
- `crates/agent`: pluggable LLM client trait + `HeuristicClient` + `HttpLlmClient`.
- Detectors: `series_jump`, `cross_source_gap`.
- Structured `Insight` records with `EvidenceRef`s.
- `/insights` endpoint; agent supervisor decoupled from serving.

## [v2] — More sources + resilience + shared cache

### Added
- `data.gov.hk`, press, and LandsD/CSDI connectors.
- Per-source rate limiting (token bucket) + circuit breakers.
- Redis `RecordStore` (`--features redis`).
- `/health/sources` endpoint.

## [v1] — Foundation

### Added
- Cargo workspace, Rust 1.96.
- HKMA connector (retry, backoff, live-verified).
- `moka` cache-first `RecordStore` + trait.
- Ingestion supervisor (per-dataset background refresh).
- axum API: `/health`, `/sources`, `/datasets/{source}/{dataset}[/records]`.
- Config (`config.toml` + `HKGOV_` env overrides) + telemetry + graceful shutdown.

---

<!-- Tags: v1 (ed4dbe2), v2–v5 (a967dda), v6 (cb26750). See `git tag`. -->
