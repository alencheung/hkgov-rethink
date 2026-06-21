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

## v2 — More sources + shared cache (✅ shipped)

- `data.gov.hk` connector (v2 filter API: `money-lenders-licensees`,
  `hko-rainfall-warning`; historical archive listing — endpoints verified in
  [DATA_SOURCES.md](DATA_SOURCES.md)).
- Press connector (HKMA press releases API: `hkma-press-releases`).
- LandsD / CSDI connector (open dataset catalog via the data.gov.hk archive).
- **Redis** `RecordStore` implementation behind the `redis` feature flag
  (the multi-node enabler).
- Per-source **rate limiter** (token bucket) + **circuit breaker** wrapping
  every outbound connector call.

Still outstanding from the original v2 scope:
- ISD/info.gov.hk HTML scraping and news.gov.hk RSS ingestion (HKMA press API
  is in; the broader press archive is not).
- More dataset mappings within the existing sources (adding a dataset is one
  entry per source's resource table).

## v3 — AI-agent analysis + agent protocols

The agent-facing surface. Everything that lets an AI IDE, MCP server, or A2A
agent treat this platform as a first-class data source.

- `crates/agent`: pluggable LLM client, prompt/tool scaffolding.
- Cross-source anomaly detection (HKMA data vs. ISD press releases).
- "Insights" stored as records and served via `/insights`.
- Scheduled agent runs decoupled from serving.
- **MCP server adapter** — expose `/datasets/.../records` (and later `/insights`)
  as Model Context Protocol tools, so AI IDEs can query live HKGOV data without
  learning four different upstream APIs. Targets the MCP tool/resource spec.
- **A2A protocol surface** — expose agent capabilities (monitor a dataset, flag an
  anomaly, validate a fact across sources) as agent-to-agent endpoints so other
  agents can subscribe to change/anomaly events and delegate validation tasks.

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
