# Capacity & scaling

How `hkgov-rethink` is *designed* to move from one node toward fleet-level
concurrency — and an honest accounting of which parts of that path are built
versus wired in and verified.

> **Status at a glance:** the architecture is designed for horizontal scaling
> (the `RecordStore` trait is the scaling contract), but the single-node
> **MemoryStore** (`moka`) is the only backend actually wired into the running
> binary today. The Redis and Postgres backends are *implemented* behind feature
> flags but **not wired in**, and the 100k figure below is a **design target**,
> not a verified measurement. The single-node ceiling has so far been measured
> only at ~500 VUs in development via the k6 smoke harness.

## Single-node ceiling (measure, don't guess)

The hot path (cache-hit reads) is what matters for concurrency. Measure it:

```bash
# 1. Boot the server
cargo run --release -p hkgov-api

# 2. Warm the cache (the ingest supervisor does this on startup)
sleep 30

# 3. Run the harness — ramps to 500 VUs, holds, ramps down
k6 run loadtest/loadtest.js
```

> **Note:** the k6 harness defaults to **500 VUs**. That default is a **smoke
> test**, not a ceiling test — it proves the server boots and serves under a
> modest load, not the single-node maximum. It is also **not run in CI**, so
> regression of the ceiling is not currently caught automatically. To actually
> probe the ceiling, raise `K6_STAGES` (below) and run it by hand.

Tune the target with `K6_STAGES`:

```bash
# Push toward a higher ceiling
K6_STAGES='[{"duration":"1m","target":2000},{"duration":"3m","target":2000},{"duration":"30s","target":0}]' \
  k6 run loadtest/loadtest.js
```

With API auth enabled, pass the key: `K6_API_KEY=... k6 run ...`.

## Expected shape (single node)

| Metric | Target | Why |
|---|---|---|
| p95 latency (cache hit) | < 100ms | axum + moka, no network |
| p99 latency (cache hit) | < 500ms | tail under GC / scheduling |
| error rate | < 1% | timeouts/slowloris shedding |
| requests/sec ceiling | ~node-dependent | bounded by `api.max_concurrency` |

A single modern node is *expected* to sustain tens of thousands of concurrent
keep-alive connections at low QPS, and thousands of cached reads/sec — but this
has **not been validated** beyond the 500-VU smoke run. That is the foundation;
100k is a fleet number.

## Scaling path (design target: 100k)

The `RecordStore` trait is the scaling contract: each tier satisfies the same
interface, so swapping the backing store is intended to be a constructor
change, not a refactor. The table below is honest about what is built, what is
wired, and what is verified.

| Stage | Change | Concurrency unlocked | Status |
|---|---|---|---|
| **v1** | in-process `moka` cache, 1 node | single-node baseline | **shipped + wired + tested** — the default and only backend the binary actually runs |
| **v2** | shared **Redis** cluster (`--features redis`) | cache hits across nodes | **implemented, NOT wired** — `RedisStore` exists behind the feature flag, but `store.backend=redis` is currently dead config in `main.rs`. Also has an architectural issue: it serializes the whole dataset as a single blob rather than keying per-record. |
| **v3** | stateless API behind a **load balancer**, N replicas | linear with N | **not implemented** — no LB tier, deploy manifests, or replica story yet. This is the stage that actually unlocks fleet-level concurrency. |
| **v4** | **Postgres** read replicas for historical reads (`--features pg`) | unbounded dataset size | **implemented, NOT wired** — `PgStore` exists behind the feature flag, but is not selected at runtime. Also has an architectural issue: a single `Mutex<Client>` that would serialize access under load. |
| **v5** | re-run `k6` against the LB front door | validate the 100k number | **harness exists, not a validation** — the k6 harness runs, but defaults to 500 VUs and is not in CI; the 100k target has never been measured against an LB tier (because v3 doesn't exist yet). |

At each stage the intended code change is only configuration — the
`RecordStore` trait absorbs the backing-store swap, leaving connectors, ingest,
agent, and routes unchanged. But for v2 and v4 that swap is not yet hooked up
in `main.rs`, so the feature flags produce a compiled backend that nothing
instantiates.

## What "100k" actually means

The 100k-concurrent-user figure is a **design target**, not a verified number.
Concretely:

- **Designed for:** yes. The cache-first, one-way, trait-bordered architecture
  is shaped so that an LB tier (v3) in front of N stateless replicas can scale
  concurrency roughly linearly with N.
- **Verified at:** no. The only load test that runs is a 500-VU smoke test on a
  single node, by hand, not in CI. Reaching a real 100k measurement requires
  the v3 LB tier (not built) and a k6 run scaled into the tens of thousands of
  VUs against it.

So: **single-node ceiling measured at ~500 VUs in development; the 100k target
requires the LB tier (v3+) which is not yet implemented.**

## Where the limits actually are

- **Upstream (HKGOV endpoints):** not a serving bottleneck — the cache fronts
  them. Politeness budgets (`hkma_rate_per_sec`, circuit breakers) keep us from
  being blocked by HKGOV.
- **Memory:** the moka `max_entries` cap bounds resident memory. Size it to RAM.
- **CPU:** normalization happens at ingest time, not request time, so the hot
  path is JSON serialization + gzip — cheap.
- **Network:** with gzip on, payloads are small. Connection count, not
  bandwidth, is the real ceiling; that's what the (not-yet-built) LB tier
  addresses.

## When you'll need v3+

If single-node p95 stays low but the **connection count** saturates (file
descriptors, ephemeral ports), you've hit the single-node ceiling and need the
LB tier. If p95 rises with QPS on cache hits, raise `max_concurrency` and check
for blocking — there should be none in the read path.
