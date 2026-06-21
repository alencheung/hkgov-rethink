# Capacity & scaling

How `hkgov-rethink` moves from one node to **100k concurrent users**, with the
load-test harness that validates each step.

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

A single modern node typically sustains **tens of thousands** of concurrent
keep-alive connections at low QPS, and thousands of cached reads/sec. That is
the foundation; 100k is a fleet number.

## Scaling path to 100k

| Stage | Change | Concurrency unlocked |
|---|---|---|
| **v1** (now) | in-process `moka` cache, 1 node | ~10k connections |
| **v2** | shared **Redis** cluster (`--features redis`, `store.backend=redis`) | cache hits across nodes |
| **v3** | stateless API behind a **load balancer**, N replicas | linear with N |
| **v4** | **Postgres** read replicas for historical reads (`--features pg`) | unbounded dataset size |
| **v5** | re-run `k6` against the LB front door | validate the 100k number |

At each stage the only code change is configuration: the `RecordStore` trait
absorbs the backing-store swap. The connectors, ingest, agent, and routes are
unchanged.

## Where the limits actually are

- **Upstream (HKGOV endpoints):** not a serving bottleneck — the cache fronts
  them. Politeness budgets (`hkma_rate_per_sec`, circuit breakers) keep us from
  being blocked by HKGOV.
- **Memory:** the moka `max_entries` cap bounds resident memory. Size it to RAM.
- **CPU:** normalization happens at ingest time, not request time, so the hot
  path is JSON serialization + gzip — cheap.
- **Network:** with gzip on, payloads are small. Connection count, not
  bandwidth, is the real ceiling; that's what the LB tier addresses.

## When you'll need v3+

If single-node p95 stays low but the **connection count** saturates (file
descriptors, ephemeral ports), you've hit the single-node ceiling and need the
LB tier. If p95 rises with QPS on cache hits, raise `max_concurrency` and check
for blocking — there should be none in the read path.
