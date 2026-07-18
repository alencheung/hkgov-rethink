# DEFECTS.md — Defect log for hkgov-rethink

> Companion to [FEATURES_TRACKER.md](FEATURES_TRACKER.md). Each defect is
> `D-###`, references the affected story id(s), and records observed vs
> expected behaviour, root cause, fix, and verification.

## Summary

| ID | Severity | Title | Stories | Status |
|----|----------|-------|---------|--------|
| D-001 | 🔴 high | `?tag=` query filter always 400s (single + repeated) | F-006, F-087 | ✅ fixed + verified |
| D-002 | 🔴 critical | Dashboard "Today's brief" renders nothing (`it.insight` undefined) | F-067 | ✅ fixed + verified |
| D-003 | 🟠 medium | Empty `api_prefix` panics the server at boot | F-084 | ✅ fixed + verified (+ regression test) |
| D-004 | 🟠 medium | Dashboard not served by API (dead in Docker + poor local UX) | F-064–F-080, F-088 | ✅ fixed + verified |
| D-005 | 🔴 high | Auth bypass via `/health` path suffix/substring matching | F-023, F-012 | ✅ fixed + verified (+ 4 regression tests) |
| D-006 | 🔴 high | Signal `series_jump` preview ≠ production (unscaled vs cadenced) | F-023 | ✅ fixed + verified (+ 4 regression tests) |
| D-007 | 🟡 low | Bad `?since=` silently returns unfiltered insights (no 400) | F-011 | ✅ fixed + verified (+ 4 regression tests) |
| D-008 | 🟡 low | Cite `base_url` docstring claims `Host` header is used (it isn't) | F-017 | ✅ fixed (doc corrected) |
| D-009 | ⚪ risk | No owner isolation on signals/investigations (shared-key model) | F-022/26/28/30 | ⚠️ waived (documented v1 design) |
| D-010 | 🟠 medium | Sessions never expire (leaked bearer valid forever) | F-035 | ✅ fixed + verified (+ 3 regression tests) |
| D-011 | 🟡 low | Python client missing 8 endpoint families (signals/auth/cite/…) | F-056 | ✅ fixed + verified (+5 tests) |
| D-012 | 🔴 critical | Widened HKMA catalog silently broke the agent: dead scan-target slug + hash record-ids + dropped `hibor` tag + agent runs before data warms | F-038,F-039,F-046,F-067,F-077,F-089 | ✅ fixed + verified (+ 7 regression tests) |
| D-013 | 🟠 medium | `trend_break` detector wired into scheduler/tools but missing from signal preview (D-006 class) | F-023 | ✅ fixed + verified (+ 1 regression test) |
| D-014 | 🔴 critical | Python `_investigation` parsed step timestamp from nonexistent `created_at` (Rust serializes `executed_at`) and silently dropped `answer` + `trace` | F-056 | ✅ fixed + verified (+ strengthened tests) |
| D-015 | 🟠 medium | `append_investigation_step` crashed on `answer=`/`trace=` (dataclasses not JSON-serializable) | F-056 | ✅ fixed + verified (+ regression test) |
| D-016 | 🟠 medium | Python client missing `update_signal` (PATCH /v1/signals/{id}); new models not exported | F-056 | ✅ fixed + verified (+ regression test) |
| D-017 | 🟡 low | Docs/config drift: stale `?api_key=` auth claim, `routes.rs`→`routes/` path, ROADMAP missing v7/v8, detectors list missing trend_break/threshold_crossing | F-084 | ✅ fixed (doc/config only) |
| D-018 | 🔴 critical | Dashboard has no authentication UI — per-user features (signals, investigations, silence-watch) all 401 from the browser | F-020–F-025, F-053, F-061, F-062 | ⚠️ waived (missing feature, not a smallest-fix defect) |
| D-019 | 🟠 high | `correlation` detector missing from signal preview (D-006/D-013 class) | F-022 | ✅ fixed + verified (+ 2 regression tests) |
| D-020 | 🟠 high | Signal preview truncated at 500 rows (single `get_page`); scheduler paginates | F-022 | ✅ fixed + verified (+ 1 regression test) |
| D-021 | 🟡 medium | `threshold_crossing` missing from agent tool belt (`run_detector`) — unreachable from LLM loop | F-043 | ✅ fixed + verified (+ 3 regression tests) |
| D-022 | 🟡 low | `funding` tab missing from boot hash-route whitelist (`#funding` cold load → overview) | F-067 | ✅ fixed + verified |
| D-023 | 🟡 low | `signal_id` omitted cadence/comparison/field_b/companion/join_field → distinct signals collided + overwrote | F-020 | ✅ fixed + verified (+ 5 regression tests) |
| D-024 | 🟡 low | `series_jump` default-threshold magic literal `25.0` undocumented across 3 dispatch sites | F-041–F-043 | ✅ fixed + verified (named constant, no behavior change) |
| D-025 | 🟠 high | `/ask` heuristic matcher returns wrong dataset when question contains stop-words ("what is the interbank liquidity?") | F-056 | ✅ fixed + verified (+ 2 regression tests) |
| D-026 | 🔴 critical | data.gov.hk catalog widened: 20 datasets return 0 records (incl. money-lenders-licensees, the documented flagship) — API returns 422 "Not a valid resource" | F-098 | ✅ fixed + verified (+ 1 regression test) |
| D-027 | 🟠 high | Dashboard `DEFAULT_API_BASE` hardcoded to a specific Railway URL — API-served `/dashboard` deploy silently points at remote prod unless a port is in the URL | F-112 | ✅ fixed + verified |
| D-028 | 🟡 low | Dashboard per-user tabs (signals/cases) show "no signals"/"no cases" on a 401 instead of prompting sign-in (misleading empty state) | F-141,F-143 | ✅ fixed + verified |
| D-029 | 🔴 critical | `record_count` derived live from the TTL-bound moka cache → after 600s TTL all datasets show 0 records until next refresh (24h for datagovhk); breaks catalog + silence index + `/ready` | F-005,F-024,F-004 | ✅ fixed + verified (+ 1 regression test) |
| D-030 | 🟠 medium | Flaky `persist::tests::debounced_snapshot_coalesces` — 100ms debounce + 300ms wait too tight under concurrent test load on Windows | F-155 | ✅ fixed (widened wait window) |
| D-031 | 🔴 critical | Records cache TTL (600s) shorter than every refresh interval (1800s–604800s) → `/records`, `/cite`, `/unprecedentedness` return 502 for ~97% of each cycle; flagship cite-manifest reproducibility broken | F-015,F-029,F-033,F-073,F-124,F-125,F-130,F-132,F-137,F-139,F-144 | ✅ fixed + verified (+ 4 regression tests) |
| D-032 | 🟠 medium | Signal preview silently returns `count:0` (no findings) when records cache is cold — misleading "never fires" instead of "couldn't evaluate right now" | F-043 | ✅ fixed + verified (`data_available` field) |
| D-033 | 🔴 high | Dashboard has no auth UI — per-user features (signals, cases, silence-watch) all 401 from the browser; prior copy told users to hand-craft curl to `/v1/auth/request-token` + `/v1/auth/redeem` | F-123,F-139–F-144 | ✅ fixed + verified (in-page magic-link sign-in) |
| D-034 | 🟡 low | FEATURES_TRACKER F-111 spec claimed hash format `#page-<tab>`; actual (and boot.js-verified) format is `#<tab>` — doc/external-link guidance wrong | F-111 | ✅ fixed (tracker corrected) |

> **Third independent re-audit (D-006 → D-011).** A fresh, from-scratch QA cycle
> was run with **no assumption** the prior audit (D-001 → D-005) was complete. It
> re-verified all five prior fixes (still fixed) and then hunted across the
> v8/v9 product surface — signals, identity, cite, silence-index,
> unprecedentedness, bilingual, the dashboard, and the Python client — for
> defects the earlier passes missed. Details below; full per-test traces in
> `docs/archive/QA_PHASE2_3_TESTS_DEFECTS.md` and `docs/archive/QA_PHASE5_REGRESSION.md`.

> **Independent re-audit.** All four defects were re-verified end-to-end from
> a clean rebuild with no assumption the fixes still held. All four reproduce
> as fixed. No new code defects were found. Details + the one environmental
> caveat below; see also the QA re-audit section at the foot of this file.

---

## D-001 — `?tag=` query filter always returns 400

- **Stories:** F-006 (`GET /v1/sources?tag=`), F-087 (Python client `.sources(tag=…)`)
- **Severity:** high — a documented, unit-tested filter is broken on the live HTTP path
- **Observed:**
  - `GET /v1/sources?tag=hibor` → `400 Failed to deserialize query string: tag: invalid type: string "hibor", expected a sequence`
  - `GET /v1/sources?tag=hibor&tag=licensing` (repeated, the form the docs/tests imply works) → **same 400**
  - `GET /v1/sources?tag[]=hibor` (bracket form) → works (200, correct results)
  - Python client `HkGov.sources(tag="hibor")` and `.sources(tag=["hibor"])` → both raise `HkGovError: 400`
- **Expected:** `?tag=hibor` and `?tag=hibor&tag=licensing` return matching datasets (any-tag semantics). Confirmed by unit tests `sources_filters_by_tag` / `sources_tag_matches_any` in `routes.rs:591-614` which pass because they call `list_sources` directly, bypassing axum's query deserializer.
- **Root cause:** `DatasetFilter.tag: Vec<String>` (`routes.rs:156`) deserializes via axum's `Query` extractor, which uses `serde_urlencoded`. That library maps a single `key=value` to a `String`, not a `Vec`, so serde rejects it as "invalid type: string, expected a sequence". The repeated form also fails because `serde_urlencoded` does not accumulate repeated keys into a sequence by default. The bracket form (`tag[]`) only "works" incidentally.
- **Fix:** Parse `tag` straight off the raw query string (`RawQuery` extractor) instead of via `serde_urlencoded`, which rejects both lone and repeated `tag=` values for any field type. `tag` was removed from the `DatasetFilter` struct entirely; `DatasetFilter::tags(raw_query)` now handles all three forms (single, repeated, comma-separated). Added a third `RawQuery` argument to `list_sources`. (`routes.rs`)
- **Verification (Phase 4):** all three forms return 200 with correct results — `?tag=hibor` → `[daily-interbank-liquidity]`; `?tag=hibor&tag=licensing` → `[money-lenders-licensees, daily-interbank-liquidity]`; `?tag=hibor,licensing` → same. Python `HkGov.sources(tag="hibor")` and `.sources(tag=["hibor","licensing"])` both work. New Rust tests `sources_filters_by_tag`, `sources_tag_matches_any_repeated`, `sources_tag_matches_any_comma` added.

## D-002 — Dashboard "Today's brief" renders nothing

- **Stories:** F-067 (Today's brief hero) — the primary product surface of the v9 dashboard
- **Severity:** critical — the headline section of the dashboard is blank
- **Observed:** The "Today's brief" section renders no cards. Reproduced by simulating `loadBrief()` (`index.html:264`) against the live `/v1/brief` payload: every item's `insightCard(it.insight, true)` call receives `undefined` because `it.insight` does not exist on the flattened brief item.
- **Expected:** Brief items render as insight cards in the hero.
- **Root cause:** Schema mismatch between Rust serialization and JS expectation.
  - Rust `BriefItem` (`brief.rs:17-25`) uses `#[serde(flatten)]` on the `insight` field, so the insight's fields (severity, title, summary, evidence, …) are **flattened to the top level** alongside `rank` and `score`. The serialized JSON has NO `insight` key.
  - Dashboard JS (`index.html:273`) reads `it.insight` and passes it to `insightCard()`. Since `it.insight` is `undefined`, `insightCard` either throws or no-ops, leaving the hero empty.
  - Note: the "All insights" feed (`loadInsights`) works because it passes the raw insight objects directly to `insightCard` — only `loadBrief` is affected.
- **Fix:** Change `loadBrief` (`dashboard/index.html:273`) to pass the (already-flattened) item itself to `insightCard`: `insightCard(it, true)`. The flattened item has every field `insightCard` reads; rank/score are harmless siblings.
- **Verification (Phase 4):** Simulated the fixed `loadBrief` against the live `/v1/brief` payload — all 3 items render as valid cards (`title`, `severity`, `rank`, `score` all present). The "All insights" feed was unaffected throughout (it already passed raw insights).

## D-003 — Empty `api_prefix` panics the server at boot

- **Stories:** F-084 (API prefix configurable)
- **Severity:** medium — a documented config knob crashes the process
- **Observed:** `HKGOV_API__API_PREFIX="" ./hkgov-api` →
  ```
  thread 'main' panicked at crates\api\src\routes.rs:68:16:
  Overlapping method route. Handler for `GET /health` already exists
  ```
  The server never starts.
- **Expected:** an empty prefix mounts all routes at root (as the code intends: `routes.rs:67-71` switches to `merge` when prefix is empty).
- **Root cause:** when the prefix is empty, the code `merge`s `api_routes` (which defines `/health`) into the root router (which also defines `/health` at `routes.rs:65`). Two handlers for the same path → axum panics. The `if prefix.is_empty()` branch was added to support no-prefix mode but didn't account for the duplicate `/health`.
- **Fix:** When the prefix is empty (merge path), do NOT add a root-level `/health` — `api_routes` already carries one and merge brings it to root. The root-level `/health` is now only added in the nested (non-empty prefix) case. (`routes.rs:62-76`)
- **Verification (Phase 4):** `HKGOV_API__API_PREFIX="" ./hkgov-api` boots cleanly (no panic); `/health`, `/sources`, `/insights`, and `/` all respond 200 at root. Default `/v1` prefix path unchanged.

## D-004 — Dashboard not served by the API

- **Stories:** F-064–F-080 (all dashboard stories, since reaching the dashboard is prerequisites), F-088 (Docker image claims to carry the dashboard)
- **Severity:** medium — UX/logistical; the dashboard works if you find the file and open it, but the documented happy paths are broken
- **Observed:**
  - `GET /dashboard`, `GET /index.html` → 404. The API serves no static files.
  - The README instructs users to "open dashboard/index.html in a browser (point it at http://localhost:8080)". Opening via `file://` works only because the JS falls back to `http://localhost:8080` when the baseUrl input is empty — fragile and non-obvious.
  - The Dockerfile (`Dockerfile:4,47`) claims the image carries "the static dashboard" and copies `dashboard/index.html` into `/app/dashboard/index.html`, but there's no way to retrieve it from the running container. The dashboard is dead in the Docker path.
- **Expected:** the API serves the dashboard at a known path (e.g. `GET /` returns the dashboard HTML, or `GET /dashboard/` serves it), so `docker run` + open-browser "just works".
- **Root cause:** no static-file route is wired into the axum router. The dashboard was designed as a standalone file but the deployment/packaging story assumes it's served.
- **Fix:** Add a `GET /dashboard` (and `/dashboard/`) route serving the dashboard HTML embedded at compile time via `include_str!("../../../dashboard/index.html")`, so the binary — and the Docker image — are self-contained. The route lives at the root router level (outside the versioned API) so it's reachable regardless of `api_prefix`, and is exempt from API-key auth (a static asset, not data). The root `GET /` directory now advertises it. (`routes.rs`)
- **Verification (Phase 4):** `curl /dashboard` → 200, `Content-Type: text/html`, body begins `<!DOCTYPE html>` and contains "HK City Pulse". `/dashboard/` (trailing slash) also 200. With `HKGOV_API__API_KEY` set, `/dashboard` returns 200 without a key (exempt) while `/v1/sources` correctly returns 401. Python client unchanged (static asset). The Docker path now works: `docker run -p 8080:8080 …` then open `http://localhost:8080/dashboard`.

## D-005 — Authentication bypass via `/health` path suffix/substring matching

- **Stories:** F-023 (API key auth), F-012 (`GET /v1/datasets/{source}/{dataset}`)
- **Severity:** high — broken access control. When API-key auth is enabled, an
  unauthenticated requester can reach protected data routes whose path collides
  with the health-exemption pattern.
- **Discovered by:** independent re-audit (a fresh QA pass that did not assume the
  prior four fixes were the complete set).

- **Observed (pre-fix, key-enabled instance, no key sent):**
  - `GET /v1/datasets/hkma/health` → **HTTP 200** (auth bypassed; the dataset is
    unknown so the body is `null`, but the gate that should have returned 401
    never ran).
  - `GET /v1/datasets/hkma/health/records` → **HTTP 502** with
    `{"error":{"kind":"store","message":"no records cached for hkma/health"}}` —
    the request reached the records handler and the store layer, proving the
    bypass goes past auth into data-path code.
  - `GET /v1/datasets/health/anything` → **HTTP 404** (auth bypassed; the 404 is
    from `DataSource::parse("health")` failing, not from auth).
  - Control: `GET /v1/datasets/hkma/daily-interbank-liquidity` (no key) → **401**
    — normal protected paths are gated correctly. So only the colliding paths leak.

- **Expected:** every non-health `/v1` route requires a key when `api.api_key` is
  set (`auth.rs:23-35`). The exemption is meant for the liveness endpoints only.

- **Root cause:** the guard's exemption test in `crates/api/src/auth.rs:26` was:
  ```rust
  if path.ends_with("/health") || path.contains("/health/") || path == "/" {
  ```
  `ends_with("/health")` matches *any* path ending in `/health`, including data
  routes like `/v1/datasets/hkma/health`. `contains("/health/")` likewise matches
  `/v1/datasets/health/records`. These substring/suffix checks were written to be
  prefix-agnostic (the API can be mounted under a custom prefix), but the guard
  runs on `api_routes` *after* axum strips the prefix — so the health endpoints
  always resolve to exactly `/health` and `/health/sources` here. There was no
  need for fuzzy matching; exact matching is both correct and safe.

- **Impact assessment:**
  - The bypass reaches the **dataset metadata + records handlers** for any
    `{dataset}` whose name ends in `health` (suffix form) or for any path
    containing a `/health/` segment. With today's dataset names none end in
    `health`, so the live data leak is currently nil — but the auth mechanism was
    structurally broken, and any future dataset named `*health` would be fully
    exposed (metadata + records) without a key. A latent security landmine.

- **Fix:** replace the loose check with exact path matching (`crates/api/src/auth.rs`):
  ```rust
  if path == "/" || path == "/health" || path == "/health/sources" {
  ```
  The guard lives on `api_routes`, which axum mounts under the configured prefix
  (stripping it for inner middleware), so `/health` and `/health/sources` are the
  exact paths seen here regardless of the prefix. No fuzzy matching is needed.

- **Verification (Phase 5):**
  - New unit/regression tests in `auth.rs` (drive the full `router()` with a key
    enabled): `dataset_route_named_health_requires_key` asserts
    `/v1/datasets/hkma/health` and `/v1/datasets/hkma/health/records` both return
    **401** without a key and pass with a correct key;
    `health_paths_exempt_without_key` asserts `/health`, `/v1/health/sources`,
    and `/` stay exempt; `normal_protected_routes_require_key` and
    `wrong_key_rejected` guard the normal path. (+4 net new tests; workspace
    count 86 → **90**.)
  - Live HTTP regression on a key-enabled instance: the two bypass paths are now
    **401** (were 200/502); `/health`, `/v1/health/sources`, `/`, and
    `/dashboard` remain **200** without a key; D-001 → D-004 all still pass.

---

## QA re-audit (independent end-to-end re-verification)

A full 4-phase audit/test/remediate/regress cycle was re-run from a clean
`cargo build --release` with **no assumption** that the fixes above still held.
Each defect was reproduced from its documented trigger and the spec'd expected
behaviour was asserted.

### Per-defect re-verification

| Defect | Trigger exercised | Observed | Verdict |
|--------|-------------------|----------|---------|
| D-001 | `?tag=hibor`, `?tag=hibor&tag=licensing`, `?tag=hibor,licensing` | all **200**; `1`, `2`, `2` datasets respectively (any-tag match) | ✅ fixed |
| D-002 | `GET /v1/brief?limit=5` + dashboard `loadBrief` simulated against the live payload | items carry insight fields **flattened** (no `.insight` key); `insightCard(it, true)` renders **5 cards** (was 0); zero stale `it.insight` refs in served HTML | ✅ fixed |
| D-003 | empty prefix via `config.toml` (`api_prefix = ""`) **and** via `HKGOV_API__API_PREFIX=""` | boots clean (no panic, no "Overlapping method route"); `/health`, `/sources`, `/insights`, `/brief`, `/categories`, `/alerts`, `/dashboard` all **200 at root**; `/v1/sources` **404** | ✅ fixed |
| D-004 | `GET /dashboard`, `GET /dashboard/` | both **200**; `text/html; charset=utf-8`; 23115 bytes; `<!DOCTYPE html>` + "HK City Pulse"; exempt from API-key auth | ✅ fixed |

### Findings (no new code defects)

1. **All four defects genuinely fixed.** The Phase 2 → Phase 4 claims in
   `FEATURES_TRACKER.md` are accurate.
2. **D-003 hardening.** The empty-prefix merge branch had no integration test
   locking it down — a silent regression (routes dropped by the merge) would
   not have been caught. Added two routing tests that drive the full `router()`
   through `tower::ServiceExt::oneshot`:
   - `empty_prefix_mounts_all_routes_at_root` — asserts all 11 API + root +
     dashboard paths return 200 at root and `/v1/sources` returns 404.
   - `default_prefix_nests_routes_under_v1` — symmetric guard for the
     `/v1` default.
   Workspace test count: 84 → **86**. Both new tests pass; clippy/fmt clean.
3. **Auth matrix re-verified** on a key-enabled instance: 401 on missing/wrong
   key, 200 on correct key (both `X-API-Key` header and `?api_key=`), and
   `/`, `/health`, `/dashboard` all correctly exempt.

### Environmental notes (not defects)

- **Port conflicts on the dev host.** Ports 8080 and 8090 were occupied by
  unrelated services (`akshare-sidecar`, a `uvicorn` app). The live regression
  was run on free ports (8765/8771/8780). This is host-specific and has no
  bearing on the binary, which honours `HKGOV_API__BIND`.
- **One transient false-negative.** An early empty-prefix probe (with
  `HKGOV_API__API_PREFIX=""`) reported `/sources` 404 while `/v1/sources`
  returned 200 — implying the override hadn't taken. This did **not**
   reproduce on any subsequent clean run (the env override works). Attributed
  to a race with a concurrently-launched sibling process during the first
  probe. Documented here so a future auditor doesn't chase a ghost.
- **Press connector flakiness is upstream.** One boot logged a transient
  transport error fetching `press-releases`; the retry path recovered on the
  next interval. Not a code defect — the HKMA upstream occasionally resets.

---

## Second independent re-audit (the pass that found D-005)

A fresh, from-scratch QA cycle was run with **no assumption** that the prior
audit was complete. It re-verified D-001 → D-004 (all still fixed) and then
hunted for defects the first pass missed, focusing on the auth/middleware layer,
the detector math, the Python client, and the dashboard JS.

### What it found

- **D-005 (new, high/security):** the API-key auth guard exempted any path ending
  in `/health` or containing `/health/`, which let unauthenticated requests reach
  `/v1/datasets/{source}/health` and friends. Fixed with exact-path matching;
  details above. This is the only new code defect found in this pass.

### What it checked and cleared

- All five detectors' math (`series_jump`, `outlier` MAD, `seasonality`
  autocorrelation, `correlation`/`proxy_divergence` Pearson, `benchmark_deviation`,
  `year_over_year`, `threshold_crossing`) — guards for zero-variance, empty
  inputs, sub-min-samples, and division-by-zero are all present and correct.
- HKMA retry/backoff loop, three-state circuit breaker, per-source rate limiter.
- Python client (`hkgov-py`): tag list/string handling, brief re-nesting,
  feedback, error mapping — all correct (14/14 tests pass).
- Dashboard JS: `loadBrief` (D-002 fix holds), `insightCard`, chat rail, vote,
  collapse toggles, auto-poll — logic sound. Minor non-blocking notes only
  (see FEATURES_TRACKER.md "Non-blocking observations").

### Verification gates (this pass)

| Gate | Result |
|------|--------|
| `cargo build --release -p hkgov-api` | ✅ clean |
| `cargo test --workspace --release` | ✅ **90 passed**, 0 failed (+4 auth guards) |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ clean |
| `cargo fmt --all -- --check` | ✅ clean |
| Python `pytest tests/` | ✅ 14 passed |
| Live regression (key-enabled + open + empty-prefix instances) | ✅ D-005 fixed; D-001 → D-004 intact |

### Environmental notes (this pass)

- **HKMA monetary-statistics endpoints unreachable from the sandbox.** Direct
  `curl` to `api.hkma.gov.hk/.../capital-market-statistics` and `.../daily-figures-interbank-liquidity`
  timed out (HTTP 000), while `.../press-releases` returned 200. The connector's
  retry path fired correctly (attempts 0→3, then gave up with an `Upstream`
  error); the circuit breaker recorded the failures. This is a network
  reachability issue in the test environment, not a code defect — the agent
  produced 0 insights only because its HKMA scan targets had no data to analyze.
  The pipeline itself (pass started → completed → stored:0) ran end to end.

---

## Third independent re-audit (the pass that found D-006 → D-011)

A fresh, from-scratch QA cycle was run with **no assumption** that the prior
audits were complete. It re-verified D-001 → D-005 (all still fixed — their 22
guards are green) and then hunted across the v8/v9 product surface (signals,
identity, cite, silence-index, unprecedentedness, bilingual, dashboard, Python
client) for defects the earlier passes missed. Full per-test traces in
`docs/archive/QA_PHASE2_3_TESTS_DEFECTS.md`; regression in `docs/archive/QA_PHASE5_REGRESSION.md`.

### Verification gates (this pass)

| Gate | Result |
|------|--------|
| `cargo test --workspace` | ✅ **189 passed**, 0 failed (baseline 178; +11 new regression tests) |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ clean |
| `cargo fmt --all -- --check` | ✅ clean |
| Python `pytest tests/` | ✅ 14 passed |

### D-006 — Signal `series_jump` preview ≠ production

- **Stories:** F-023 (`POST /v1/signals/preview`)
- **Severity:** high — breaks the core product promise of signal subscriptions
- **Observed:** The `signal.rs` module docstring promises *"preview IS what will
  fire"* and *"reuses the scheduler's `run_one_target` so preview == production"*.
  The code violated both for `series_jump`: preview called the **unscaled**
  `detect_series_jumps` (`signal.rs:322`) while production called the
  **cadence-scaled** `detect_series_jumps_cadenced` (`scheduler.rs:222`). A
  quarterly signal previewed at threshold 25% fired on a 35% jump, but
  production (effective threshold 25 × √3 ≈ 43.3%) stayed silent. The preview
  also lacked a `year_over_year` arm entirely.
- **Empirical proof:** a throwaway example binary calling both paths on
  identical inputs printed `unscaled (preview) findings: 1 / cadenced (prod)
  findings: 0 / D-006 CONFIRMED: they DIVERGE`.
- **Expected:** preview == production, as documented.
- **Root cause:** the preview dispatcher predates the v7 cadence-scaling work
  and was never updated to mirror the scheduler's `run_one_target`.
- **Fix:** `run_detector_preview` (`signal.rs`) now mirrors the scheduler: the
  `series_jump` arm delegates to `detect_series_jumps_cadenced` (passing
  `target.cadence`) and routes YoY-comparison targets to
  `detect_year_over_year`; a new `year_over_year` arm handles direct YoY signals.
- **Verification (Phase 5):** 4 regression tests assert preview==production for
  quarterly/monthly/unknown cadences and for the YoY path. All pass.

### D-007 — Bad `?since=` silently returns unfiltered insights

- **Stories:** F-011 (`GET /v1/insights?since=`)
- **Severity:** low — misleading, not data-corrupting
- **Observed:** `GET /v1/insights?since=banana` returned 200 with the **full**
  insight list, as if "everything is new since banana".
- **Expected:** a 400 naming the bad value and the accepted formats.
- **Root cause:** `routes.rs` `list_insights` fell through to
  `state.insights.list(...)` when `parse_since` returned `Err`.
- **Fix:** the handler now returns `Err(ApiError(BadRequest(...)))` on an
  unparseable `since`.
- **Verification (Phase 5):** 4 tests — bad since → 400; valid RFC3339 / epoch
  / absent → Ok. All pass.

### D-008 — Cite `base_url` docstring claims `Host` header is used

- **Stories:** F-017 (`GET /v1/insights/{id}/cite`)
- **Severity:** low — doc/behaviour mismatch
- **Observed:** `CiteQuery::base_url` doc said "Defaults to the request's `Host`
  header origin, then localhost". The code only checked the query param, then
  hardcoded `http://localhost:8080` — the `Host` header was never read.
- **Expected:** doc and behaviour agree.
- **Root cause:** aspirational doc written before the simpler implementation landed.
- **Fix (doc-only, per approval):** corrected the docstring to state the caller
  must pass the public origin explicitly, with a deployment note for proxy
  setups. Behaviour unchanged (changing it behind a proxy needs operator sign-off).
- **Verification (Phase 5):** docstring now matches code.

### D-009 — No owner isolation on signals/investigations

- **Stories:** F-022, F-026, F-028, F-030
- **Severity:** risk (not a code bug per the v1 design)
- **Observed:** any keyed caller can `GET /v1/signals?owner=` (empty → all
  owners), and read/update/delete any other user's signals or investigations.
- **Expected (for multi-tenant):** owner-scoped ACL.
- **Root cause:** `owner` is a filter, not a guard — the documented "shared-key
  trust model" where every keyed caller is mutually trusting.
- **Resolution (waived, per approval):** not fixed in v1 — the single-trust-
  domain model is intentional. A loud `⚠️ D-009` note was added to `routes.rs`
  at the signals section with the remediation path: derive `owner` from the
  authenticated session and reject cross-owner mutations before any multi-tenant
  deployment.

### D-010 — Sessions never expire

- **Stories:** F-035 (`GET /v1/auth/me`)
- **Severity:** medium — security; magic-link identity's value is undermined if
  sessions are immortal
- **Observed:** a redeemed bearer resolved indefinitely. `Session` had no
  `expires_at`; `lookup_session` did no TTL check.
- **Expected:** a session TTL, mirroring the one-time token's 15-min TTL.
- **Root cause:** the `Session` struct and `lookup_session` were written before
  the security review and never gained an expiry.
- **Fix:** `Session` now carries `expires_at` (default far-future for back-compat
  with any legacy serialized blob); `redeem_token` sets it to `now + 30 days`;
  `lookup_session` rejects `now > expires_at`.
- **Verification (Phase 5):** 3 tests — fresh session expires ~30d out + resolves;
  back-dated session → None; legacy far-future default keeps old sessions alive.

### D-011 — Python client missing 8 endpoint families

- **Stories:** F-056 (`hkgov-py` client coverage)
- **Severity:** low — typed contract incomplete; endpoints still reachable via
  `_get`/`_post`
- **Observed:** `dir(HkGov)` lacked methods for signals, investigations, auth,
  cite, silence-index, unprecedentedness, insight-history, and the `since`/`lang`
  params — 8 endpoint families added in v8/v9 that the client never grew.
- **Expected:** parity with the HTTP surface.
- **Resolution (fixed):** the 8 originally-listed families were added in a
  prior pass (v7/v8 client methods + tests). This cycle closed the remaining
  2 gaps the original enumeration didn't anticipate: `market_players()` (`GET
  /v1/market-players` — the related-market-players directory) and
  `append_investigation_step()` (`POST /v1/investigations/{id}/steps` — the
  agent-driven step append). Also fixed a latent bug in
  `add_investigation_note()` which sent `{"text": text}` but the Rust
  `AddNoteRequest` expects `{"body": body}`. New `MarketPlayerGroup` +
  `PlayerEntry` dataclasses added to `models.py`. +5 Python tests (32 total,
  all passing).

### What this pass checked and cleared (no defect)

- All detector math (`series_jump`/`outlier`/`seasonality`/`correlation`/
  `cross_source_gap`/`proxy_divergence`/`benchmark_deviation`/`year_over_year`/
  `threshold_crossing`) — zero-variance, empty-input, sub-min-sample, and
  divide-by-zero guards all present and correct.
- Silence Index scoring (weights, squash constant, HKMA scoping, determinism).
- Unprecedentedness (percentile, MAD band, 1-in-N, MIN_HISTORY_POINTS gate).
- Cite-It manifest (SHA-256 drift detection, all 5 formats, determinism).
- Bilingual zh-HK reframer (all detector kinds, fallback for unknown kinds,
  severity translation, determinism).
- Agent loop (tool dispatch, step-exhaustion error, Findings-vs-Answer framing).
- Auth gate exact-path matching (D-005 regression).
- Dashboard JS (brief flattening per D-002, severity filter, vote, chat rail,
  auto-poll, responsive layout, ARIA).
- Telemetry bootstrap (plain/json/otel paths).
- Config load order (defaults < toml < env) and empty-prefix routing (D-003).

---

## Fourth independent re-audit (the pass that found D-012)

A fresh, from-scratch QA cycle was run with **no assumption** that the prior
audits were complete. Unlike the earlier passes — which leaned heavily on
unit tests and "JS logic verified by inspection" — this pass **booted the
live server against real HKGOV data and drove every user story end-to-end**
through HTTP (curl) and through a headless Node harness that executes the
dashboard's actual JS against the live API payloads.

### Verification gates (this pass)

| Gate | Result |
|------|--------|
| `cargo build --release -p hkgov-api` | ✅ clean |
| `cargo test --workspace` | ✅ **200 passed**, 0 failed (baseline 193; +7 D-012 guards) |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ clean |
| `cargo fmt --all -- --check` | ✅ clean |
| Python `pytest tests/` | ✅ 14 passed |
| Live server regression (3 instances: open / key-enabled / empty-prefix) | ✅ all pass |
| Headless dashboard harness (executes every page's JS vs live API) | ✅ no throws |

### D-012 — Widened HKMA catalog silently broke the agent (4 linked defects)

- **Stories:** F-038 (agent enabled), F-039 (series_jump / HIBOR), F-046
  (cross_source_gap), F-067 (brief hero), F-077 (tag chips), F-089 (Silence
  Index) — i.e. the project's **entire flagship surface**.
- **Severity:** critical — the headline feature (HIBOR spike detection +
  the "press room leaves it untold" Silence Index) produced nothing on a
  real boot, despite the prior audits marking these stories "pass".
- **Root cause:** the HKMA connector was expanded from 5 datasets to the
  full 151-dataset public catalog, but the downstream agent integration was
  never updated to match. Four linked breakages resulted:

  1. **Dead scan-target slug.** `default_scan_targets()` (config.rs) still
     referenced `daily-interbank-liquidity`, which was renamed to
     `daily-figures-interbank-liquidity` in the catalog rewrite. 3 of the 4
     default detectors pointed at a dataset that no longer exists.
  2. **Hash record-ids.** `record_id_for` (hkma.rs) only mapped 2 legacy
     slugs to date keys; all 149 new datasets fell through to an opaque
     `id-<hash>` record_id. This broke `cross_source_gap` (which joins press
     dates against data record_ids — hashes never matched, so every press
     date looked "unexplained") and made evidence pointers unreadable.
  3. **Dropped `hibor` tag.** The tag was removed from the interbank
     datasets in the rewrite, so `?tag=hibor` returned 0 — breaking the
     dashboard's tag-search and contradicting the README's flagship
     narrative.
  4. **Agent runs before its data warms.** The agent's first pass fired
     after a fixed 20s `sleep`. With 186 datasets warming concurrently under
     per-source rate limits (HKMA 5/s ⇒ ~37s for HKMA alone), the HIBOR feed
     had not been fetched when the first (and for 6 hours the only) pass
     ran, so it scanned an empty store. The README's "241 insights" was
     unreachable from a real boot.

- **Observed (pre-fix, live boot, agent enabled):**
  - `/v1/insights` → **4** insights (all `capital-market-statistics`); README
    claims ~241.
  - `/v1/silence-index?period=2026-Q2` → score **0.0**, total_events **0**.
  - `/v1/sources?tag=hibor` → **0** datasets.
  - `daily-figures-interbank-liquidity` record_ids → `id-839da50f…` (hash).
  - Dashboard Signals page: source=hkma defaults to the first-listed dataset
    (`hkd-interbank-trans`, which lacks `hibor_overnight`), so a fresh user's
    first Preview returned 0 findings.

- **Expected:** the flagship HIBOR detection produces findings on a real
  boot; the Silence Index reflects real opacity; tags and evidence pointers
  are human-readable.

- **Fix (four parts):**
  1. `default_scan_targets()` (config.rs): updated all three references from
     the dead slug to `daily-figures-interbank-liquidity`.
  2. `record_id_for` (hkma.rs): added a generic date/period-field fallback
     (`end_of_date`, `end_of_month`, `end_of_quarter`, `date`, `year_month`,
     `quarter`, `year`) so the ~150 widened datasets get natural, joinable
     ids instead of hashes; kept the explicit map for the legacy slugs.
  3. Restored the `hibor` tag on `daily-figures-interbank-liquidity` and the
     three `hk-interbank-ir-*` datasets; fixed the dashboard Signals page
     (`sigDatasetFill`) to auto-select a dataset that carries the configured
     field (preferring the canonical HIBOR feed), so a fresh user's first
     Preview hits real data.
  4. Replaced the fixed 20s `sleep` before the agent's first pass
     (main.rs) with `wait_for_scan_readiness`, which polls the store until
     every scan-target dataset (primary + companion) has ≥1 record, capped
     at 180s so a permanently-unreachable upstream never blocks the agent.

- **Verification (Phase 4, live boot, agent enabled):**
  - `/v1/insights` → **242** insights (238 HIBOR `series_jump` + 4
    capital-market), incl. "hibor_overnight moved +99.3% in 2026-02-16" —
    exactly the README's flagship example.
  - `/v1/silence-index?period=2026-Q2` → score **75.76**, 25 unattributed
    jump events.
  - `/v1/sources?tag=hibor` → **4** datasets.
  - `daily-figures-interbank-liquidity` record_ids → real dates
    (`2026-06-23`, …).
  - Dashboard harness: `insights` region 10548 → **164632** chars rendered;
    no JS throws across any page.
  - 7 new regression tests (3 in config.rs, 4 in hkma.rs) lock the slug,
    the HIBOR coverage, the cross_source_gap companion, the date-key
    record_id behaviour, and the `hibor` tag. Workspace count 193 → **200**.

### Why the prior audits missed it

The earlier passes verified these stories through unit tests (which seed
their own in-memory stores with the old slug) and by "inspecting the JS
logic". Neither path exercises the real connector → real HKMA data → real
agent-scheduler chain, so a slug rename that broke the live pipeline was
invisible to them. This pass's distinguishing method was **booting against
live data and asserting on the served output**, which is where the 4-insight
count became undeniable.

### What this pass checked and cleared (no new defect)

- All 11 prior defects (D-001 → D-011): re-verified end-to-end against a
  clean rebuild — all still fixed.
- Full auth matrix (key on/off, header vs query, D-005 health-suffix bypass)
  on a key-enabled instance.
- Empty-prefix routing (D-003) on a dedicated instance.
- Dashboard JS across all six pages via the headless harness (Overview,
  Datasets, Signals, Cases, Health, Licences) — no throws, no undefined refs.
- Signals preview, create, list, investigations create/steps/notes/delete,
  auth request-token/redeem/me, cite all five formats + bundle + manifest,
  unprecedentedness, since-filter (D-007), feedback round-trip.

---

## Fifth PM-coordinated audit (the pass that found D-013 → D-017)

A project-manager-coordinated review fanned out three independent audit agents
(Rust crates, Python client, docs/config/CI) across the full tree plus the
uncommitted work-in-progress (the new `trend_break` detector, the Python
v7/v8 client additions, the `include_str!` path hardening, and the doc
reorganization). It re-verified D-001 → D-012 (all still fixed) and then
hunted across the WIP surface for defects the earlier passes — which predated
the `trend_break` work and the Python v7/v8 expansion — could not have seen.

### Verification gates (this pass)

| Gate | Result |
|------|--------|
| `cargo test --workspace` | ✅ **268 passed**, 0 failed (+1 new `preview_trend_break` test) |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ clean |
| `cargo fmt --all -- --check` | ✅ clean |
| Python `pytest tests/` | ✅ **35 passed** (baseline 32; +3 new tests) |

### D-013 — `trend_break` missing from signal preview (D-006 class)

- **Stories:** F-023 (`POST /v1/signals/preview`)
- **Severity:** medium — same defect class as the documented D-006: preview ≠
  production for a detector. A `trend_break` signal previewed empty even though
  the scheduler would fire it in production.
- **Root cause:** the new `trend_break` detector was wired into the scheduler
  (`run_one_target`) and the tool belt (`RunDetectorTool`) but the preview
  dispatcher (`run_detector_preview` in `signal.rs`) — whose module docstring
  states the invariant "preview IS what will fire" — had no `trend_break` arm.
- **Fix:** added a `"trend_break" =>` arm mirroring the scheduler (`threshold`
  as min-run-length, defaulting to `DEFAULT_TREND_BREAK_MIN_RUN`).
- **Verification:** new test `preview_trend_break_fires_on_reversal` asserts a
  3-period-rise-then-reversal previews ≥1 finding (was 0 before the arm).

### D-014 — Python `_investigation` dropped step `answer`/`trace` + wrong timestamp field

- **Stories:** F-056 (Python client coverage)
- **Severity:** critical — silently lost data on every investigation read. The
  Rust `InvestigationStep` serializes `executed_at`, `answer`, and `trace`; the
  Python helper read the nonexistent `created_at` (always `None`) and never
  parsed `answer` or `trace`.
- **Root cause:** the helper predated the agent-driven step endpoint and was
  never updated when `answer`/`trace`/`executed_at` were added to the Rust
  struct. The existing tests mocked rich payloads but only asserted `kind`/
  `prompt`, masking the drop.
- **Fix:** renamed the dataclass field `created_at` → `executed_at`; the helper
  now parses `executed_at`, `answer` (via a new `_answer` helper), and `trace`
  (via a new `_trace` helper). Existing step tests strengthened to assert on
  the previously-dropped fields.

### D-015 — `append_investigation_step` crashed on `answer=`/`trace=`

- **Stories:** F-056
- **Severity:** medium — runtime `TypeError` on a documented parameter.
  `Answer`/`TraceStep` are dataclasses; `requests` cannot JSON-serialize them,
  so passing an `Answer` returned from `ask()` crashed at call time.
- **Fix:** added `_trace_to_dict` serializer; `answer.trace` and `trace` are
  now serialized to dicts before going into the request body.
- **Verification:** new test `test_append_investigation_step_serializes_trace`
  passes a `TraceStep` list and asserts the serialized body (would have raised
  before the fix).

### D-016 — Python client missing `update_signal`; new models not exported

- **Stories:** F-056
- **Severity:** medium — API parity gap. The Rust router exposes
  `PATCH /v1/signals/{id}` but the client had no method for it. `MarketPlayerGroup`
  and `PlayerEntry` were added to `models.py` but not re-exported from the
  package `__all__`. (`auth_me` also hardcoded `/v1` instead of the configured
  prefix — fixed in the same pass.)
- **Fix:** added `update_signal(signal_id, *, question, compiled, channels,
  enabled)` (+ a `_patch` HTTP helper); exported the two new models; `auth_me`
  now uses `_get("/auth/me")` and respects the prefix.
- **Verification:** new tests `test_update_signal` + `test_update_signal_requires_a_field`.

### D-017 — Docs/config drift (stale claims, missing milestones, missing detectors)

- **Stories:** F-084 (docs accuracy)
- **Severity:** low — documentation only.
- **Root cause:** incremental WIP left several surfaces stale: README claimed
  `?api_key=` query auth (removed as security hardening) and referenced
  `routes.rs` (now `routes/`); `docs/ROADMAP.md` documented only v1–v6 while
  README/CHANGELOG listed v7/v8 as shipped; `config.toml`'s detector list
  omitted `threshold_crossing` and `trend_break` despite both being real,
  dispatched detectors.
- **Fix:** corrected all stale references; added v7/v8 sections to ROADMAP and
  synced INDEX (`v1–v6` → `v1–v8`); documented `threshold_crossing` +
  `trend_break` in `config.toml`; refreshed the `run_detector` tool description
  to list all 10 detectors.

### What this pass checked and cleared (no new defect beyond D-013 → D-017)

- The `include_str!` `CARGO_MANIFEST_DIR` path change resolves correctly
  (`crates/api` + `/../../dashboard/` = `dashboard/`); all 7 referenced files
  exist; `cargo check`/`clippy` clean.
- `detect_trend_break` math (run-length counting, reversal detection, index
  mapping, division-by-zero guard, `min_run.max(2)` clamp) — verified by hand
  and by 8 passing unit tests.
- All README curl examples reference real routes; all referenced files exist
  (`scripts/demo.sh`, `EXAMPLES.md`, `CHANGELOG.md`, etc.).
- Local tooling artifacts (`.agentic/`, `.zcode/`, `skills-staging/`, `data/`,
  `agentic.json`) now `.gitignore`d — were previously at risk of an accidental
  `git add .` commit.

---

> **Fourth independent re-audit (D-018 → D-024).** A fresh, from-scratch QA cycle
> was run with **no assumption** the prior audits (D-001 → D-017) were complete.
> It re-verified the full feature surface (73 features across 4 roles: every
> route, every dashboard screen, every operator config knob) and hunted for
> defects the earlier passes missed — focusing on the agent detection layer's
> three dispatch sites (scheduler / signal preview / tool belt), the dashboard's
> auth model, and id-dedup correctness.

## D-018 — Dashboard has no authentication UI (per-user features all 401)

- **Stories:** F-020–F-025 (signals/investigations CRUD), F-053 (silence-watch),
  F-061 (investigations UI), F-062 (signal subscriptions UI)
- **Severity:** critical — an entire UX surface (Signals tab, Cases tab,
  "Save a watch") is non-functional from the browser.
- **Observed:** Every per-user action from the dashboard returns
  `401 Unauthorized`: "authentication required: send a valid
  `Authorization: Bearer {session}`". `saveSignal`, `loadSignals`,
  `investigate`, `loadCases`, `watchSilenceIndex` all fail. The dashboard's
  API client (`api.js`) only ever attaches an `X-API-Key` header (the operator
  credential, R2), never a `Bearer` session token (the user credential, R3).
  The signal/case requests carry a dead `owner:'dashboard'` body field that
  the server ignores (V-004 derived owner from the session).
- **Expected:** The dashboard provides a login flow (`POST /v1/auth/request-token`
  with an email → magic link → `POST /v1/auth/redeem` → store the session
  token → attach `Authorization: Bearer {session}` on per-user requests), so a
  user can manage their own signals and investigations.
- **Root cause:** The identity tier (P-108) shipped the server-side store +
  HTTP routes but the dashboard was never given a login UI or session
  management. `require_principal` (the guard on every mutating per-user route)
  returns 401 when no Bearer session is present — which is always, from the
  dashboard. This is a **missing feature**, not a regression.
- **Waiver rationale:** Implementing a magic-link login flow in the dashboard
  (email input → token request → delivery → redeem → session persistence →
  header injection on every per-user call → session-expiry handling →
  bilingual strings → i18n wiring per AGENT.md) is a multi-file feature build,
  not the "smallest, safest code fix" the QA remediation phase targets. It is
  documented here as a known gap and tracked as future work. The server-side
  contract is correct and fully tested; the gap is purely client-side.
- **Status:** ⚠️ waived (documented missing feature).

## D-019 — `correlation` detector missing from signal preview

- **Stories:** F-022 (signal preview)
- **Severity:** high — a documented, single-dataset detector previews as 0
  findings even when production fires. Violates the module's core invariant
  ("preview IS what will fire").
- **Observed:** `POST /v1/signals/preview` with a `correlation` scan target
  returns `{count: 0, findings: []}` regardless of data, even when the
  scheduler's `run_one_target` would fire the same target.
- **Expected:** Preview runs the same detector the scheduler does. `correlation`
  is a single-dataset detector (both fields on the same records,
  `analysis.rs:507`), so — unlike `proxy_divergence`/`benchmark_deviation`/
  `cross_source_gap` — it needs no companion dataset and IS previewable.
- **Root cause:** `run_detector_preview` (`signal.rs`) is a hand-maintained
  mirror of the scheduler's dispatch. It had arms for `threshold_crossing`,
  `series_jump`, `year_over_year`, `outlier`, `seasonality`, `trend_break`,
  but **no `correlation` arm** — it fell through to `_ => Vec::new()`. The
  scheduler (`scheduler.rs:300`) and the tool belt (`tools.rs:586`) both
  handle `correlation`. This is the exact D-006/D-013 defect class (a detector
  wired into the scheduler but silently empty in preview); the module doc at
  `signal.rs:432` even claims every self-contained detector arm calls the same
  function the scheduler does, and `correlation` broke that claim.
- **Fix:** Added a `correlation` arm to `run_detector_preview` (`signal.rs`)
  that mirrors the scheduler's guard (requires `field_b`, else empty) and calls
  `detect_correlation` with the same threshold-defaulting (`DEFAULT_CORRELATION_R`).
- **Verification:** new tests `d019_correlation_preview_fires_when_decoupled`
  (asserts preview == production on decoupled series) +
  `d019_correlation_preview_missing_field_b_is_empty`. Both pass.

## D-020 — Signal preview truncated at 500 rows

- **Stories:** F-022 (signal preview)
- **Severity:** high — preview silently scores a strict subset of the data the
  scheduler sees, so a finding on row 600 is invisible to preview.
- **Observed:** `preview_signal` called a single `store.get_page(id, 0, 500)`.
  The store's `get_page` caps at 500 rows (`memory.rs:114`), so any dataset
  with >500 records was scored on its first 500 only.
- **Expected:** Preview sees the whole feed, same as the scheduler.
- **Root cause:** The scheduler paginates via `collect_all_records`
  (`scheduler.rs:105-128`) — a helper written precisely because a single
  `get_page` truncated. Its own doc comment (`scheduler.rs:99-104`) explains
  the rationale. `preview_signal` did not use that pattern; it took one page.
  A finding on record 600 (e.g. a HIBOR jump on the 600th day) would fire in
  production but not in preview — the "preview IS what will fire" invariant
  broken by truncation rather than by a dispatch mismatch.
- **Fix:** Added `collect_all_records_for_preview` (`signal.rs`), mirroring the
  scheduler's `collect_all_records`, and switched `preview_signal` to use it.
- **Verification:** new test `d020_preview_sees_records_beyond_first_page`
  (seeds 600 records with a jump on row 600; asserts preview fires). Passes.

## D-021 — `threshold_crossing` missing from agent tool belt

- **Stories:** F-043 (agent tool belt)
- **Severity:** medium — the flagship "HIBOR above X%" signal is unreachable
  from the LLM agent-loop tool surface (`POST /v1/ask` → `run_detector`).
- **Observed:** `run_detector` (`tools.rs`) with `detector: "threshold_crossing"`
  returned `Error::Internal("run_detector: unknown detector \`threshold_crossing\`")`.
  It hit the `other =>` catch-all.
- **Expected:** The tool belt dispatches every detector the scheduler does.
  `threshold_crossing` is wired into the scheduler (`scheduler.rs:321`) and the
  signal preview (`signal.rs:455`).
- **Root cause:** Same class as D-019 — three dispatch sites (scheduler /
  preview / tools), each hand-maintained. `threshold_crossing` was added to
  the scheduler (v7 wiring, P-102 prerequisite) and to preview (D-013 class),
  but the tool belt's match was not updated in parallel.
- **Fix:** Added a `threshold_crossing` arm to `run_detector` (`tools.rs`),
  mirroring the scheduler's direction handling (`"below"` → Below, else Above)
  and calling `detect_threshold_crossing`. Added `detect_threshold_crossing` +
  `CrossDirection` to the analysis import.
- **Verification:** new tests `d021_run_detector_threshold_crossing_works`
  (fires on a crossing) + `d021_run_detector_threshold_crossing_silent_when_not_crossed`
  + `d021_run_detector_threshold_crossing_below_direction`. All pass.

## D-022 — `funding` tab missing from boot hash-route whitelist

- **Stories:** F-067 (Funding & Credits page)
- **Severity:** low — a cold load of `#funding` lands on overview instead of
  the Funding tab.
- **Observed:** Visiting `https://app/#funding` on a fresh page load activates
  the overview tab, not funding. The `go('funding')` call is never made.
- **Expected:** A cold load of `#funding` activates the funding tab (same as
  every other tab).
- **Root cause:** `boot.js:7` whitelists the 7 original tabs for hash-route
  activation but `funding` (added in v10) was not added to the list:
  `['overview','datasets','divergence','signals','cases','health','licences']`.
- **Fix:** Added `'funding'` to the whitelist array.
- **Verification:** traced `boot.js` — `initTab` now matches `'funding'` →
  `go('funding')` → `renderFund()`. Manual: `#funding` cold load now activates
  the tab.

## D-023 — `signal_id` omitted detection-affecting fields (collision + overwrite)

- **Stories:** F-020 (create signal)
- **Severity:** low (risk) — two semantically distinct signals with the same
  owner collide and one silently overwrites the other on create.
- **Observed:** Creating a `series_jump` signal with `cadence=Daily` then
  another with `cadence=Quarterly` (all else equal) produces the **same id**
  → the second `SignalStore::create` (`BTreeMap::insert`) silently overwrites
  the first. Same for `comparison` (PoP vs YoY), `field_b` (correlation),
  `companion` source (cross-source detectors).
- **Expected:** Two signals that the scheduler would score differently get
  different ids.
- **Root cause:** `signal_id` (`signal.rs`) hashed only `(owner, source,
  dataset, detector, field, threshold, direction)`, omitting `cadence`,
  `comparison`, `field_b`, `companion`, `companion_field`, `join_field`. The
  cadence/comparison omission is most acute: D-006 fixed cadence/comparison
  at the *detection* level, but the *id* still treated them as identical, so
  the dedup undone the fix at the subscription level.
- **Fix:** Added the six omitted fields to the hash. `Cadence`/`Comparison`
  are `Eq` but not `Hash` (no derive on the enums) and `CompanionRef` is a
  struct in another crate, so the id hashes stable string slugs
  (`cadence_slug`/`comparison_slug`, mirroring the serde renames) and the
  companion's `(source, dataset)` with a presence tag, instead of the values.
- **Verification:** new tests `d023_different_cadence_yields_different_id` +
  `d023_different_comparison_yields_different_id` +
  `d023_different_field_b_yields_different_id` +
  `d023_different_companion_yields_different_id` +
  `d023_identical_targets_still_dedup` (regression guard). All pass.

## D-024 — `series_jump` default-threshold magic literal `25.0` undocumented

- **Stories:** F-041–F-043 (scheduler / preview / tool belt dispatch)
- **Severity:** low — no behavior bug, but an undocumented magic literal drifts
  across three dispatch sites while the detector fn's own default
  (`DEFAULT_PCT_THRESHOLD = 15.0`) is a named constant.
- **Observed:** The scheduler (`scheduler.rs:251`), signal preview
  (`signal.rs:490`), and tool belt (`tools.rs:558`) each carried a bare `25.0`
  for the `series_jump` no-threshold default. The cadenced detector's own
  fallback (`analysis.rs:618`) is `DEFAULT_PCT_THRESHOLD = 15.0`.
- **Expected:** The dispatch-level default is a named, documented constant,
  not a drifting literal, so the intentional distinction (25 = watch
  sensitivity; 15 = scan sensitivity) is auditable.
- **Root cause:** The literal was copy-pasted across the three sites without a
  named constant; nothing documented why it differs from the detector default.
- **Fix:** Added `DEFAULT_SERIES_JUMP_WATCH_PCT = 25.0` (`analysis.rs`) with a
  doc comment explaining the distinction, and replaced all three literals.
  **No behavior change** — the value stays 25.0; it is now named and centralized.
- **Verification:** existing D-006 regression tests (which assert 25.0 behavior)
  still pass unchanged; `cargo clippy -D warnings` clean.

---

## D-025 — `/ask` heuristic matcher returns wrong dataset on stop-word contamination

- **Stories:** F-056 (`POST /v1/ask` heuristic mode)
- **Severity:** high — the primary zero-config Q&A path returns the wrong
  dataset for the most natural phrasings
- **Observed (live, 192-dataset catalog):**
  - `POST /ask {"question":"what is the interbank liquidity?"}` → returned
    `hotlines-auth-retailbanks-rep` (29 records) instead of the interbank
    dataset (1000 records). Confidence 0.5 (a "match").
  - `POST /ask {"question":"what is hibor?"}` → returned `list-of-cmu-members`
    instead of any interbank dataset.
- **Expected:** the dataset whose title/name/source actually contains the
  question's content tokens wins.
- **Root cause:** `heuristic_answer` (`qa.rs:23`) scored every dataset by
  counting how many **individual** question tokens appear as substrings in
  `source+dataset+title`. With ~190 datasets, stop-words ("what", "is", "the")
  matched many unrelated titles and tied the real match; the tie-break
  (`sort_by_key` on `Reverse(score)` — unstable for equal keys) then picked
  whichever dataset came first in the list. The unit test passed only because
  it used a 1-dataset store where no tie was possible.
- **Fix:** (1) strip English stop-words + tokens ≤2 chars before scoring
  (`qa.rs` `STOP_WORDS` + `is_stop_word`); (2) add `tags` to the haystack so a
  domain term present only in tags (e.g. `hibor`) still matches.
- **Verification:** live — `what is the interbank liquidity?` now returns the
  interbank dataset; `hibor` now matches via tags. Unit tests
  `d025_stopwords_do_not_let_distractor_win` (multi-dataset, distractor with
  stop-word "the") + `d025_is_stop_word_filters_common_words`.

## D-026 — data.gov.hk: 20 resources return 422 "Not a valid resource" (upstream de-registration)

- **Stories:** F-098 (data.gov.hk connector), F-005 (catalog)
- **Severity:** critical — the documented flagship dataset
  (`money-lenders-licensees`, F-031 in the old tracker) is dead, and 19 others
  with it; they registered as 0-record ghosts polluting `/v1/sources`.
- **Observed (live boot log):** `ingest: fetch failed source=datagovhk
  dataset="money-lenders-licensees" error=upstream error for datagovhk: 422:
  {"code":"422","message":"Not a valid resource"}` — for 20 of the 33
  registered datagovhk resources. Direct probes against
  `api.data.gov.hk/v2/filter` confirmed each returns 422 (the platform
  de-registered the PSI URLs).
- **Expected:** the catalog lists only resources that actually return data.
- **Root cause:** upstream drift — the data.gov.hk platform de-registered 20
  PSI resource URLs. The connector table (`datagovhk.rs RESOURCES`) still
  listed them, so they registered eagerly and sat at 0 records forever.
- **Fix:** removed the 20 confirmed-dead resources from `RESOURCES`, keeping
  the 12 probe-verified-alive ones. Updated the `resource_table_is_well_formed`
  minimum (30 → 12) and replaced `money_lenders_resource_preserved` with
  `d026_dead_resources_removed` (a guard that the dead slugs stay out).
- **Verification:** live — `/v1/sources?source=datagovhk` now lists 12
  datasets, all with records >0; no dead slug present.

## D-027 — Dashboard `DEFAULT_API_BASE` hardcoded to a specific Railway URL

- **Stories:** F-112 (Base URL + API key config)
- **Severity:** high — a self-served `/dashboard` deploy silently points the
  browser at a *different* host's API.
- **Observed:** `dashboard/api.js` had
  `const DEFAULT_API_BASE = 'https://hkgov-rethink-production.up.railway.app';`
  and `boot.js` only auto-filled the base when `location.port` was truthy.
  Production deploys on standard ports (80/443) have an empty `location.port`,
  so the auto-fill was skipped and the hardcoded Railway URL was used —
  meaning a local/Docker/Railway-direct dashboard fetched data from the remote
  production API.
- **Expected:** a dashboard served by hkgov-api uses same-origin by default.
- **Fix:** `DEFAULT_API_BASE = ''` (same-origin fallback); `boot.js` now
  auto-fills the page's own origin for any http(s) page (no port requirement).
  A split deploy still overrides via the header input / localStorage.
- **Verification:** served `/api.js` shows empty default; `/boot.js` fills
  origin unconditionally.

## D-028 — Dashboard per-user tabs show misleading "empty" on a 401

- **Stories:** F-141 (signals list), F-143 (cases list)
- **Severity:** low (UX) — the tabs read "no signals yet" / "no cases yet"
  even when the real reason is an unauthenticated session.
- **Observed:** `loadSignals`/`loadCases` treated any `getJSON` error
  (including a 401) as "no data" and rendered the empty-state. A user with no
  session saw "no signals yet — create one above" even though the create
  action would also 401.
- **Expected:** a 401 surfaces a "sign in to view…" message distinct from a
  genuinely empty list.
- **Fix:** both functions now check `list.__error === 401` first and render
  `auth_needed_signals` / `auth_needed_cases` (added to i18n, EN + zh-HK).
- **Verification:** served `/features.js` shows the 401 branch; i18n keys
  present (4).

## D-029 — `record_count` derived from the TTL-bound cache → catalog goes to 0 after TTL

- **Stories:** F-005 (`/v1/sources`), F-024 (silence index), F-004 (`/ready`)
- **Severity:** critical — the entire catalog, the flagship silence index, and
  the readiness probe all depend on `record_count`, which silently became 0
  for every dataset 600s after boot (and stayed 0 until the next 24h refresh
  for datagovhk).
- **Observed (live):** `/v1/sources` showed 13 datagovhk datasets with records
  immediately after warm; ~10 min later (TTL expiry) the same query showed 0
  records for all of them. `is_degraded` (which gates `/ready`) would flip to
  degraded even though the data had been fetched successfully.
- **Root cause:** `MemoryStore::meta`/`list` (`memory.rs`) derived
  `record_count` live from `self.records.get(&id)` — the moka cache, which
  evicts entries after `ttl_secs` (default 600s). The registry (which holds
  titles/categories/tags and is NOT TTL-bound) did not store the count, so
  eviction reset the count to 0.
- **Fix:** added `last_record_count` + `last_refreshed_at` to `RegisteredMeta`
  (persisted in the registry, survives TTL eviction); `put_dataset` updates
  them atomically; `meta`/`list` prefer the live cache count but fall back to
  the persisted count when the entry has been evicted. `register` preserves
  them across a re-register (ingest re-registers on each boot).
- **Verification:** unit test `record_count_survives_cache_ttl_eviction`
  (1s TTL + invalidate → count still 3). Live: catalog stable across a TTL
  cycle.

## D-030 — Flaky `persist::tests::debounced_snapshot_coalesces`

- **Stories:** F-155 (store persistence)
- **Severity:** medium (test reliability) — the test failed intermittently
  under concurrent test load on Windows, breaking CI confidence.
- **Observed:** `panicked at crates\agent\src\persist.rs:250:14: snapshot was
  written` — the debounced write did not land within the 300ms wait.
- **Root cause:** the test armed 10 schedules over ~20ms, then waited 300ms
  for a 100ms-debounce write to complete. Under concurrent test load the tokio
  scheduler could delay the debounce task past the 300ms window.
- **Fix:** widened the post-arm wait from 300ms to 1000ms — a wide margin
  that keeps the test fast (~1s) while tolerating scheduler latency.
- **Verification:** the test passes consistently in isolation and under full
  workspace concurrency.

---

> **Fourth independent QA cycle (D-031 → D-034).** A fresh end-to-end pass was
> run with the live server (172 sources, 500 insights, all 7 connectors warm)
> and a Playwright headless dashboard harness. It re-verified every prior fix
> still held, then enumerated every user-facing story against the running
> binary + the rendered dashboard. Four new defects surfaced — one critical
> availability regression in the records path (D-031), one silent-failure in
> signal preview (D-032), the long-standing missing dashboard auth UI (D-033,
> formerly waived as D-018), and a tracker-doc mismatch (D-034). All four are
> fixed below; full per-story trace in `phase2_results.json` (regenerated each
> cycle, gitignored).

## D-031 — Records cache TTL (600s) shorter than every refresh interval → `/records`, `/cite`, `/unprecedentedness` 502 for ~97% of each cycle

- **Stories:** F-015 (records), F-029 (unprecedentedness), F-033 (cite bundle),
  F-073 (evidence pointers), F-124 (timeline), F-125 (comparator), F-130
  (evidence rendered), F-132 (cite drawer), F-137 (divergence explorer),
  F-139 (signal preview), F-144 (investigation Q&A) — every read path that
  touches the record cache.
- **Severity:** critical — the flagship Cite-It reproducibility manifest
  (`/cite`) and the unprecedentedness comparator were unreachable for most of
  each refresh cycle, and the dashboard's record drill-down/cite drawer
  showed a blank or 502 toast. This is the highest-impact defect found this
  cycle: it silently invalidated the project's "evidence you can verify"
  promise whenever a reader happened to land in the eviction window.
- **Observed (live):** immediately after warm, `/v1/datasets/hkma/daily-
  figures-interbank-liquidity/records?limit=5` → 200 (total 1000). ~10 minutes
  later (moka TTL 600s elapsed, dataset refresh 3600s) the same call →
  `502 {"error":{"kind":"store","message":"internal server error"}}`. A
  15-dataset random sample returned 502 for **all 15**. `/cite` and
  `/unprecedentedness` were 502 too. The headless dashboard captured 224
  console 502s during a single load (the comparator fires one per clicked
  value). The dataset's *metadata* still reported `record_count: 1000`
  (D-029 fix held for the catalog) — only the records themselves were gone.
- **Root cause:** the record vectors live in a `moka::Cache` with
  `time_to_live(600s)` (`memory.rs:65`), but each dataset's ingest refresh
  interval is far longer — 1800s (1), 3600s (11), 21600s/6h (125, the HKMA
  majority), 86400s (32), 604800s/7d (3). So records evaporated ~10 min
  after each refresh and stayed gone until the next one: up to 50 min for the
  flagship interbank dataset, ~5h50m for most HKMA feeds, ~7d for a few. The
  D-029 fix made the catalog's `record_count` survive this gap (by persisting
  the count in the non-TTL'd registry) but left the records path returning
  `Error::Store` → 502 for the same gap. Staleness was already bounded by the
  refresh interval; the TTL added nothing but an availability hole.
- **Fix (two-part):**
  1. **Config root cause.** `ttl_secs` default changed 600 → 0
     (`config.rs::CacheSettings`, `config.toml`, `MemoryStore::new`). 0 now
     means "no time-based eviction" — the `time_to_live` builder call is
     skipped entirely when `ttl_secs == 0` (moka treats `Some(Duration::ZERO)`
     ambiguously, so the build-time skip is the safe formulation). Records
     stay resident until the ingest supervisor refreshes them on schedule, or
     `max_entries` (200k) evicts by LRU under pressure. This is the invariant
     the system always wanted: staleness bounded by refresh cadence, memory
     bounded by capacity.
  2. **Defense-in-depth.** `get_page` now returns an honest empty page
     (`total:0, records:[]`) on a cold cache instead of `Error::Store` → 502,
     so fresh-boot, LRU pressure, or an operator-set non-zero TTL can't crash
     browse-oriented callers. `get_by_ids` (the cite manifest path) keeps an
     error because an empty evidence set would produce a *wrong* manifest hash
     — but it now returns the new `Error::StoreUnavailable` (mapped to a
     retryable **503**, not 502) with a clear client message, so the cite
     drawer can say "data temporarily unavailable, retry" instead of showing
     an internal-error toast. `/unprecedentedness` likewise surfaces 503 when
     the cache is cold for a dataset the registry reports as having data
     (rather than silently scoring against empty history and producing a
     misleading "no historical data" band). The dashboard's cite drawer,
     comparator, and signal preview all render a "data temporarily
     unavailable — retry shortly" notice on 503.
- **Verification (Phase 4):**
  - Unit: `ttl_zero_disables_time_based_eviction`,
    `get_page_returns_empty_page_on_cold_cache`,
    `get_by_ids_errors_on_cold_cache` (store), and
    `unprecedentedness_cold_cache_returns_store_unavailable` (api) — 4 new
    regression tests.
  - Live: records endpoint stayed 200/total=1000 across +60s/+90s/+120s
    probes (the prior build 502'd at +600s); `/cite` and `/unprecedentedness`
    200 immediately after warm and remain 200; a fresh cold-cache dataset
    returns a 200 empty page (not 502).
  - Dashboard: the 224 console 502s observed in Phase 2 dropped to 0 in
    Phase 4 (`F-NO-CONSOLE-ERRORS` passes).

## D-032 — Signal preview silently returns `count:0` when the records cache is cold

- **Stories:** F-043 (signal preview)
- **Severity:** medium — a user previewing a signal during the eviction window
  saw "0 findings over 90 days" and reasonably concluded the signal would
  never fire, when in fact the detector had no data to evaluate.
- **Observed (live):** during the D-031 eviction window,
  `POST /v1/signals/preview` returned `200 {count:0, findings:[]}` for a
  `series_jump` watch that demonstrably fires (it returned 22 findings once
  records were resident). The `collect_all_records_for_preview` loop `break`ed
  on the first `get_page` error and the detector ran over an empty vector.
- **Root cause:** graceful degradation hiding a real problem — the preview
  loader treated any `get_page` error as "end of pagination" and returned
  whatever it had (nothing), so the detector scored zero records and reported
  zero findings with no flag that data was missing.
- **Fix:** `SignalPreview` carries a new `data_available: bool` field
  (defaults `true` for back-compat). `collect_all_records_for_preview` now
  returns `(records, data_available)`; when the collected set is empty it
  checks the registry's persisted `record_count` — if the dataset *should*
  have records, `data_available=false`. The dashboard's `previewSignal` shows
  "data temporarily unavailable — retry shortly to see real findings" when
  `count===0 && data_available===false`, instead of the misleading "0
  findings". (With D-031 fixed, the cold-cache case is now rare — this is the
  honest signal for when it does occur, e.g. fresh boot before first fetch.)
- **Verification (Phase 4):** live `POST /v1/signals/preview` response now
  includes `data_available:true` when records are resident; the dashboard
  harness confirms the preview renders findings. The cold-cache branch is
  unit-covered by the `data_available` logic + the D-031 store regressions.

## D-033 — Dashboard has no authentication UI (formerly D-018, waived)

- **Stories:** F-123 (watch silence index), F-139–F-144 (signals builder,
  list, toggle, delete, dispatch log; cases list, investigation workspace).
  Every per-user dashboard behaviour was unreachable from the browser.
- **Severity:** high (UX) — the per-user tier is a documented v1 feature with
  full API + Python-client support, but the dashboard — the primary human
  surface — had no way to obtain a session. The prior copy literally
  instructed users to "POST /v1/auth/request-token, then /v1/auth/redeem,
  then send the session as a Bearer header" by hand. D-018 documented this
  and waived it as "missing feature, not a smallest-fix defect"; this cycle's
  mandate ("fix every logistical error or ux error") puts it in scope.
- **Observed (live):** every per-user tab showed the auth-needed empty state;
  the signal builder's Save button alerted a generic error on the 401; the
  silence-index Watch button 401'd silently. A user could not, from the
  browser, save a single signal or open a single case.
- **Fix:** an in-page magic-link sign-in flow.
  - `dashboard/api.js`: `getJSON`/`postJSON`/`fetchText` now attach
    `Authorization: Bearer {session}` from `sessionStorage` (or `localStorage`
    for the pasted-link path) when a session is present.
  - `dashboard/index.html`: a `Sign in` badge in the header opens a modal
    with an email field + "Send sign-in link" button. When the server returns
    the token inline (`dev_return_auth_token`, dev/CI), it auto-redeems; in
    production (token delivered out-of-band) a manual paste field accepts
    either a bare token or the full magic-link URL. A signed-in state shows
    the user's email + a Sign-out button.
  - `dashboard/features.js`: `openAuth`/`sendAuthLink`/`redeemAuthToken`/
    `signOut`/`refreshAuthModal`; `saveSignal` now opens the auth modal on
    401 instead of a generic alert; the per-user empty states gained a "Sign
    in" button next to the message.
  - `dashboard/i18n.js`: full EN + zh-HK strings for the auth flow.
- **Verification (Phase 4):** the headless dashboard harness drives the full
  flow end-to-end — open modal → enter email → send link → auto-redeem →
  header flips to the user's email → signals tab loads without the auth wall.
  A separate browser test created a real signal via the builder post-auth
  (`POST /v1/signals` → 200; re-GET shows the created signal), proving the
  entire per-user write path is now reachable from the dashboard. Four new
  harness checks (`F-033-auth-button`, `-modal-opens`, `-signin-flow`,
  `-signals-load`) all pass.

## D-034 — FEATURES_TRACKER F-111 spec claimed hash format `#page-<tab>`; actual is `#<tab>`

- **Stories:** F-111 (hash routing + tabs) — documentation/external-link
  guidance only; no code behaviour change.
- **Severity:** low — anyone copying the documented hash format into a
  bookmark, external link, or the `boot.js` init-tab whitelist would get a
  no-op (the dashboard would load overview instead of the intended tab).
- **Observed (live):** clicking the Divergences tab sets `location.hash` to
  `#divergence`, not `#page-divergence`. The tracker row F-111 asserted the
  latter. `boot.js:15-16` reads `location.hash.replace('#','')` and matches
  against bare tab names, confirming `#<tab>` is the intended and implemented
  format. The tracker was simply wrong.
- **Fix:** tracker row F-111 corrected to `#<tab>` (see FEATURES_TRACKER.md).
  No code change — the code was already self-consistent. The Phase 4 harness
  was written to assert the *actual* format and passes.



