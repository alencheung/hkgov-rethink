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

## AI-agent layer (ROADMAP v2–v3)

The agent layer sits *on top of* the store, not inside connectors:

- It reads normalized `NormalizedRecord`s (one dialect, regardless of source).
- It cross-references sources (e.g. HKMA monetary stats vs. ISD press releases)
  to detect divergences — "the press release says X, the data says Y".
- It runs on its own scheduler so it never blocks serving.
- Outputs (insights, alerts) are themselves stored as records and served via
  the same API, so insights get the same concurrency guarantees as raw data.

Where it plugs in: a new `crates/agent` crate depending on `store` + a pluggable
LLM client. No change to `api` routes beyond adding `/insights`.

## Configuration & operations

- `config.toml` + `HKGOV_` env overrides (see `crates/common/src/config.rs`).
- Structured `tracing`; switch to JSON for log shippers via `log.format=json`.
- Graceful shutdown wired (SIGTERM/Ctrl-C) so deploys drain in flight.
