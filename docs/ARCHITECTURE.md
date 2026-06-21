# Architecture

This document explains the design and — specifically — how the v1 foundation is
shaped to reach the project's stated goals: **100k concurrent users** and an
**AI-agent analysis layer** that surfaces untold insights.

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

| Stage | Change | Why |
|---|---|---|
| v1 (now) | in-process `moka` cache, one node | proves the contract |
| v2 | shared **Redis** cluster behind `RecordStore` trait | cache hit across nodes |
| v3 | stateless API behind a **LB**, N replicas | horizontal scale |
| v4 | **Postgres** read replicas for cold/historical reads | unbounded dataset size |
| v5 | **load-test harness** (k6/oha) + capacity model | validate the 100k number |

The `RecordStore` trait in `crates/store` is the contract each tier satisfies —
swapping the backing store is a constructor change, not a refactor.

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

## Configuration & operations

- `config.toml` + `HKGOV_` env overrides (see `crates/common/src/config.rs`).
- Structured `tracing`; switch to JSON for log shippers via `log.format=json`.
- Graceful shutdown wired (SIGTERM/Ctrl-C) so deploys drain in flight.
