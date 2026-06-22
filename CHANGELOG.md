# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) for its
public API surface (the HTTP API + the `hkgov-py` client).

## [v8] — 2026-06-22 — Product layer: Lifeline + Signals + Investigations + Bilingual + Identity

> Completes the PM strategy feature set (P-100..P-108). All 8 planned features
> are now implemented; the remaining gaps from the strategy tracker are
> dashboard UI work (which composes against these APIs) + the Postgres
> persistence tier (roadmap).

### Added
- **P-104 Insight Lifeline** — evolution-aware `InsightStore::upsert`. The store
  now detects content changes on re-fire: a changed severity/title/summary/
  confidence/producer produces an `EvolutionDiff`, bumps the version, and
  archives the prior version to a history store. Content-stable re-fires are
  now no-ops (fixes a prior bug where every pass churned `generated_at`). New
  `first_seen`, `version`, `evolution` fields on `Insight` (all `serde-default`).
  `GET /v1/insights?since=` (the "what's new since you left" filter) +
  `GET /v1/insights/{id}/history`. (`crates/agent/src/insight.rs`)
- **P-102 Signal Subscriptions** — a `Signal` is a user-owned `ScanTarget` plus
  channel routing. v1 ships authoring + preview (stateless). `preview_signal`
  runs the real detector against the last 90 days so "preview IS what will
  fire" — the determinism guarantee holds. Routes: `POST /signals`,
  `POST /signals/preview`, `GET/PATCH/DELETE /signals/{id}`.
  (`crates/agent/src/signal.rs`)
- **P-105 Drill-In Investigation** — saved, resumable, shareable case files.
  From any insight, a user launches a multi-step investigation; each step
  (chip/qa/finding_promotion) is persisted. Routes: `POST /investigations`,
  `GET/DELETE /investigations/{id}`, `POST /investigations/{id}/steps`,
  `POST /investigations/{id}/notes`. (`crates/agent/src/investigation.rs`)
- **P-106 Bilingual Surface** — zh-HK insight summaries via a deterministic
  re-framer (`frame_zh_hk`) keyed by detector kind. No LLM, no scheduler change.
  `?lang=zh-HK` on `GET /v1/insights` selects the localized summary.
  (`crates/agent/src/bilingual.rs`)
- **P-108 Identity Tier** — email + magic-link identity. `POST /auth/request-
  token` issues a one-time token (provisioning the `User` idempotently);
  `POST /auth/redeem` exchanges it for a session; `GET /auth/me` resolves
  `Authorization: Bearer`. The `User.id` is the principal the per-user features
  key on as `owner`. (`crates/agent/src/identity.rs`)

### Fixed (critical)
- **`detect_threshold_crossing` was unreachable from the scheduler.** It existed
  and was tested but had no match arm in `run_one_target` — hard-blocked the
  flagship P-102 use case ("tell me when HIBOR breaks 2.5%"). Now wired, with a
  new `direction` field on `ScanTarget` (above/below, defaults above).

### Tests
- +42 tests since v7 (8 lifeline + 6 signal + 7 investigation + 9 bilingual +
  8 identity + 4 threshold-crossing). Workspace total 136 → **178**, all
  passing. clippy + fmt clean.

## [v7] — 2026-06-22 — Product layer: Silence Index + Unprecedentedness + Cite-It

> First features shipped from the PM/UX strategy
> (`docs/PM_STRATEGY/`): the highest-RICE, fully self-contained features
> that compose from existing deterministic detectors and need no
> identity/persistence infrastructure.

### Added
- **Silence Index (P-100, RICE 12,000)** — the flagship. A versioned, HKMA-scoped
  `0–100` opacity score ("how much did HKGOV not explain this period") built as a
  pure-Rust rollup of `cross_source_gap` + unattributed `series_jump` +
  missing-data days. Methodology v1.0; weights + the half-saturation squash
  constant are centralized so a methodology bump is a one-line change. New
  endpoint `GET /v1/silence-index?period=2026-Q2`. The determinism guarantee is
  the defense against "your opacity score is biased": critics reproduce it from
  the evidence. (`crates/agent/src/silence.rs`)
- **Unprecedentedness Score (P-103, RICE 10,667)** — the historical-rarity layer.
  Scores a numeric value against its own stored history: percentile rank, a
  `median ± k·MAD` "normal range" band, a 1-in-N return period, and the most
  recent prior exceedance ("last time this happened was ___"). New endpoint
  `GET /v1/unprecedentedness?source=hkma&dataset=…&field=…&value=…`. Composes
  from the same MAD math the `outlier` detector uses. (`crates/agent/src/unprecedentedness.rs`)
- **Cite-It (P-101, RICE 4,000)** — the citation/reproducibility moat. From any
  insight → a stable permalink + citation strings (BibTeX/RIS/APA/Chicago/Markdown)
  + a `ReproducibilityManifest` whose SHA-256 content hash detects upstream data
  drift (so a citation never false-claims reproducibility). New endpoint
  `GET /v1/insights/{id}/cite[?format=…&base_url=…]`. Experimental findings carry
  an honesty marker in every rendered string. (`crates/agent/src/cite.rs`)

### Added (infrastructure)
- `InsightStore::get(id)` — by-id insight lookup (powers `/cite`; P-104 will
  reuse it for the permalink landing + evolution tracking).
- `Error::NotFound` (404) + `Error::BadRequest` (400) — two common error variants
  with status-code + `kind_for` mappings.

### Tests
- +46 tests since v6 (31 for P-100/P-103 + 15 for P-101). Workspace total
  90 → **136**, all passing. clippy + fmt clean across all feature combinations.

### Documentation
- New `FEATURES_TRACKER.md` section J (F-089 → F-107) covers all three features
  with expected-behaviour specs and the unit/route tests backing each.
- PM strategy docs (`docs/PM_STRATEGY/`) are the design rationale; this entry is
  the shipped implementation of its R1.3 (P-103), R2.3 (P-101) and R2.4 (P-100)
  rows.

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
