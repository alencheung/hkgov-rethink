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
