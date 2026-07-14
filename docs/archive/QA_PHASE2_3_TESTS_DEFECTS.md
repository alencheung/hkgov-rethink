# Phases 2 & 3 — Test Suites + Simulated Execution

> Phase 2 expands each Phase-1 feature into concrete test scenarios (happy /
> error / boundary / security / state). Phase 3 traces each scenario against
> the actual code and records the verdict. Defects found are logged as
> `D-006+` (D-001..D-005 were the prior audit's, all re-verified fixed).
>
> **Method:** every "Verdict" below is reached by reading the cited source
> lines, not by assumption. Where a test would require a live server, the
> behaviour is traced through the handler → store → response path and cross-
> checked against the existing unit/integration test that already exercises it
> (cited as `(guarded by <test>)`).

Legend: ✅ PASS · ❌ FAIL (defect) · ⚠️ LATENT (passes today but fragile/wrong-by-design)

---

## A. System & health (F-001..F-004)

| T# | Feature | Scenario | Trace | Verdict |
|----|---------|----------|-------|---------|
| T001 | F-001 | GET /health, no key | `routes.rs:191` → 200 `{status:"ok"}`; `auth.rs:34` exempts `/health` | ✅ (guarded by `health_paths_exempt_without_key`) |
| T002 | F-001 | GET /health, key enabled, no key sent | same exemption | ✅ |
| T003 | F-001 | empty-prefix mode, GET /health at root | `routes.rs:107` merge brings `/health` to root | ✅ (guarded by `empty_prefix_mounts_all_routes_at_root`) |
| T004 | F-001 | GET /health.xml (suffix) w/ key enabled, no key | `auth.rs:34` exact-match → not exempt → 401 | ✅ (D-005 regression: exact path only) |
| T005 | F-002 | GET /v1/health/sources | `routes.rs:204` → `breaker_states()` array | ✅ |
| T006 | F-002 | empty-prefix → /health/sources at root | merge path | ✅ |
| T007 | F-002 | no sources registered | returns `[]` (breaker_states over empty registry) | ✅ |
| T008 | F-003 | GET / | `routes.rs:133` advertises `/ask` + `/dashboard` | ✅ (guarded by `root_lists_ask_endpoint`) |
| T009 | F-004 | GET /dashboard | `routes.rs:176` → 200 text/html, `include_str!` body | ✅ |
| T010 | F-004 | GET /dashboard/ (trailing slash) | separate route `routes.rs:104` | ✅ |
| T011 | F-004 | key enabled, GET /dashboard no key | root-level route, **outside** the keyed `api_routes` nest → exempt | ✅ (guarded by `health_paths_exempt_without_key` family) |

**A verdict: 11/11 ✅. No defects.**

## B. Data catalog & records (F-005..F-009)

| T# | Feature | Scenario | Trace | Verdict |
|----|---------|----------|-------|---------|
| T012 | F-005 | GET /v1/sources, no filter | `routes.rs:300` → `store.list(None)` | ✅ (guarded by `sources_returns_all_when_no_filter`) |
| T013 | F-005 | empty store | `list` → `[]` | ✅ |
| T014 | F-005 | limit param | no limit on /sources (returns all) — only /insights & /records paginate | ✅ (by design) |
| T015 | F-006 | ?tag=hibor (single) | `routes.rs:248` `tags()` parses raw query → AND-retains | ✅ (guarded by `sources_filters_by_tag`) |
| T016 | F-006 | ?tag=a&tag=b (repeated) | `tags()` splits on `&` | ✅ (`sources_tag_matches_any_repeated`) |
| T017 | F-006 | ?tag=a,b (comma) | `tags()` splits on `,` | ✅ (`sources_tag_matches_any_comma`) |
| T018 | F-006 | ?category=monetary | `dataset_matches` `Category::parse` | ✅ (`sources_filters_by_category`) |
| T019 | F-006 | ?category=NONSENSE | `Category::parse` → None → `dataset_matches` returns false → empty | ✅ (`sources_invalid_category_returns_empty`) |
| T020 | F-006 | ?cadence=monthly | `Cadence::parse` + compare | ✅ (`sources_filters_by_cadence`) |
| T021 | F-006 | ?cadence=garbage | `parse`→None → `want.is_none()` true → excluded → empty | ✅ |
| T022 | F-006 | ?q=interbank | case-insensitive substring over title+desc+id | ✅ (`sources_free_text_search`) |
| T023 | F-006 | category AND cadence compose | both filters applied | ✅ (`sources_composes_filters`) |
| T024 | F-006 | ?source=hkma | `DataSource::parse` → `store.list(Some(Hkma))` | ✅ |
| T025 | F-007 | GET /v1/categories | groups by category, counts, sorted datasets | ✅ (`categories_groups_with_counts`) |
| T026 | F-007 | empty store → `[]` | BTreeMap empty | ✅ |
| T027 | F-008 | GET /v1/datasets/hkma/daily-interbank-liquidity | 200 `Option<Some(meta)>` | ✅ |
| T028 | F-008 | unknown dataset (valid source) | 200 `null` (**not** 404) — `meta` returns `Ok(None)` → `Json(None)` | ✅ (documented behaviour) |
| T029 | F-008 | unknown source /v1/datasets/zzz/x | `parse_source` → `UnknownSource` → 404 | ✅ |
| T030 | F-008 | dataset literally named `health` w/ key on, no key | `auth.rs:34` exact `/health` only → `/v1/datasets/hkma/health` is NOT exempt → 401 | ✅ (guarded by `dataset_route_named_health_requires_key`) |
| T031 | F-009 | records w/ offset+limit | `get_page` | ✅ |
| T032 | F-009 | offset > total | empty records, correct `total` | ✅ (RecordPage contract) |
| T033 | F-009 | limit=0 | `PageQuery.limit` default=100 applies only when absent; **limit=0 → `get_page(..,0,0)`** → memory store returns 0 records | ⚠️ returns empty silently rather than 400; minor |
| T034 | F-009 | huge limit (e.g. 100000) | no upper clamp on /records (`PageQuery` has no max) → store returns up to total | ⚠️ no DoS guard; minor |
| T035 | F-009 | unknown source | 404 | ✅ |
| T036 | F-009 | unknown dataset | store miss → 502 `Store("no records cached…")` | ✅ (documented; the 502 is intentional for cache-miss) |

**B verdict: 23/25 ✅, 2 ⚠️ (T033/T034 — minor pagination hardening, non-blocking). No defects.**

## C. Insights & product layer (F-010..F-017)

| T# | Feature | Scenario | Trace | Verdict |
|----|---------|----------|-------|---------|
| T037 | F-010 | GET /v1/insights?limit=10 | `insights.list(10)` newest-first (BTreeMap rev) | ✅ |
| T038 | F-010 | empty store → `[]` | ✅ |
| T039 | F-010 | limit huge (e.g. 99999) | `list` takes usize; no clamp → returns all | ⚠️ minor (no upper bound; store is small in v1) |
| T040 | F-010 | limit=0 | returns 0 insights (`.take(0)`) — silent empty, not an error | ⚠️ minor |
| T041 | F-011 | ?since=2026-06-01T00:00:00Z (RFC3339) | `parse_since` ok → `list_since` | ✅ |
| T042 | F-011 | ?since=1717200000 (epoch secs) | `parse_since` fallback branch ok | ✅ |
| T043 | F-011 | ?since=garbage | `parse_since`→Err → **silently falls back to unfiltered `list`** — no 400, no warning | ❌ **D-007** (see below) |
| T044 | F-011 | ?since=future | filters everything out → `[]` | ✅ (correct, if surprising) |
| T045 | F-011 | evolution.to_generated_at > since but first_seen < since | included (OR clause `insight.rs:262`) | ✅ |
| T046 | F-012 | ?lang=zh-HK on series_jump insight | `select_summary` → `frame_zh_hk` w/ pct | ✅ |
| T047 | F-012 | ?lang unset | English stored summary | ✅ |
| T048 | F-012 | ?lang=en (explicit) | English (only zh-Hk triggers reframing, `routes.rs:411`) | ✅ |
| T049 | F-012 | ?lang=zh-HK on unknown kind | falls back to English summary (`bilingual.rs:115`) | ✅ (guarded by `unknown_kind_falls_back_to_english`) |
| T050 | F-013 | GET history for evolved insight | `history(id,50)` newest-first | ✅ (guarded by `history_retains_prior_versions`) |
| T051 | F-013 | unknown id → `[]` | ✅ |
| T052 | F-013 | v1 insight (never evolved) → `[]` | ✅ |
| T053 | F-014 | GET /v1/brief?limit=5 | ranked, flattened items | ✅ |
| T054 | F-014 | empty store → `items:[]` | ✅ (`brief_empty_when_no_insights`) |
| T055 | F-014 | limit=0 | `build_brief` clamps to 1 (`brief.rs:41`) | ✅ (`brief_respects_limit` spirit) |
| T056 | F-014 | limit=100 | clamps to 50 | ✅ |
| T057 | F-014 | experimental discounted | ×0.7 (`brief.rs:100`) | ✅ (`brief_discounts_experimental`) |
| T058 | F-015 | POST feedback useful=true | 200 `{recorded:true}` | ✅ |
| T059 | F-015 | body missing `useful` | axum `Json` deserde → 422 (axum default for missing required field) | ✅ |
| T060 | F-015 | idempotent (no dedup) | each POST appends a row (`insight.rs:367`) | ✅ (documented) |
| T061 | F-016 | GET feedback net | `net_useful` up−down | ✅ |
| T062 | F-017 | GET cite JSON bundle | permalink + manifest w/ 64-char sha256 | ✅ (`cite_returns_bundle_with_manifest`) |
| T063 | F-017 | ?format=bibtex | text/plain `@misc{...}` | ✅ (`cite_renders_format_as_text`) |
| T064 | F-017 | ?format=notreal | 400 BadRequest | ✅ (`cite_bad_format_400s`) |
| T065 | F-017 | unknown insight id | 404 NotFound | ✅ (`cite_unknown_insight_404s`) |
| T066 | F-017 | deterministic (same insight+records) | byte-identical | ✅ (`cite_manifest_is_deterministic`) |
| T067 | F-017 | ?base_url=https://x | permalink starts with `https://x/cite/` | ✅ |
| T068 | F-017 | no base_url, no Host header | falls back to hardcoded `http://localhost:8080` (`routes.rs:522`) | ✅ (works) |
| T069 | F-017 | no base_url **but** request has `Host: api.example.com` | docstring (`routes.rs:502-504`) claims Host is used; **code ignores it** → permalink uses `localhost:8080` | ❌ **D-008** (doc/behaviour mismatch — see below) |

**C verdict: 28/33 ✅, 2 ⚠️, 2 ❌ (D-007, D-008).**

## D. Flagship scores (F-018..F-020)

| T# | Feature | Scenario | Trace | Verdict |
|----|---------|----------|-------|---------|
| T070 | F-018 | GET /v1/silence-index?period=2026-Q2 | HKMA-scoped score >0 when gaps exist | ✅ (`silence_index_returns_versioned_hkma_scoped_score`) |
| T071 | F-018 | no insights | score 0, events 0 | ✅ (`silence_index_empty_when_no_insights`) |
| T072 | F-018 | non-HKMA insights only | excluded (COVERED_SOURCE=Hkma) → score 0 | ✅ |
| T073 | F-018 | empty period → all history | `period=""` → derive falls back | ✅ |
| T074 | F-018 | bad period (e.g. "banana") | quarter-parse fails → treated as empty → all history (no 400) | ⚠️ silent fallback (consistent with T043 pattern) |
| T075 | F-018 | methodology_version present | `"1.0"` | ✅ |
| T076 | F-019 | unprecedentedness, spike value | is_unprecedented, band Some, pct>90 | ✅ (`unprecedentedness_marks_spike_unprecedented`) |
| T077 | F-019 | in-band value | not unprecedented | ✅ (`unprecedentedness_in_band_value_not_unprecedented`) |
| T078 | F-019 | unknown source | 404 | ✅ (`unprecedentedness_unknown_source_errors`) |
| T079 | F-019 | deterministic | byte-identical | ✅ (`unprecedentedness_is_deterministic`) |
| T080 | F-019 | < MIN_HISTORY_POINTS (12) | band None (documented) | ✅ (`score_returns_none_band_below_min_history`) |
| T081 | F-019 | custom k | honored | ✅ |
| T082 | F-019 | field absent in records | history empty → band None, percentile None | ✅ |
| T083 | F-020 | GET /v1/alerts, alerts on + dispatched | array of entries | ✅ |
| T084 | F-020 | alerts off | empty log (AlertLog still present, empty) | ✅ |

**D verdict: 15/15 ✅. No defects.**

## E. Signals (F-021..F-026)

| T# | Feature | Scenario | Trace | Verdict |
|----|---------|----------|-------|---------|
| T085 | F-021 | POST signal, owner="alice" | id `sig:alice:…`, stored | ✅ (`store_crud_roundtrip`) |
| T086 | F-021 | identical target (dedup) | same hash → same id → **overwrite** (insert, `signal.rs:159`) | ✅ (dedup-by-overwrite) |
| T087 | F-021 | owner omitted | `owner=""` | ✅ |
| T088 | F-021 | channels omitted | `channels:[]` | ✅ |
| T089 | F-022 | GET signals owner="alice" | filtered | ✅ |
| T090 | F-022 | GET signals owner="" (empty) | **returns ALL owners** (`signal.rs:170`) — R3 isolation gap | ⚠️ **D-009 risk** (documented shared-key model; flagged not fixed) |
| T091 | F-022 | owner filter mismatch | returns subset correctly | ✅ |
| T092 | F-023 | preview threshold_crossing above | fires on crossing | ✅ (`preview_threshold_crossing_counts_fires`) |
| T093 | F-023 | preview silent when not crossed | count 0 | ✅ (`preview_silent_when_not_crossed`) |
| T094 | F-023 | preview deterministic | count+fired_on identical | ✅ (`preview_is_deterministic`) |
| T095 | F-023 | **preview series_jump on a QUARTERLY series** | preview uses `detect_series_jumps` (unscaled, `signal.rs:322`); production uses `detect_series_jumps_cadenced` (scaled ×√(12/4)=1.73, `scheduler.rs:222`) → **different fire set** | ❌ **D-006** (confirmed — the flagship finding) |
| T096 | F-023 | preview series_jump on UNKNOWN cadence | unscaled == scaled for Unknown (`analysis.rs:506`) → matches | ✅ (accidentally correct for the default) |
| T097 | F-023 | preview outlier | both use `detect_outliers` → match | ✅ |
| T098 | F-023 | preview seasonality | both use `detect_seasonality` → match | ✅ |
| T099 | F-024 | GET signal unknown id | 200 `null` (not 404) | ✅ (documented) |
| T100 | F-025 | PATCH existing | 200 updated, `updated_at` set | ✅ |
| T101 | F-025 | PATCH unknown id | 404 | ✅ |
| T102 | F-026 | DELETE existing | `{deleted:true}` | ✅ |
| T103 | F-026 | DELETE unknown | `{deleted:false}` | ✅ |

**E verdict: 16/19 ✅, 1 ⚠️ (D-009 risk), 1 ❌ (D-006).**

## F. Investigations (F-027..F-032)

| T# | Feature | Scenario | Trace | Verdict |
|----|---------|----------|-------|---------|
| T104 | F-027 | POST investigation | id `inv:…`, title defaults to seed_title | ✅ |
| T105 | F-027 | unknown seed_source | `parse_source` → 404 | ✅ |
| T106 | F-027 | title provided | honored | ✅ |
| T107 | F-028 | GET investigations owner="" | returns ALL (`investigation.rs:134`) — same isolation gap as D-009 | ⚠️ D-009 risk |
| T108 | F-029 | GET unknown id | 200 null | ✅ |
| T109 | F-030 | DELETE unknown | `{deleted:false}` | ✅ |
| T110 | F-031 | append step kind=chip | `s1` assigned | ✅ (`append_step_assigns_monotonic_ids_and_bumps_updated`) |
| T111 | F-031 | kind=qa | `s2` | ✅ |
| T112 | F-031 | kind=bogus | 400 BadRequest (`routes.rs:858`) | ✅ |
| T113 | F-031 | unknown investigation id | 404 | ✅ (`append_step_unknown_id_returns_none`) |
| T114 | F-032 | add note | `n1` assigned | ✅ (`add_note_appends_and_bumps_updated`) |
| T115 | F-032 | unknown id | 404 | ✅ |

**F verdict: 11/12 ✅, 1 ⚠️ (D-009 risk, shared). No new defects.**

## G. Identity (F-033..F-035)

| T# | Feature | Scenario | Trace | Verdict |
|----|---------|----------|-------|---------|
| T116 | F-033 | POST request-token, new email | user provisioned + token returned, TTL 15min | ✅ (`issue_token_provisions_user_idempotently`) |
| T117 | F-033 | duplicate email | same user, **new** token (`identity.rs:261`) | ✅ (`two_tokens_for_same_email_differ`) |
| T118 | F-033 | case-insensitive email | `user_id_for` lowercases (`identity.rs:176`) | ✅ (`user_id_is_stable_and_case_insensitive`) |
| T119 | F-033 | empty/whitespace email | `email.trim()` → `""` → still provisions user `u:<hash of "">` | ⚠️ no validation; minor |
| T120 | F-034 | redeem valid token | session minted | ✅ (`redeem_valid_token_returns_session`) |
| T121 | F-034 | double-spend | second redeem → None → 400 | ✅ (`redeemed_token_cannot_be_reused`) |
| T122 | F-034 | expired token | `Utc::now() > expires_at` → None → 400 | ✅ |
| T123 | F-034 | unknown token | None → 400 | ✅ (`unknown_token_redeems_none`) |
| T124 | F-035 | GET auth/me w/ valid bearer | 200 `Option<User>` Some | ✅ (`end_to_end_identity_flow`) |
| T125 | F-035 | no Authorization header | 200 `null` (not 401 — documented) | ✅ |
| T126 | F-035 | session **never expires** | `Session` has no `expires_at` field (`identity.rs:61`); `lookup_session` has no TTL check → a leaked bearer is valid forever | ❌ **D-010** (security — see below) |

**G verdict: 9/11 ✅, 1 ⚠️, 1 ❌ (D-010).**

## H. Q&A (F-036)

| T# | Feature | Scenario | Trace | Verdict |
|----|---------|----------|-------|---------|
| T127 | F-036 | heuristic keyword match | summarize dataset | ✅ (`ask_heuristic_answers_on_keyword_match`) |
| T128 | F-036 | no match → inventory fallback | lists dataset names | ✅ (`ask_heuristic_falls_back_to_inventory`) |
| T129 | F-036 | empty store | honest "don't have any datasets" msg | ✅ (`heuristic_empty_store_is_honest`) |
| T130 | F-036 | empty question body | axum Json deserde missing `question` → 422 | ✅ |
| T131 | F-036 | rich mode, loop exhausts max_steps (6) | `Err(Internal)` → 502 | ✅ (`loop_errors_on_step_exhaustion`) |
| T132 | F-036 | rich mode, AgentOutcome::Findings | framed as 0.4-confidence Answer (`routes.rs:1030`) | ✅ |

**H verdict: 6/6 ✅. No defects.**

## I. Config & deploy (F-037..F-041)

| T# | Feature | Scenario | Trace | Verdict |
|----|---------|----------|-------|---------|
| T133 | F-037 | config.toml overrides defaults | figment merge order | ✅ |
| T134 | F-037 | HKGOV_ env overrides config | `Env::prefixed("HKGOV_").split("__")` | ✅ |
| T135 | F-037 | bad TOML | `Settings::load` Err → `main.rs:23` falls back to defaults w/ stderr | ✅ (graceful) |
| T136 | F-037 | empty api_prefix | boots clean (D-003 fixed) | ✅ (`empty_prefix_mounts_all_routes_at_root`) |
| T137 | F-038 | empty scan → defaults | `effective_scan` (`scheduler.rs:90`) | ✅ |
| T138 | F-038 | unknown detector | skip + warn | ✅ (`run_pass_unknown_detector_is_skipped`) |
| T139 | F-038 | threshold_crossing reachable | v7 wiring | ✅ (`threshold_crossing_target_fires_when_above`) |
| T140 | F-038 | companion missing on cross_source_gap | skip + warn | ✅ |
| T141 | F-039 | alerts off | dispatcher None, empty log | ✅ |
| T142 | F-039 | alerts on, feature off | dispatcher built, sinks warn | ✅ |
| T143 | F-040 | docker image serves dashboard | D-004 fixed | ✅ |
| T144 | F-041 | SIGTERM (unix) / Ctrl-C (win) | `shutdown_signal` select | ✅ |

**I verdict: 12/12 ✅. No defects.**

## J. Frontend dashboard (F-042..F-051)

| T# | Feature | Scenario | Trace | Verdict |
|----|---------|----------|-------|---------|
| T145 | F-042 | served over http, base auto-filled | `index.html:404` sets from location | ✅ |
| T146 | F-042 | opened via file:// | no location.port → base empty → falls back to localhost:8080 | ✅ |
| T147 | F-043 | brief renders cards | D-002 fixed (`index.html:276` passes flattened `it`) | ✅ |
| T148 | F-043 | empty brief | helpful empty state w/ agent-enable hint | ✅ |
| T149 | F-044 | severity filter | client-side filter | ✅ |
| T150 | F-045 | vote useful | POST + note update | ✅ |
| T151 | F-045 | insight id w/ `:` (e.g. `series_jump:hkma:…`) | `encodeURIComponent` in URL (`index.html:303`) | ✅ |
| T152 | F-046 | ask, normal response | appended to chatLog | ✅ |
| T153 | F-046 | ask, r.ok false | `{text:"error: "+status}` | ✅ |
| T154 | F-046 | empty question | `if(!q) return` | ✅ |
| T155 | F-047 | category + search + tag chip | builds query | ✅ |
| T156 | F-048 | health pills | tries /health/sources then /v1/health/sources | ✅ |
| T157 | F-049 | 30s auto-poll | setInterval brief+insights | ✅ |
| T158 | F-050 | mobile (≤900px) | grid 1col, header static | ✅ |
| T159 | F-051 | ARIA labels | sr-only inputs, aria-live regions | ✅ |

**J verdict: 15/15 ✅. No defects. (D-002 re-verified fixed.)**

## K. Python client (F-052..F-056)

| T# | Feature | Scenario | Trace | Verdict |
|----|---------|----------|-------|---------|
| T160 | F-052 | client constructs, sends X-API-Key | `client.py:52` | ✅ (`test_api_key_header_sent`) |
| T161 | F-052 | transport error → HkGovError | `_get`/`_post` catch | ✅ |
| T162 | F-052 | empty prefix | `_url` branches on prefix | ✅ |
| T163 | F-053 | tag as list | repeated params | ✅ (`test_sources_filters_pass_query_params`) |
| T164 | F-053 | tag as string | single param | ✅ (`test_sources_single_tag_string`) |
| T165 | F-053 | no filters | params None | ✅ (`test_sources`) |
| T166 | F-054 | brief re-nests insight | `client.py:190` | ✅ (`test_brief_re_nests_flattened_insight`) |
| T167 | F-055 | feedback POST + score GET | `client.py:196` | ✅ (`test_feedback_posts_and_reads_score`) |
| T168 | F-056 | client has NO method for: signals, investigations, auth, cite, silence-index, unprecedentedness, insight history, since/lang | grep `client.py` confirms only health/sources/categories/dataset/records/insights/brief/feedback/alerts/ask exist | ❌ **D-011** (coverage gap — see below) |

**K verdict: 8/9 ✅, 1 ❌ (D-011).**

---

## Defects found in Phase 3 (D-006..D-011)

### D-006 — Signal `series_jump` preview ≠ production (HIGH)
- **Feature:** F-023 (POST /v1/signals/preview)
- **Repro:** create a `series_jump` signal with `cadence=quarterly`, `threshold=25`, over data with a 30% jump. Preview reports a fire; the live scheduler (cadenced → ~43.3% effective threshold) does NOT fire.
- **Expected:** preview == production (module docstring `signal.rs:13-14,29-31`).
- **Actual:** preview calls `detect_series_jumps` (`signal.rs:322`); production calls `detect_series_jumps_cadenced` (`scheduler.rs:222`).
- **Root cause:** the preview dispatcher was written before v7 cadence scaling and never updated; it also lacks a `year_over_year` arm (so YoY signals aren't previewable at all).
- **Severity:** high — breaks the core product promise of signal subscriptions.

### D-007 — Bad `?since=` silently returns unfiltered insights (LOW)
- **Feature:** F-011 (GET /v1/insights?since=)
- **Repro:** `GET /v1/insights?since=banana` → 200 with the **full** insight list, no error.
- **Expected:** either 400 (bad timestamp) or an empty result — not a silent full dump that looks like "everything is new since banana".
- **Root cause:** `routes.rs:401-406` — `parse_since` failure falls through to `state.insights.list(q.limit)`.
- **Severity:** low — misleading but not data-corrupting.

### D-008 — Cite `base_url` ignores the Host header despite docstring (LOW)
- **Feature:** F-017 (GET /v1/insights/{id}/cite)
- **Repro:** request with `Host: api.example.com`, no `?base_url=` → permalink uses `http://localhost:8080`.
- **Expected:** `routes.rs:502-504` docstring says "Defaults to the request's Host header origin, then localhost".
- **Actual:** code (`routes.rs:520-522`) only checks the query param, then hardcodes localhost. The `Host` header is never read.
- **Severity:** low — doc/behaviour mismatch; permalinks are wrong in production behind a proxy.

### D-009 — No owner isolation on signals/investigations (RISK, documented)
- **Features:** F-022, F-026, F-028, F-030
- **Repro:** any keyed caller `GET /v1/signals?owner=` (empty) → all users' signals; `DELETE /v1/signals/{id}` → deletes anyone's signal.
- **Expected (for multi-tenant):** owner-scoped ACL.
- **Actual:** owner is a filter, not a guard. Code comments (`signal.rs:23`, `routes.rs:647`) explicitly call this "the shared-key trust model" for v1.
- **Severity:** risk (not a code bug per the v1 design) — **recommend waiving with reason** rather than fixing, but documenting loudly.

### D-010 — Sessions never expire (MEDIUM, security)
- **Feature:** F-035 (GET /v1/auth/me)
- **Repro:** redeem a token → get bearer → wait indefinitely → bearer still resolves.
- **Expected:** session has a TTL (the token does: 15 min).
- **Actual:** `Session` (`identity.rs:61`) has no `expires_at`; `lookup_session` (`identity.rs:149`) does no expiry check. A leaked bearer is valid forever.
- **Severity:** medium — security; magic-link identity's value is undermined if sessions are immortal.

### D-011 — Python client covers only v6–v9 read surface (LOW)
- **Feature:** F-056
- **Repro:** `dir(HkGov)` lacks methods for signals/investigations/auth/cite/silence-index/unprecedentedness/insight-history/since/lang.
- **Expected:** parity with the HTTP surface (the client is advertised as the typed interface to the API).
- **Severity:** low — the missing endpoints still work via `_get`/`_post`, but the typed contract is incomplete.

---

## Phase 3 exit-criteria check

- [x] Every test case (T001–T168) simulated and executed via code tracing.
- [x] Every defect documented with root-cause hypothesis + affected files.
- [x] `Affected Files` populated (cited inline per defect).
- [x] Prior D-001..D-005 re-verified as still fixed (their guards are present and the tests that lock them are in the green baseline of 178).

**Defect tally:** 6 new (D-006..D-011) · 1 high (D-006) · 1 medium (D-010) · 3 low (D-007, D-008, D-011) · 1 documented risk (D-009, recommend waive).
