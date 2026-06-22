# hkgov-rethink

> **Agentic monitoring that surfaces what the HKGOV press room leaves unsaid.**
> A Rust platform that ingests Hong Kong Government open data, runs deterministic
> anomaly detection across sources, and tells you when the official narrative and
> the published data disagree — with evidence you can verify.

## What it actually finds (on real data, captured 2026-06-21)

These are **not synthetic** — they were produced by this project's own detectors
against live HKGOV open data. Full verbatim output + evidence pointers:
[EXAMPLES.md](EXAMPLES.md).

> ⚠️ **`series_jump` (critical):** HIBOR overnight **doubled in one settlement
> window** — +99.3% from 2026-02-13 (1.47) to 2026-02-16 (2.93). HKMA issued no
> press release attributing the move on those dates.

> 📉 **`outlier`:** A sustained sub-1.3% HIBOR cluster across **8 days in March
> 2026** (robust-z down to −4.7), invisible to a single-day spike detector but
> flagged by the MAD outlier scan as a regime, not a blip.

> 🔎 **`cross_source_gap`:** Dates where HKMA issued a press release but published
> no matching statistical data row (or vice versa) — the literal "press room
> leaves it untold" signal, with the specific dates as evidence.

**The determinism guarantee:** every number above is reproducible. Same data in,
same findings out, **no API key required**. The LLM only frames results; detection
is pure Rust. See [Architecture → The determinism guarantee](docs/ARCHITECTURE.md).

## Why this exists

HKGOV publishes rich open data (HKMA monetary stats, data.gov.hk, press releases,
geospatial) but the press releases rarely spell out *why a number moved* or *when
the narrative and the data diverge*. `hkgov-rethink` cross-references those
sources automatically and surfaces the gaps — anomalies, outliers, and
missing-data days — as structured `Insight` records with pointers back to the
source rows so a human can verify the claim.

## Try it in 60 seconds

```bash
./scripts/demo.sh        # one-shot: boots, warms, prints 3 real insights, exits
```

Or run the server and explore:

```bash
cargo run --release -p hkgov-api     # boots on :8080, warms cache from HKGOV
curl 'http://localhost:8080/v1/insights?limit=5'
curl -X POST http://localhost:8080/v1/ask \
  -H 'Content-Type: application/json' \
  -d '{"question":"what is the interbank liquidity doing?"}'
```

Prefer Docker? `docker run ghcr.io/alencheung/hkgov-rethink` (see
[Docker](#docker)). Prefer Python? `pip install hkgov-py` (see
[Python client](#python-client)).

## What makes it different

- **Deterministic-first AI.** Detection is pure Rust; the LLM only frames. The
  whole agent layer — insights, Q&A, alerting — works end to end with zero
  external dependencies. Reproducible in CI.
- **Cross-source, not single-source.** The interesting findings come from
  *comparing* sources (press vs. data, one series vs. another), which a
  per-source dashboard can't do.
- **Evidence, not assertions.** Every `Insight` carries `EvidenceRef`s pointing
  back into the store, so a reader can verify the claim against the source data.
- **Built for scale from day one.** One-way pipeline, cache-first serving, the
  `RecordStore` trait as the scaling contract. Single node → 100k-concurrency
  fleet is a config change, not a refactor ([docs/CAPACITY.md](docs/CAPACITY.md)).

---

## Table of contents

- [What it actually finds](#what-it-actually-finds-on-real-data-captured-2026-06-21) — real captured insights
- [Try it in 60 seconds](#try-it-in-60-seconds)
- [What makes it different](#what-makes-it-different)
- [Architecture](#architecture) — design, data flow, the why behind each crate
- [Features](#features) — what exists today, what's planned
- [Quick start](#quick-start) — boot the API + dashboard
- [Docker](#docker) — zero-friction container path
- [Python client](#python-client) — `pip install hkgov-py`
- [API reference](#api-reference) — every endpoint, with examples
- [The AI agent layer](#the-ai-agent-layer) — how insights are produced
- [Configuration](#configuration) — `config.toml` + env overrides
- [Feature flags](#feature-flags) — opt-in backends
- [Testing](#testing)
- [Repository layout](#repository-layout)
- [Roadmap status](#roadmap-status) — shipped vs. future, milestone by milestone
- [Contributing](#contributing) — good first issues, feature matrix
- [Deeper docs](#deeper-docs) — breadcrumbs for collaborators

---

## Architecture

The platform is a **one-way data pipeline**: upstream → connectors → ingest →
store → api → client. The serving API **never** calls a connector directly; it
only reads from the cache. This is the single most important property — it's
what lets the API scale to fleet-level concurrency without ever saturating the
free HKGOV endpoints it depends on.

```
                         ┌─────────────────────────────────────────────┐
   HKGOV open data       │  connectors (per-source, resilient)         │
   ─────────────────     │  HKMA · data.gov.hk · press · LandsD/CSDI   │
   api.hkma.gov.hk   ──▶ │  + token-bucket rate limit                  │
   api.data.gov.hk   ──▶ │  + three-state circuit breaker              │
   press releases    ──▶ │  + retry w/ exponential backoff             │
   CSDI / LandsD     ──▶ └───────────────────────┬─────────────────────┘
                                                  │ NormalizedRecord[]
                           ┌──────────────────────▼──────────────────────┐
                           │  ingest  (per-dataset refresh supervisor)   │
                           │  one tokio task per dataset, own cadence    │
                           └──────────────────────┬──────────────────────┘
                                                  │ put_dataset()
   ┌──────────────────────────────────────────────▼─────────────────────┐
   │  store  (RecordStore trait — the scaling contract)                 │
   │  ┌─────────────┐   ┌───────────┐   ┌────────────────────────────┐  │
   │  │ moka (def)  │   │  redis    │   │  postgres (cold/historical) │  │
   │  │ in-process  │   │  cluster  │   │  read replicas             │  │
   │  └─────────────┘   └───────────┘   └────────────────────────────┘  │
   └──────────┬───────────────────────────────────────┬──────────────────┘
              │ get_page() / meta() / list()           │
              │                                        │ read-only
   ┌──────────▼──────────────────┐    ┌────────────────▼─────────────────┐
   │  api  (axum, the only       │    │  agent (decoupled scheduler)     │
   │  thing that is deployed)    │    │  reads store → runs detectors →  │
   │  tower stack: timeout,      │    │  LLM frames → writes Insights    │
   │  gzip, CORS, trace,         │    └────────────────┬─────────────────┘
   │  concurrency load-shed      │                     │ upsert()
   │  optional X-API-Key auth    │◀────────────────────┘
   └──────────┬──────────────────┘
              │ HTTP /v1/*
              ▼
          clients + dashboard
```

### Design principles

1. **Cache-first serving.** Hot reads never touch the network — they come from
   an in-process `moka` cache (or a shared Redis tier). This is the single
   biggest concurrency lever. See `crates/store/src/lib.rs:38` (`RecordStore`
   trait) and `crates/store/src/memory.rs`.
2. **One normalized dialect.** HKGOV sources disagree wildly about shapes
   (HKMA wraps in `{header, result:{records}}`, data.gov.hk returns a bare
   array, press is HTML/RSS). Everything collapses onto `NormalizedRecord` so
   the store, API, and agent speak one language. See
   `crates/common/src/model.rs`.
3. **CPU at ingest, not at request.** Normalization, parsing, and typing happen
   when data is *fetched*, not when it's *read*. The request hot path is just
   JSON serialization + gzip.
4. **The agent is a reader, not a blocker.** The AI-agent scheduler reads from
   the warmed store on its own timer and writes `Insight`s back. Serving
   latency is untouched even when the LLM client is slow. See
   `crates/agent/src/scheduler.rs`.
5. **Resilience is per-source, not global.** Each connector is wrapped in its
   own rate limiter + circuit breaker, so one degraded HKGOV endpoint can never
   starve the others. See `crates/connectors/src/resilience.rs` and
   `crates/connectors/src/registry.rs`.
6. **The `RecordStore` trait is the scaling contract.** Going from one node to
   a 100k-concurrency fleet is a *constructor change, not a refactor* — swap
   `MemoryStore` for `RedisStore` or `PgStore`. No other code moves.

For the full rationale (async model, middleware stack, scaling math) see
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and the capacity model in
[docs/CAPACITY.md](docs/CAPACITY.md).

---

## Features

### ✅ Shipped (v1–v5)

**Ingestion & connectors**
- HKMA connector — monetary & market statistics, press releases
  (`crates/connectors/src/hkma.rs`)
- data.gov.hk connector — cross-departmental data via the v2 filter + historical
  archive APIs (`crates/connectors/src/datagovhk.rs`)
- Press connector — HKMA press releases API
  (`crates/connectors/src/press.rs`)
- LandsD/CSDI connector — open geospatial catalog via the data.gov.hk archive
  (`crates/connectors/src/landsd.rs`)
- Per-source **token-bucket rate limiting** + **three-state circuit breakers**
  (closed → open → half-open) wrapping every connector
- Retry with exponential backoff at the HTTP client layer

**Storage**
- `RecordStore` trait with three interchangeable backends:
  - `MemoryStore` — in-process `moka` cache (default, zero-config)
    (`crates/store/src/memory.rs`)
  - `RedisStore` — shared multi-node cache tier (`--features redis`)
    (`crates/store/src/redis_store.rs`)
  - `PgStore` — Postgres persistent/cold tier (`--features pg`)
    (`crates/store/src/pg_store.rs`)

**Ingest pipeline**
- Per-dataset refresh supervisor — one tokio task per dataset, each on its own
  cadence; a slow/failed dataset never blocks others
  (`crates/ingest/src/lib.rs`)

**API** (axum 0.8, the only thing deployed — `crates/api/`)
- Cache-first read endpoints (see [API reference](#api-reference))
- Tower middleware stack: timeout (slowloris protection), gzip, CORS, tracing,
  concurrency load-shedding (`crates/api/src/routes.rs`)
- Optional `X-API-Key` / `?api_key=` auth (`crates/api/src/auth.rs`)
- `/v1` API versioning (health kept at root for LB probes)
- Graceful shutdown (SIGTERM/Ctrl-C) for zero-downtime deploys

**AI agent layer** (`crates/agent/`)
- Pluggable `LlmClient` trait with two implementations:
  - `HeuristicClient` — pure-Rust statistical heuristics, deterministic,
    zero-config (default) (`crates/agent/src/llm.rs`)
  - `HttpLlmClient` — OpenAI-compatible chat-completions client with
    function-calling support (`--features llm`) (`crates/agent/src/llm.rs`)
- Deterministic detectors in `crates/agent/src/analysis.rs`:
  - `series_jump` — flags numeric series that moved beyond a % threshold
    (HIBOR spikes, balance swings, HSI moves)
  - `outlier` — MAD-based robust z-score (skew-resistant outlier flagging) (v6)
  - `seasonality` — autocorrelation at monthly/quarterly periods (v6)
  - `correlation` — Pearson r flagging series decoupling (v6)
  - `cross_source_gap` — flags dates where a press release exists but no
    matching data row does (or vice versa)
- Config-driven scan targets — `[[agent.scan]]` controls which detectors run on
  which datasets; empty list = the v3 defaults (v6)
- Agent tool belt — `list_datasets` / `query_dataset` / `run_detector` behind a
  uniform `Tool` trait with OpenAI-compatible schemas
  (`crates/agent/src/tools.rs`) (v6)
- Agentic investigation loop — LLM-driven multi-step reasoning, bounded by
  `max_steps`; heuristic client opts out via default impl
  (`crates/agent/src/loop_mod.rs`) (v6)
- Natural-language Q&A — `POST /v1/ask`; rich mode runs the agent loop,
  heuristic mode does keyword→dataset matching (`crates/agent/src/qa.rs`) (v6)
- Proactive alerting — `AlertDispatcher` pushes severity-filtered, deduped
  insights to webhook sinks; `WebhookSink` behind `--features alerts`
  (`crates/agent/src/alerts.rs`) (v6)
- Structured `Insight` records with verifiable evidence pointers
  (`crates/agent/src/insight.rs`)
- Decoupled agent supervisor on its own scheduler
  (`crates/agent/src/scheduler.rs`)

**Operations**
- Structured `tracing` logging (plain or JSON for log shippers)
- Optional OpenTelemetry trace export (`--features otel`)
- k6 load-test harness + capacity model (`loadtest/`)

**Public surface**
- Insights dashboard — static HTML reading the live API, incl. a Q&A box
  (`dashboard/index.html`)
- Python API client example (`examples/`)
- CONTRIBUTING guide with data-source verification rules (`CONTRIBUTING.md`)

### 🔮 Planned (from [docs/ROADMAP.md](docs/ROADMAP.md) "Remaining")

- ISD / info.gov.hk HTML scraping + news.gov.hk RSS (press connector v2)
- More `data.gov.hk` resources (each must be probe-verified first)
- Persisting insights to the Postgres tier (currently in-process only)
- Generalize `ToolBelt` / `AgentSupervisor` to `Arc<dyn RecordStore>` so the
  agent works against Redis/Postgres backends
- Deploy manifests (k8s/Helm), OTLP collector config, production hardening
- Auth via OAuth/JWT (current is static API key)

---

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
curl -X POST http://localhost:8080/v1/ask \
  -H 'Content-Type: application/json' \
  -d '{"question":"what is the interbank liquidity?"}'
```

Enable the AI agent (heuristic mode, no API key needed):

```bash
HKGOV_AGENT__ENABLED=true cargo run -p hkgov-api
```

Then open the dashboard in a browser — it's served by the API itself, so
there's nothing to open as a file:

```
http://localhost:8080/dashboard      # the live insights dashboard
```

(The dashboard is also available as the static file
[dashboard/index.html](dashboard/index.html) if you prefer; point its base-URL
field at `http://localhost:8080`.) You'll see live source health + AI-generated
insights.

---

## Docker

A container image is the fastest way to try the project without a Rust toolchain.

```bash
# Build locally
docker build -t hkgov-rethink .

# Run with the agent enabled (heuristic mode, no API key needed)
docker run --rm -p 8080:8080 -e HKGOV_AGENT__ENABLED=true hkgov-rethink

# Then:
curl 'http://localhost:8080/v1/insights?limit=5'
```

The image is multi-stage and distroless-slim; the final image is ~30MB. See
[`Dockerfile`](Dockerfile). (CI publishes to `ghcr.io/alencheung/hkgov-rethink`
on tags — see [`.github/workflows/release.yml`](.github/workflows/release.yml).)

---

## Python client

For data work without writing Rust, install the typed Python client:

```bash
pip install hkgov-py
```

```python
from hkgov import HkGov

client = HkGov("http://localhost:8080")           # API key optional
for s in client.sources():
    print(s.source, s.dataset, s.record_count)

answer = client.ask("what is the interbank liquidity doing?")
print(answer.text, f"{answer.confidence:.0%}")
```

Source and packaging live in [`python/`](python/). The client is a thin wrapper
over the HTTP API, so anything the API can do, Python can do.

---

## API reference

All data endpoints are under `/v1` (configurable via `api.api_prefix`).
`/health` stays at root for LB/k8s probes.

| Method | Path | Description |
|---|---|---|
| `GET` | `/` | Service name, version, and an endpoint directory |
| `GET` | `/health` | Liveness probe (`{status, version}`) |
| `GET` | `/dashboard` | The static insights dashboard (served by the API; exempt from API-key auth) |
| `GET` | `/v1/health/sources` | Per-source circuit-breaker state (closed/open/half-open) |
| `GET` | `/v1/sources?category=&tag=&cadence=&source=&q=` | Datasets, filterable by domain/category, tags, cadence, and free text (v8) |
| `GET` | `/v1/categories` | The domain taxonomy with dataset counts — the browse entry point (v8) |
| `GET` | `/v1/datasets/{source}/{dataset}` | Metadata for one dataset |
| `GET` | `/v1/datasets/{source}/{dataset}/records?offset=&limit=` | Paginated records from cache |
| `GET` | `/v1/insights?limit=` | AI-agent generated insights with evidence |
| `GET` | `/v1/alerts?limit=` | Proactive alert dispatch log (v6) |
| `POST` | `/v1/ask` | Natural-language Q&A over the data (v6) |

`{source}` is one of `hkma`, `datagovhk`, `press`, `landsd` — see
`crates/common/src/model.rs` (`DataSource::parse`).

**Dataset categories** (v8): every dataset declares exactly one of
`monetary`, `fiscal`, `property`, `trade`, `population`, `livability`,
`government`, `other` — plus free-form `tags` and a `cadence`. Filter with
`?category=monetary&tag=hibor&cadence=daily&q=interbank`. The `tag` filter
accepts a single tag (`?tag=hibor`), repeated tags
(`?tag=hibor&tag=liquidity`, matches ANY), or comma-separated
(`?tag=hibor,liquidity`). See [EXAMPLES.md](EXAMPLES.md) and
`GET /v1/categories` for the browse entry point.

When `api.api_key` is set, every `/v1` request must carry it via the
`X-API-Key` header or `?api_key=` query param.

---

## The AI agent layer

The agent is what makes this more than "a chat over data". It runs
**deterministic detection** first, then asks an LLM only to *frame* the finding
into natural language — so insights work end to end with zero external
dependencies.

v6 made it genuinely agentic: the LLM can now *choose* which deterministic tool
to call and reason multi-step to an answer (via `POST /v1/ask`), and insights
can be pushed proactively to webhooks. The defining invariant is unchanged —
**the LLM never performs detection**, only the pure-Rust detectors do.

```
warmed NormalizedRecords in the store
        │
        ▼
 ┌────────────────────────────────────────────────────┐
 │ analysis.rs — deterministic detectors (pure Rust)  │
 │  series_jump · outlier · seasonality · correlation │  each Finding carries
 │  cross_source_gap                                  │  EvidenceRef[] pointers
 └───────────────────────┬────────────────────────────┘  back into the store
                         │
        ┌────────────────▼──────────────────┐
        │ tools.rs — ToolBelt               │  list_datasets · query_dataset
        │  (uniform Tool trait, OpenAI      │  · run_detector
        │   schemas)                        │
        └────────────────┬──────────────────┘
                         │ called by
        ┌────────────────▼──────────────────────────────┐
        │ two callers, same substrate                  │
        │  · scheduler run_pass (periodic scan)         │
        │  · loop_mod run_agent_loop (LLM-driven Q&A)   │  bounded by max_steps
        └────────────────┬──────────────────────────────┘
                         │
        ┌────────────────▼──────────────────┐    ┌──────────────────────┐
        │ LlmClient (crates/agent/llm.rs)   │    │ qa.rs heuristic      │
        │  HeuristicClient (default, no key)│    │  keyword→dataset     │
        │  HttpLlmClient   (--features llm) │    │  fallback for /ask   │
        └────────────────┬──────────────────┘    └──────────────────────┘
                         │ into_insight()
        ┌────────────────▼──────────────────┐
        │ InsightStore ──▶ GET /v1/insights │
        │  severity ≥ threshold ──▶ AlertDispatcher ──▶ webhooks (alerts feat)
        │                              │                + GET /v1/alerts
        └──────────────────────────────┘
```

Key property: **detection stays deterministic regardless of provider.** The
heuristic client surfaces the same structured findings an LLM would, so the
quality bar for insights doesn't depend on whether an API key is configured.
See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) §"The determinism guarantee".

---

## Configuration

[`config.toml`](config.toml) with env overrides using a `HKGOV_` prefix and
`__` separator (via `figment`). Later wins: defaults < `config.toml` < env.

```bash
HKGOV_API__BIND=0.0.0.0:9090              # bind address
HKGOV_API__API_KEY=secret                 # enable API key auth
HKGOV_API__MAX_CONCURRENCY=100000         # tower load-shedding ceiling
HKGOV_STORE__BACKEND=redis                # memory | redis | pg
HKGOV_STORE__REDIS_URL=redis://...        # only used when backend=redis
HKGOV_AGENT__ENABLED=true                 # turn on the AI agent
HKGOV_AGENT__LLM_BASE_URL=https://...     # empty = heuristic mode
HKGOV_AGENT__LLM_API_KEY=sk-...           # for the HTTP LLM client
HKGOV_UPSTREAM__HKMA_RATE_PER_SEC=5       # politeness budget (don't raise)
HKGOV_ALERTS__ENABLED=true                # turn on proactive alerting
HKGOV_ALERTS__MIN_SEVERITY=warning        # info | warning | critical
HKGOV_ALERTS__WEBHOOK_TOKEN=secret        # Bearer token for webhook POSTs
```

Scan targets (which detectors run on which datasets) are configured via
`[[agent.scan]]` blocks — see `config.toml` for the full matrix. An empty
`scan` list runs the built-in defaults (the v3 targets), so out-of-the-box
behavior is unchanged.

All knobs are documented inline in `crates/common/src/config.rs`. The
scaling-relevant fields (worker threads, connection pool, cache size) are all
exposed there — nothing is hardcoded.

---

## Feature flags

Optional backends/integrations, off by default so the default build needs no
external services:

```bash
cargo build -p hkgov-store --features redis    # Redis cache tier
cargo build -p hkgov-store --features pg       # Postgres persistent tier
cargo build -p hkgov-common --features otel    # OpenTelemetry export
cargo build -p hkgov-api --features llm        # HTTP LLM client for insights + /ask
cargo build -p hkgov-api --features alerts     # webhook sink for proactive alerting
```

Live HKMA integration tests are behind:

```bash
cargo test -p hkgov-connectors --features live
```


See [CONTRIBUTING.md](CONTRIBUTING.md) for the full feature matrix.

---

## Testing

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

---

## Repository layout

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

---

## Roadmap status

| Milestone | Status | Highlights |
|---|---|---|
| **v1** Foundation | ✅ | HKMA connector, cache-first store, axum API |
| **v2** Sources + resilience | ✅ | data.gov.hk + press + LandsD, rate limiting, circuit breakers, Redis store |
| **v3** AI-agent layer | ✅ | anomaly detection, cross-source gaps, `/insights`, heuristic + LLM clients |
| **v4** Scale & hardening | ✅ | Postgres store, API auth + `/v1` versioning, OpenTelemetry, k6 harness |
| **v5** Public surface | ✅ | Insights dashboard, examples, CONTRIBUTING |
| **v6** Intelligence & agentic | ✅ | richer detectors, tool belt, agent loop, `/ask` NL Q&A, proactive alerting |

Full detail, including the remaining/future work, in
[docs/ROADMAP.md](docs/ROADMAP.md).

---

## Contributing

Contributions are welcome — the project is structured to make first PRs easy.
Start with [CONTRIBUTING.md](CONTRIBUTING.md) (feature matrix, architecture
invariants, step-by-step guides for adding a connector or a detector).

**Good first issues** are labeled [`good first issue`](https://github.com/alencheung/hkgov-rethink/labels/good%20first%20issue)
on GitHub — each is bounded, self-contained, and points at the file to change.
A few representative ones:

- Add a `detect_trend_break` detector (follows the existing detector pattern)
- Wire `store.backend` config selection into `main.rs` (currently dead config)
- Add a `news.gov.hk` RSS connector (press connector v2)
- Promote `seasonality`/`correlation` from experimental once they catch a real finding
- Frontend: a richer dashboard with a chat UI for `/ask`

See [CHANGELOG.md](CHANGELOG.md) for what shipped in each milestone. Please open
an issue before large changes so we can align on direction. By contributing you
agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

---

## Deeper docs

Breadcrumbs for collaborators — read these in roughly this order to get fully
oriented:

| If you want to understand… | read this |
|---|---|
| The end-to-end design and the "why" behind each crate | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| The path from one node to 100k concurrent users | [docs/CAPACITY.md](docs/CAPACITY.md) |
| What's done vs. what's next | [docs/ROADMAP.md](docs/ROADMAP.md) |
| Which HKGOV endpoints we hit and why (verified live) | [docs/DATA_SOURCES.md](docs/DATA_SOURCES.md) |
| The normalized data model every source maps onto | `crates/common/src/model.rs` |
| Every runtime knob | `crates/common/src/config.rs` |
| The scaling contract (how to add a new store backend) | `crates/store/src/lib.rs` (`RecordStore` trait) |
| How to add a new data source | `crates/connectors/src/lib.rs` (`Connector` trait) + `crates/connectors/src/registry.rs` |
| How insights are detected and framed | `crates/agent/src/analysis.rs` + `crates/agent/src/llm.rs` |
| The resilience layer (rate limiting + circuit breaking) | `crates/connectors/src/resilience.rs` |
| How to contribute + the feature matrix + source-verification rules | [CONTRIBUTING.md](CONTRIBUTING.md) |

## Data sources

All public HKGOV open data — see [docs/DATA_SOURCES.md](docs/DATA_SOURCES.md)
for the verified endpoint table. The government-only
`api.portal.hkmapservice.gov.hk` is intentionally excluded; we use the open
LandsD/CSDI endpoints on data.gov.hk instead.

## License

MIT — see [LICENSE](LICENSE).
