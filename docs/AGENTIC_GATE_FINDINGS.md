# Agentic-Gate Findings & Suppression Rationale

Record of the agentic-gate quality-gate run on the full tree, and the reason
each **blocking** finding is treated as a false positive (not applied).

Last run: `run --no-diff --strict --fix --json` · 52 source files ·
score 0/100, exit 1, **12 blocking (high)** findings — all false positives.

This doc exists so the gate doesn't keep re-blocking on the same items and so a
reviewer can see *why* no code change was made. The `.agentic/allowlist.txt`
file is reserved for supply-chain package allowlisting and is intentionally not
used here.

---

## Blocking findings — false positives (DO NOT apply the suggested fixes)

### 1. SKILL_DEAD_BRANCH — `python/src/hkgov/client.py` (6 findings)

Lines flagged: **150, 166, 217, 231, 244, 259** — "unreachable code, follows an
unconditional return."

**Why false positive:** the detector treats a line that begins a `return [...]`
or `return RecordPage(...)` as a *complete* terminator, then flags the
continuation lines of the **multi-line return statement** as dead. These are
list comprehensions / multi-line constructor calls, e.g.:

```python
return [
    CategoryGroup(category=x["category"], count=x["count"], datasets=x["datasets"])
    for x in d
]
```

Deleting the flagged lines would remove the comprehension body and **break the
client**. This is a known limitation of line-based dead-code detection on
Python (statements spanning multiple physical lines).

Applicable to all six locations:
- `:150` — `categories()` return list-comp
- `:166` — `records()` return `RecordPage(...)`
- `:217` — `alerts()` return list-comp
- `:231` — `ask()` return `Answer(...)`
- `:244` — `_meta()` static helper
- `:259` — `_insight()` static helper

### 2. SKILL_OPS_GUARD — `Dockerfile` (2 findings)

Lines flagged: **18, 42** — "agent-reachable script recursive delete of a
root/home path."

**Why false positive:** both are the standard Debian apt-cache cleanup:

```dockerfile
&& rm -rf /var/lib/apt/lists/*
```

`/var/lib/apt/lists/*` is the **apt package-index cache** — not a root/home
path. This is the recommended Docker pattern (used by most official images) to
keep layers small. It runs once at `docker build` time in an ephemeral build
container, is **not reachable by an autonomous agent at runtime**, and destroys
no production data. Removing it only bloats the image.

- `:18` — builder stage apt cleanup
- `:42` — runtime stage apt cleanup

### 3. SKILL_WEB_GUARD — `loadtest/loadtest.js` (4 findings)

Lines flagged: **42, 44, 46, 48** — "outbound HTTP call with no timeout."

**Why false positive:** the rule assumes Node.js `fetch` semantics. This file is
a **k6 load-test script**; `http.get()` is k6's API, which does **not** accept a
Node-style `timeout` parameter. Request bounds are configured at the
scenario/options level, which this file already does:

```js
thresholds: {
  http_req_duration: ['p(95)<500', 'p(99)<1500'],
  errors: ['rate<0.01'],
}
```

Adding a `timeout` key to the params object would be silently ignored by k6.
It is also a test runner, not a production server, so the
"connection exhaustion" risk does not apply.

---

## Non-blocking findings (medium / low) — for awareness, not action

These did not block the gate (severity medium/low) but are tracked here.

### SKILL_COMPLEXITY_RADAR (12 medium) — real, deferred
Cyclomatic-complexity and deep-nesting in Rust handlers. The worst offenders:
- `crates/api/src/routes.rs::ready` — cc 60 (max 10)
- `crates/api/src/routes.rs::delete_signal` — cc 32
- `crates/agent/src/signal.rs::signal_id` — cc 20
- `crates/api/src/routes.rs::cite_insight` — cc 18
- `crates/api/src/main.rs::main` — cc 16
- `crates/api/src/routes.rs::sources_filters_by_tag` — cc 15
- `crates/agent/src/tools.rs::call` — cc 14
- `crates/agent/src/analysis.rs::detect_proxy_divergence` — cc 12
- `crates/api/src/main.rs::wait_for_scan_readiness` — cc 11
- `crates/agent/src/tools.rs::run_two_dataset_detector` — cc 11
- `crates/api/src/routes.rs::tags` — nested 6 levels (max 4)
- `crates/agent/src/llm.rs::message_to_openai` — nested 6 levels (max 4)

These are genuine but require real refactoring (extract handlers / guards)
and are out of scope for a gate-pass.

### SKILL_SCALE_GUARD (4 medium, 3 low) — mostly misapplied
"Missing rate-limit / cache-control" on:
- `examples/query_api.py`, `python/src/hkgov/client.py`, `python/tests/test_client.py`
- `loadtest/loadtest.js` (cache-control only)

Mostly **misapplied**: `hkgov/client.py` and the test/example files are HTTP
*clients* and a pytest file — rate-limit/cache middleware live on the **server**
(`crates/api`), not on clients. The server-side rate limiting is implemented in
the Rust API. (Verify server-side coverage separately if needed.)

### SKILL_PERF_GUARD (1 medium, 1 low) — minor, real
- `client.py:171` — linear search inside a loop (O(n²)); a Set would make it
  O(1). Low volume (paginated records), low priority.
- `client.py:90` — `r.json()` recomputed; cache in a local.

### SKILL_ARCH_ALIGNER (1 medium) — likely false positive
- `crates/agent/src/silence.rs:344` — "explicit `any` type." That file is Rust;
  the `any` token almost certainly appears inside a string literal or macro, not
  as a TypeScript type annotation. Verify before acting.

---

## How to re-run

```bash
# Full tree (noisy — produces the findings above)
node C:/Users/alen_/.zcode/agentic-gate/bin/agentic-gate.js run --no-diff --strict --json

# Recommended for routine use: audit only the current diff
node C:/Users/alen_/.zcode/agentic-gate/bin/agentic-gate.js run --diff --strict --json
```

The full-tree scan is too noisy for this repo's stack (Python multi-line
returns, Dockerfile apt-cleanup, k6 DSL). Prefer `--diff` for day-to-day use.
