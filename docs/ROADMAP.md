# Roadmap

The end goal — **100k concurrent users** served from **AI-agent-generated
insights** over consolidated HKGOV data — is a multi-month build. This roadmap
breaks it into milestones that each ship something runnable.

## v1 — Foundation (✅ shipped)

- Cargo workspace, Rust 1.96, clippy/fmt/test green, CI.
- HKMA connector (retry, backoff, verified live).
- `moka` cache-first `RecordStore` + trait.
- Ingestion supervisor (per-dataset background refresh).
- axum API: `/health`, `/sources`, `/datasets/:source/:dataset[/records]`.
- Config + telemetry + graceful shutdown.

## v2 — More sources + shared cache

- `data.gov.hk` connector (v2 filter + historical archive — endpoints verified,
  see [DATA_SOURCES.md](DATA_SOURCES.md)).
- Press/RSS connector (info.gov.hk archive + news.gov.hk RSS).
- CSDI / open LandsD tile connector.
- **Redis** `RecordStore` implementation (the multi-node enabler).
- Per-source rate limiting and circuit breaker.

## v3 — AI-agent analysis layer

- `crates/agent`: pluggable LLM client, prompt/tool scaffolding.
- Cross-source anomaly detection (HKMA data vs. ISD press releases).
- "Insights" stored as records and served via `/insights`.
- Scheduled agent runs decoupled from serving.

## v4 — Scale & hardening

- Stateless API behind LB, N replicas.
- Postgres read replicas for historical reads.
- Auth, rate limiting at the edge, API versioning.
- OpenTelemetry exporters wired into `telemetry`.
- Load-test harness (k6/oha) + a published capacity model targeting 100k.

## v5 — Public surface

- Public docs site, example notebooks.
- Web dashboard over the API.
- Contribution guide for new connectors.
