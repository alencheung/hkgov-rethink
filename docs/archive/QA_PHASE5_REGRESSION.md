# Phase 5 — Regression Report

> Re-simulation of every Phase-2/3 test case (T001–T168) against the **post-fix**
> code, plus re-verification that each Phase-4 fix resolves its defect and that
> no previously-working feature broke.

## Mechanical gates (all green)

| Gate | Command | Result |
|------|---------|--------|
| Workspace tests | `cargo test --workspace` | ✅ **189 passed, 0 failed** (was 178 baseline; +11 new regression tests) |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | ✅ clean (no warnings) |
| Format | `cargo fmt --all -- --check` | ✅ clean |
| Python client | `pytest tests/` | ✅ **14 passed, 0 failed** (untouched, no regression) |

The +11 tests are the D-006 (×4), D-007 (×4), D-010 (×3) regression guards added in Phase 4.

## Fix re-verification (each defect → resolved?)

### D-006 (high) — series_jump preview ≠ production → ✅ RESOLVED

**Repro re-run:** the fix swapped `signal.rs:run_detector_preview`'s `series_jump` arm from `detect_series_jumps` (unscaled) to `detect_series_jumps_cadenced` (passing `target.cadence`), and added the missing `year_over_year` arm. The 4 new tests assert preview==production directly:

| Test | Input | Preview | Production (cadenced) | Match? |
|------|-------|---------|----------------------|--------|
| `d006_quarterly_series_jump_preview_matches_production` | +35%, 25% base, quarterly | 0 | 0 (eff. 43.3%) | ✅ |
| `d006_monthly_series_jump_preview_matches_production` | +30%, 25% base, monthly | 1 | 1 (eff. 25%) | ✅ |
| `d006_unknown_cadence_preview_matches_production` | +30%, 25% base, unknown | 1 | 1 (no scaling) | ✅ |
| `d006_yoy_series_jump_preview_runs` | +50% YoY, quarterly, 8 recs | 1 (`year_over_year`) | 1 | ✅ |

The original throwaway proof binary (run in Phase 3) showed preview=1/prod=0 divergence; the fix makes them agree. **Resolved.**

### D-007 (low) — bad `?since=` silent fallback → ✅ RESOLVED

**Repro re-run:** `list_insights` (`routes.rs`) now returns `Err(ApiError(BadRequest(...)))` when `since` is present but unparseable, instead of falling through to the full list. 4 tests lock it:
- `d007_bad_since_returns_400` — `"banana"` → 400 ✅
- `d007_valid_rfc3339_since_still_works` — RFC3339 → Ok ✅
- `d007_epoch_seconds_since_still_works` — epoch secs → Ok ✅
- `d007_no_since_still_works` — absent → Ok ✅

**Resolved.**

### D-008 (low) — cite `base_url` doc/behaviour mismatch → ✅ RESOLVED (doc)

The docstring on `CiteQuery::base_url` (`routes.rs`) no longer claims the `Host` header is consulted; it now documents that the caller must pass the public origin explicitly (with a deployment note for proxy setups). Behaviour unchanged by design (approved at Phase-3 checkpoint). **Resolved.**

### D-010 (med) — sessions never expire → ✅ RESOLVED

`Session` now carries `expires_at` (default far-future for legacy blobs); `redeem_token` sets it to `now + 30 days`; `lookup_session` rejects `now > expires_at`. 3 tests:
- `d010_fresh_session_has_future_expiry_and_resolves` — minted session expires ~30d out ✅
- `d010_expired_session_resolves_to_none` — back-dated session → None ✅
- `d010_far_future_default_keeps_legacy_sessions_alive` — back-compat ✅

**Resolved.**

### D-009 (risk) — no owner isolation → ⚠️ WAIVED (documented)

Per approval, waived for v1 as the documented "shared-key trust model". A loud `⚠️ D-009` note was added to `routes.rs` at the signals section explaining the gap and the remediation path (derive owner from session, reject cross-owner mutations) for any future multi-tenant deployment. **Waived with reason; documented.**

### D-011 (low) — Python client coverage gap → ⏸️ DEFERRED

Per approval, deferred to a separate Python-scoped task (different language/toolchain). The 8 missing endpoint families (signals, investigations, auth, cite, silence-index, unprecedentedness, insight-history, since/lang) remain callable via `_get`/`_post`. Tracked, not fixed in this cycle.

## No-regression re-trace (168 test cases)

The full Phase-2/3 suite (T001–T168) was re-traced against the post-fix code. The only behavioural changes are the four intentional ones above; everything else is unchanged. Summary by domain (delta from Phase 3 in parentheses):

| Domain | Tests | ✅ | ⚠️ | ❌ | Note |
|--------|------:|---:|---:|---:|------|
| A. System & health | 11 | 11 | 0 | 0 | unchanged |
| B. Data catalog & records | 25 | 23 | 2 | 0 | unchanged (T033/T034 still minor latent) |
| C. Insights & product layer | 33 | 31 | 0 | 0 | **T043 (D-007) ❌→✅, T069 (D-008) ❌→✅** |
| D. Flagship scores | 15 | 15 | 0 | 0 | unchanged |
| E. Signals (P-102) | 19 | 18 | 1 | 0 | **T095 (D-006) ❌→✅; T090 (D-009) ⚠️ waived** |
| F. Investigations (P-105) | 12 | 11 | 1 | 0 | T107 (D-009) ⚠️ waived |
| G. Identity (P-108) | 11 | 10 | 1 | 0 | **T126 (D-010) ❌→✅**; T119 still minor latent |
| H. Q&A | 6 | 6 | 0 | 0 | unchanged |
| I. Config & deploy | 12 | 12 | 0 | 0 | unchanged |
| J. Frontend dashboard | 15 | 15 | 0 | 0 | unchanged (D-002 still fixed) |
| K. Python client | 9 | 8 | 0 | 1 | T168 (D-011) deferred |
| **Total** | **168** | **160** | **5** | **1** | was 155/7/6 |

## End-to-end user journeys (re-run)

Each journey traced through the full handler→store→response path on the post-fix code:

1. **Anonymous → dashboard → see insights** (F-004→F-043→F-010): `/dashboard` 200, `loadBrief` renders flattened items, `/v1/insights` returns array. ✅
2. **Operator enables agent → brief populates** (F-038→F-014): scheduler runs `default_scan_targets`, frames findings, upserts insights, brief ranks them. ✅ (D-006 fix does not touch this path — scheduler already used cadenced detection.)
3. **User authors + previews a signal** (F-021→F-023): create signal, preview now matches what the scheduler will fire for `series_jump`/`threshold_crossing`/`outlier`/`seasonality`/`year_over_year`. ✅ **(this is the journey D-006 broke — now fixed)**
4. **Magic-link identity flow** (F-033→F-034→F-035): request token → redeem → `/auth/me` resolves; session now expires in 30d. ✅ **(D-010 hardens this journey)**
5. **Cite an insight** (F-017): cite bundle + manifest deterministic; `base_url` documented behaviour matches code. ✅
6. **Auth gate** (D-005): key-enabled instance, health/dashboard exempt, data routes 401 without key. ✅
7. **`/v1/insights?since=` filtering** (F-011): valid since filters; bad since now 400. ✅ **(D-007 hardens this journey)**

## Phase 5 exit criteria

- [x] All tests pass (189 Rust + 14 Python, 0 fail).
- [x] No open critical or high-severity defects (D-006 high → resolved; D-010 med → resolved).
- [x] No broken user journeys (7/7 re-traced green).
- [x] D-001..D-005 (prior audit) all still fixed — their 22 guards are in the passing set.

## Remaining open items (carried to Phase 6)

- **D-011** (low): Python client coverage gap — deferred to a separate Python task.
- **5 latent ⚠️** (T033/T034 pagination hardening, T074 silence bad-period silent fallback, T119 empty-email provisioning, T090/T107 D-009 waived risk) — all low-severity, non-blocking, documented.
