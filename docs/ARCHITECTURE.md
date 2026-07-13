# Architecture

This document explains the design and — specifically — how the v1 foundation is
shaped to reach the project's stated goals: a **100k-concurrent-user fleet** (a
design target, not a verified measurement — see the status note under "Scaling
path" below and [docs/CAPACITY.md](CAPACITY.md)) and an **AI-agent analysis
layer** that surfaces untold insights.

## Crate graph

```
        ┌──────────┐
        │  common  │   config, normalized model, errors, telemetry
        └────┬─────┘
   ┌─────────┼──────────┐
   ▼         ▼          ▼
connectors  store     (used by all)
   │         ▲
   ▼         │
 ingest ─────┘        per-dataset refresh scheduler
   │
   ▼
 api                 axum binary (the only thing deployed)
```

Data flows one way: **upstream → connectors → ingest → store → api → client**.
The API never calls a connector directly; it only reads from the store. This is
what lets us hit high concurrency without saturating HKGOV upstreams.

Seven connectors are registered in `crates/connectors/src/registry.rs`:

| Source | File | What it fetches |
|---|---|---|
| HKMA | `hkma.rs` (+ `hkma_datasets.rs`) | 151 datasets — the full public HKMA Open API catalog |
| data.gov.hk | `datagovhk.rs` | 33 probe-verified PSI resources via the v2 filter API |
| press | `press.rs` | HKMA press releases API |
| LandsD/CSDI | `landsd.rs` | Open geospatial catalog via the data.gov.hk archive |
| Immigration | `immigration.rs` | Daily passenger-traffic CSV (border crossings) |
| RVD | `rvd.rs` | Monthly property price/rental index CSVs |
| Land Registry | `landregistry.rs` | Monthly property transaction JSON files |

See [docs/DATA_SOURCES.md](DATA_SOURCES.md) for the verified endpoint table.

## Why this targets 100k concurrency

The target is fleet-level, not single-node. The design is honest about that:

1. **Async everywhere (tokio + hyper + axum).** Every handler is non-blocking;
   idle connections cost ~kilobytes, so a single node can hold hundreds of
   thousands of keep-alive sockets. The CPU-bound work is normalization at
   ingest time, not at request time.
2. **Cache-first serving.** Hot reads in v1 are served from an in-process
   `moka` cache — they never touch the network. This is the single biggest
   concurrency lever.
3. **Tower middleware stack.** `TimeoutLayer` (slowloris protection),
   `CompressionLayer`, `CorsLayer`, `TraceLayer`. The `api.max_concurrency`
   setting is the knob for load-shedding under flood.
4. **Bounded upstream pressure.** Connectors retry with exponential backoff and
   cap concurrency; the platform stays available even if an HKGOV endpoint
   degrades.

## Scaling path (single node → 100k)

> **Status note (2026-07):** the 100k figure is a **design target, not a
> verified measurement**. v1 (in-process `moka`) is the only stage actually
> wired into the running binary and is the production backend. v2 (Redis) and
> v4 (Postgres) are **implemented** behind their feature flags but **not wired
> in** — `build_store` in `main.rs` reads `store.backend` but only `memory`
> is instantiated; `redis`/`pg` produce a loud startup error. Each also has a
> known architectural issue to address before wiring (`RedisStore`:
> whole-dataset blob; `PgStore`: single `Mutex<Client>`). v3 (the LB tier that
> actually unlocks fleet-level concurrency) is **not implemented**. The only
> load test run so far is a 500-VU k6 smoke test by hand, not in CI. See
> [docs/CAPACITY.md](CAPACITY.md) for the honest per-stage breakdown.

| Stage | Change | Why | Status |
|---|---|---|---|
| v1 (now) | in-process `moka` cache, one node | proves the contract | shipped + wired + tested |
| v2 | shared **Redis** cluster behind `RecordStore` trait | cache hit across nodes | implemented, NOT wired (config read but bails; blob issue) |
| v3 | stateless API behind a **LB**, N replicas | horizontal scale | not implemented |
| v4 | **Postgres** read replicas for cold/historical reads | unbounded dataset size | implemented, NOT wired (config read but bails; Mutex issue) |
| v5 | **load-test harness** (k6/oha) + capacity model | validate the 100k number | harness exists, defaults to 500 VUs, not in CI |

The `RecordStore` trait in `crates/store` is the contract each tier satisfies —
swapping the backing store is intended to be a constructor change, not a
refactor. But until v2/v4 are wired into `main.rs` and v3 is built, the 100k
target remains unverified.

## AI-agent layer (ROADMAP v3 foundation, v6 made it agentic)

The agent layer sits *on top of* the store, not inside connectors:

- It reads normalized `NormalizedRecord`s (one dialect, regardless of source).
- It cross-references sources (e.g. HKMA monetary stats vs. ISD press releases)
  to detect divergences — "the press release says X, the data says Y".
- It runs on its own scheduler so it never blocks serving.
- Outputs (insights, alerts) are themselves stored as records and served via
  the same API, so insights get the same concurrency guarantees as raw data.

Where it plugs in: the `crates/agent` crate depending on `store` + a pluggable
LLM client. v3 added `/insights`; v6 added the tool belt, the agent loop,
`POST /v1/ask`, and proactive alerting.

### The determinism guarantee (v6)

The defining property of the agent layer is: **the LLM never performs
detection**. It only *selects* which deterministic tool to call and *frames*
the result. Every finding originates in `crates/agent/src/analysis.rs` (pure
Rust). This means:

- The heuristic baseline (`HeuristicClient`, no API key) produces the same
  structured findings an LLM would — insights, Q&A, and alerting all work end
  to end with zero external dependencies.
- The LLM adds capability (richer framing, autonomous investigation, NL
  answers) on top of a reproducible core. A re-run with the same inputs and the
  heuristic client reproduces the same insights deterministically.

### The four layers of v6 intelligence

```
analysis.rs ── deterministic detectors (pure Rust, no deps)
   │            series_jump · outlier · seasonality · correlation · cross_source_gap
   ▼
tools.rs ──── ToolBelt: list_datasets · query_dataset · run_detector
   │            (uniform Tool trait, OpenAI-compatible schemas)
   ▼
loop_mod.rs ─ run_agent_loop: LLM proposes tool call → execute → reason → finalize
   │            (bounded by max_steps; heuristic client opts out via default impl)
   ▼
qa.rs ─────── heuristic_answer: keyword→dataset fallback when no LLM configured
```

- **`analysis.rs`** — the detectors. Adding one is a `pub fn -> Vec<Finding>`
  plus a dispatch arm in `scheduler.rs` and `tools.rs`.
- **`tools.rs`** — the substrate both the periodic scan and the agent loop call
  through. Wraps store reads + detector dispatch behind a uniform interface.
- **`loop_mod.rs`** — the provider-agnostic agent loop. `LlmClient::step` has a
  default impl that finalizes immediately, so heuristic clients skip the loop.
- **`qa.rs`** — keeps `POST /v1/ask` useful without an LLM key.

### Proactive alerting (v6)

When the supervisor produces new insights, an `AlertDispatcher` decides which
are worth pushing (severity ≥ `[alerts] min_severity`, deduped by insight id)
and fans them out to `AlertSink`s. The built-in `WebhookSink` (POST JSON to a
URL, bounded retry) is behind the `alerts` feature. The dispatch log is served
via `GET /v1/alerts` for ops visibility. Sinks that fail are logged, not fatal
— one bad webhook can't block the others.

### Product layer (v7–v8)

The v7 and v8 milestones added product-layer modules on top of the detector
substrate, all preserving the determinism guarantee:

| Module | Feature | What it does |
|---|---|---|
| `silence.rs` | P-100 | **Silence Index** — a 0–100 opacity score rolled up from `cross_source_gap` + unattributed `series_jump` + missing-data days. HKMA-scoped v1. |
| `unprecedentedness.rs` | P-103 | **Unprecedentedness Score** — percentile rank, median±k·MAD band, 1-in-N return period, "last exceeded" comparator. |
| `cite.rs` | P-101 | **Cite-It** — permalink + citation strings (BibTeX/RIS/APA/Chicago/MD) + a SHA-256 reproducibility manifest over the evidence. |
| `insight.rs` | P-104 | **Insight Lifeline** — evolution-aware `upsert`; detects content changes, archives prior versions, exposes `first_seen`/`version`/`evolution`. |
| `signal.rs` | P-102 | **Signal Subscriptions** — user-owned `ScanTarget` + channel routing; preview runs the real detector so "preview IS what will fire." |
| `investigation.rs` | P-105 | **Drill-In Investigations** — saved, resumable, shareable case files from any insight. |
| `identity.rs` | P-108 | **Identity Tier** — email + magic-link → bearer session; the principal for per-user state. |
| `bilingual.rs` | P-106 | **Bilingual Surface** — deterministic zh-HK insight summaries via `frame_zh_hk`, keyed by detector kind. |
| `brief.rs` | — | Ranked daily brief; experimental findings discounted ×0.7. |
| `persist.rs` | — | File-based snapshot persistence for the v8 in-process stores (stopgap until Postgres tier; makes user state survive a graceful restart). |

All product-layer stores (`InsightStore`, `SignalStore`, `InvestigationStore`,
`UserStore`, `FeedbackStore`) are `Arc<RwLock<BTreeMap>>` — volatile by
default. `persist.rs` provides atomic snapshot-to-file + restore-on-boot so a
graceful restart doesn't wipe signals, investigations, identity, and sessions.
Full Postgres persistence remains the G2 roadmap workstream.

## Configuration & operations

- `config.toml` + `HKGOV_` env overrides (see `crates/common/src/config.rs`).
- Structured `tracing`; switch to JSON for log shippers via `log.format=json`.
- Graceful shutdown wired (SIGTERM/Ctrl-C) so deploys drain in flight.
- **Per-IP rate limiting** (`crates/api/src/ratelimit.rs`) — a `governor`-
  backed token bucket attached as an axum `from_fn` middleware. `api.rate_per_sec`
  is now wired (was dead config); 0 = unlimited. Driven directly because
  `tower-governor` doesn't yet support axum 0.8's body type.
- **Constant-time secret comparison** (`crates/api/src/secrets.rs`) — the API-key
  guard routes both the length check and the byte compare through `subtle`'s
  `ConstantTimeEq`, closing a timing side-channel on the auth path.
- **Routes module** (`crates/api/src/routes/`) — the router is split across
  `mod.rs` (core data/health/dashboard routes), `auth_routes.rs` (identity),
  `signals.rs` (signal subscriptions), and `investigations.rs` (case files).
  The full API table is in the [README's API reference section](../README.md#api-reference).
