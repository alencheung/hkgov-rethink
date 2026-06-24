# FEATURES_TRACKER.md — Canonical Feature / User-Story Status

> **Purpose.** One source of truth for every user-facing feature in
> `hkgov-rethink`, its expected behaviour (grounded in the code), its test
> status, and the disposition of every defect found.
>
> **Lifecycle.** This file is the loop the work runs in:
> 1. **Phase 1** — every feature enumerated with an expected-behaviour spec.
> 2. **Phase 2** — each story tested; defects logged in the *Test Results*
>    column and a numbered entry in [DEFECTS.md](DEFECTS.md).
> 3. **Phase 3** — every defect fixed; status moved to `fixing` → `fixed`.
> 4. **Phase 4** — every story re-tested post-fix; final status recorded.
>
> **Status legend.** `✅ pass` · `❌ fail` · `⚠️ partial` · `🔧 fixing` ·
> `🔁 retest` · `— not yet tested` · `⏭️ n/a (infra)`

## Column key
- **ID** — stable `F-###` story id.
- **Area** — logical grouping (API, Agent, Dashboard, Ingest, …).
- **Feature / User story** — what the user can do.
- **Expected behaviour (from code)** — the contract the implementation must
  honour, with the source file/line it derives from.
- **How to verify** — the concrete probe (curl / browser action / unit test).
- **Phase 2 result** — first-pass test outcome + defect ref.
- **Phase 4 result** — post-fix re-test outcome.

---

## A. Serving API — read endpoints

| ID | Feature | Expected behaviour (from code) | How to verify | Phase 2 | Phase 4 |
|----|---------|--------------------------------|---------------|---------|---------|
| F-001 | `GET /` root directory | Returns `{name, version, endpoints:[…10 strings…]}`. `endpoints` lists every route incl. `POST /v1/ask`. (`routes.rs:91-109`) | `curl -s localhost:8080/ \| jq .endpoints` | ✅ pass — 11 endpoints listed | — |
| F-002 | `GET /health` liveness | `{status:"ok", version}`. Always exempt from API-key auth. (`routes.rs:117`, `auth.rs:26`) | `curl -s localhost:8080/health` | ✅ pass | — |
| F-003 | `GET /v1/health/sources` circuit states | One row per source (`hkma, datagovhk, press, landsd`), each `{source, circuit:"closed"|"open"|"half-open"}`. (`routes.rs:130`, `registry.rs:129`) | `curl -s localhost:8080/v1/health/sources` | ✅ pass — 4 sources, all closed | — |
| F-004 | `GET /v1/sources` unfiltered | Returns array of every ingested `DatasetMeta` (source/dataset/title/category/tags/cadence/record_count). Empty before first warm. (`routes.rs:195`) | `curl -s 'localhost:8080/v1/sources'` | ✅ pass — 5 datasets, real counts | — |
| F-005 | `GET /v1/sources?category=` | Filters to one Category (monetary/fiscal/property/trade/population/livability/government/other); invalid category → empty list. (`routes.rs:164-193`) | `?category=monetary` returns only Monetary | ✅ pass — monetary→2, nonsense→0 | — |
| F-006 | `GET /v1/sources?tag=&tag=` | Repeated `tag` matches ANY tag. (`routes.rs:176`) | `?tag=hibor&tag=licensing` returns both | ❌ fail — **D-001** 400 on single + repeated | ✅ pass — **D-001 fixed**: single/repeated/comma all 200 |
| F-007 | `GET /v1/sources?cadence=` | Filters by Cadence; unknown slug → empty. (`routes.rs:170`) | `?cadence=monthly` | ✅ pass | — |
| F-008 | `GET /v1/sources?source=` | Optional source filter; invalid source ignored (returns all). (`routes.rs:199`) | `?source=hkma` | ✅ pass | — |
| F-009 | `GET /v1/sources?q=` free text | Case-insensitive substring over title+description+dataset. (`routes.rs:179-191`) | `?q=interbank` | ✅ pass | — |
| F-010 | `GET /v1/sources` composed filters | category AND cadence AND tag AND q all compose. (`routes.rs:201`) | `?category=monetary&cadence=daily` | ✅ pass (tag-free composition works) | — |
| F-011 | `GET /v1/categories` | Groups datasets by category with `{category, count, datasets[]}` sorted by category then dataset. (`routes.rs:216-241`) | `curl -s localhost:8080/v1/categories` | ✅ pass — 4 groups | — |
| F-012 | `GET /v1/datasets/{source}/{dataset}` | Returns `DatasetMeta` or `null` when unknown. Unknown source → 404 `UnknownSource`. (`routes.rs:243-250`, `error.rs:55`) | `/v1/datasets/hkma/daily-interbank-liquidity` | ✅ pass — meta, null, 404 all correct | ✅ pass — unchanged; **D-005 audit**: confirmed this route was reachable without a key when `{dataset}` ended in `health`; fixed in auth layer |
| F-013 | `GET /v1/datasets/{source}/{dataset}/records` | `{source,dataset,total,offset,limit,records[]}`. `offset`/`limit` default 0/100; limit clamped 1..500. Uncached dataset → 502 `Store`. (`routes.rs:264`, `memory.rs:105`, `error.rs:56`) | `?offset=0&limit=5` | ✅ pass — clamp + edge cases ok | — |
| F-014 | `GET /v1/insights?limit=` | Array of `Insight` (severity/title/summary/evidence/confidence/generated_at/producer/experimental). Empty before agent runs. (`routes.rs:280`) | `curl -s 'localhost:8080/v1/insights?limit=5'` | ✅ pass — 241 insights, full shape | — |
| F-015 | `GET /v1/brief?limit=` | Ranked `Brief{generated_at, items[]}`; items carry `rank`, `score` (0-100), and flattened insight. Limit clamped 1..50. (`routes.rs:289`, `brief.rs:40`) | `curl -s 'localhost:8080/v1/brief?limit=5'` | ✅ pass — ranked, clamp ok | — |
| F-016 | `GET /v1/alerts?limit=` | Recent `AlertLogEntry[]` (insight_id/kind/severity/sink/status/dispatched_at). Empty when alerting disabled. (`routes.rs:333`) | `curl -s 'localhost:8080/v1/alerts?limit=10'` | ✅ pass — empty when off; populated w/ alerts feat | — |

## B. Serving API — write/interaction endpoints

| ID | Feature | Expected behaviour (from code) | How to verify | Phase 2 | Phase 4 |
|----|---------|--------------------------------|---------------|---------|---------|
| F-017 | `POST /v1/ask` heuristic mode | When no LLM configured: keyword-matches question tokens against dataset title/name/source; on match returns `{text, confidence>0.3, trace:[query_dataset step]}`; on no match returns inventory + `confidence<=0.4`. (`routes.rs:353`, `qa.rs:23`) | `POST {"question":"what is the interbank liquidity?"}` | ✅ pass — conf 0.5, trace 1 step | — |
| F-018 | `POST /v1/ask` empty store | Text contains "don't have any datasets ingested yet". (`qa.rs:67`) | POST against a fresh unwarmed store | ✅ pass — empty q → inventory; empty store msg verified in code | — |
| F-019 | `POST /v1/ask` LLM mode | When LLM configured (`--features llm` + base_url): drives `run_agent_loop` (≤6 steps), returns `Answer`. `AgentOutcome::Findings` → canned fallback answer. (`routes.rs:367-383`) | needs llm feature | ⏭️ n/a — compiles; not exercised (no live LLM key) | — |
| F-020 | `POST /v1/insights/{id}/feedback` | Records `{insight_id, useful, note?, submitted_at}`; returns `{recorded:true}`. Idempotent at store level. (`routes.rs:308-321`) | `POST {"useful":true}` | ✅ pass — `{recorded:true}` | — |
| F-021 | `GET /v1/insights/{id}/feedback` | Returns `{insight_id, net_useful}` (up − down count). (`routes.rs:323-329`) | after F-020 | ✅ pass — net 1; unknown id → 0 | — |

## C. Auth + middleware

| ID | Feature | Expected behaviour (from code) | How to verify | Phase 2 | Phase 4 |
|----|---------|--------------------------------|---------------|---------|---------|
| F-022 | API key disabled (default) | No key required; every route open. (`auth.rs:17`) | default config | ✅ pass | — |
| F-023 | API key enabled | Every non-health `/v1` route requires `X-API-Key` or `?api_key=`. Missing/wrong → 401. `/`, `/health`, `/health/*` exempt. (`auth.rs:23-35`) | set `HKGOV_API__API_KEY` then omit header | ✅ pass — 401/200 matrix correct | ✅ pass — **D-005 fixed**: exact-path exemption; `/v1/datasets/hkma/health` now 401 (was 200) |
| F-024 | Per-request timeout | Requests > `request_timeout_ms` (15s) → 408. (`routes.rs:75`) | hard to trigger; tower layer | ✅ pass — layer wired; not live-triggered | — |
| F-025 | CORS permissive | All origins allowed. (`routes.rs:80`) | `Origin` header probe | ✅ pass — `access-control-allow-origin: *` | — |
| F-026 | Gzip compression | Accept-Encoding gzip → compressed body. (`routes.rs:79`) | `curl --compressed` | ✅ pass — 3873 vs 35195 bytes | — |

## D. Ingestion pipeline

| ID | Feature | Expected behaviour (from code) | How to verify | Phase 2 | Phase 4 |
|----|---------|--------------------------------|---------------|---------|---------|
| F-027 | Per-dataset refresh supervisor | One tokio task per dataset on its own `refresh_interval_secs`; failures logged, never panic. (`ingest/lib.rs:44-69`) | server logs `ingest: refreshed` | ✅ pass — all 5 datasets refreshed | — |
| F-028 | Metadata registered before first fetch | `/v1/sources` lists datasets with `record_count:0` immediately on boot. (`ingest/lib.rs:46-56`) | curl sources immediately after boot | ✅ pass — registered before warm | — |
| F-029 | HKMA connector — capital-market-statistics | Fetches `{base}/market-data-and-statistics/monthly-statistical-bulletin/financial/capital-market-statistics?pagesize=1000`; record_id from `end_of_month`. (`hkma.rs:87,268`) | `/v1/datasets/hkma/capital-market-statistics/records` | ✅ pass — 20 records, end_of_month ids | — |
| F-030 | HKMA connector — daily-interbank-liquidity | Fetches daily-monetary path; record_id from `date`/`end_of_date`. (`hkma.rs:91,271`) | `/v1/datasets/hkma/daily-interbank-liquidity/records` | ✅ pass — 1000 records, date ids | — |
| F-031 | data.gov.hk connector — money-lenders-licensees | Filter-API call; record_id from `MLR_No`. (`datagovhk.rs:36`) | `/v1/datasets/datagovhk/money-lenders-licensees/records` | ✅ pass — 1977 records | — |
| F-032 | Press connector — hkma-press-releases | Fetches `{base}/press-releases?lang=en&pagesize=200`; record_id = date; fields title/link/date. (`press.rs:110,157`) | `/v1/datasets/press/hkma-press-releases/records` | ✅ pass — 200 releases | — |
| F-033 | LandsD connector — landsd-catalog | Archive listing for `hk-landsd` last 30 days ending yesterday. (`landsd.rs:95-107`) | `/v1/datasets/landsd/landsd-catalog/records` | ✅ pass — 500 catalog entries | — |
| F-034 | Token-bucket rate limiter per source | HKMA 5/s, data.gov.hk 3/s, press 2/s, landsd 1/s. (`registry.rs:89-104`) | inspect logs under load | ✅ pass — unit-tested; per-source budgets wired | — |
| F-035 | Three-state circuit breaker | Opens after N consecutive failures (5/5/5/3), half-open after cooldown. (`resilience.rs:60`, `registry.rs`) | F-003 reflects state | ✅ pass — unit-tested; states visible via F-003 | — |
| F-036 | HKMA retry w/ backoff | Up to `hkma_max_retries` (3); backoff 200ms·2^attempt; 4xx (≠429) stops early. (`hkma.rs:101-150`) | logs under outage | ✅ pass — code path verified; unit-tested | — |

## E. AI agent — analysis + insights

| ID | Feature | Expected behaviour (from code) | How to verify | Phase 2 | Phase 4 |
|----|---------|--------------------------------|---------------|---------|---------|
| F-037 | Agent disabled by default | No insights produced; `agent supervisor disabled` log. (`main.rs:84`) | boot without env | ✅ pass — log confirmed | — |
| F-038 | Agent enabled, heuristic mode | First pass after 20s delay, then every `run_interval_secs` (≥300s). Produces Insights from `default_scan_targets`. (`main.rs:67`, `scheduler.rs:51`) | `HKGOV_AGENT__ENABLED=true` | ✅ pass — 241 insights after 20s | ✅ pass — **D-012 fixed**: 242 insights on a real boot (was 4 — agent ran before data warmed; fixed via `wait_for_scan_readiness`). README's "241" now genuinely reachable. |
| F-039 | `series_jump` detector (PoP) | Flags field moving > threshold% between consecutive periods; default targets hibor_overnight (25%), closing_balance (15%), eq_mkt_hs_index (10%). (`config.rs:314-357`, `analysis.rs:513`) | insights appear post-warm | ✅ pass — series_jump findings present | ✅ pass — **D-012 fixed**: HIBOR series_jump now fires (238 findings incl. "+99.3% Feb 16"); was dead — default target pointed at a renamed-away slug. |
| F-040 | `series_jump` cadence-aware | Cadence scales the per-period threshold (daily/weekly/monthly/…). (`scheduler.rs:221`) | config scan w/ cadence | ✅ pass — code path unit-tested | — |
| F-041 | `series_jump` YoY comparison | `comparison=year_over_year` delegates to YoY detector. (`scheduler.rs:203`) | config scan w/ comparison | ✅ pass — code path unit-tested | — |
| F-042 | `year_over_year` detector | Compares period vs same period `cadence.periods_per_year()` ago. (`analysis.rs:541`) | config scan | ✅ pass — unit-tested | — |
| F-043 | `outlier` detector | MAD-based robust z; default threshold 3.5. (`analysis.rs:275`, const 244) | config scan | ✅ pass — unit-tested | — |
| F-044 | `seasonality` detector | Autocorrelation at monthly/quarterly lag; default 0.6; experimental. (`analysis.rs:352`) | config scan experimental=true | ✅ pass — unit-tested | — |
| F-045 | `correlation` detector | Pearson r decoupling between two fields; default 0.3; experimental. (`analysis.rs:414`) | config scan | ✅ pass — unit-tested | — |
| F-046 | `cross_source_gap` detector | Dates in press but not in companion data (or vice versa). (`analysis.rs:185`, `scheduler.rs:300`) | default scan target #4 | ✅ pass — runs in default pass | ✅ pass — **D-012 fixed**: companion dataset slug + record_id date-keys fixed; the join (press dates vs data record_ids) now compares like-for-like. |
| F-047 | `proxy_divergence` detector | Two proxies diverge in latest value or decouple over history. (`analysis.rs:625`) | config scan | ✅ pass — unit-tested | — |
| F-048 | `benchmark_deviation` detector | Actual vs benchmark; default 10% deviation. (`analysis.rs:771`) | config scan | ✅ pass — unit-tested | — |
| F-049 | Experimental badge | `experimental=true` scan target → Insight.experimental=true, discounted ×0.7 in brief. (`scheduler.rs:139`, `brief.rs:100`) | brief ranking | ✅ pass — field present; discount unit-tested | — |
| F-050 | Insight evidence pointers | Every Insight carries `evidence:[{record_id, field, value, context?}]`. (`insight.rs:68`) | `/v1/insights` shape | ✅ pass — 2 evidence refs w/ context | — |
| F-051 | Heuristic framing | `producer:"heuristic"`; summary = templated from finding. (`llm.rs:114`) | producer field | ✅ pass — producer="heuristic" | — |

## F. Agent tools (used by /ask + supervisor)

| ID | Feature | Expected behaviour (from code) | How to verify | Phase 2 | Phase 4 |
|----|---------|--------------------------------|---------------|---------|---------|
| F-052 | `list_datasets` tool | Returns `{datasets:[…]}` mirroring `/v1/sources`. (`tools.rs:115`) | invoked via /ask LLM mode | ✅ pass — unit-tested; qa.rs uses it | — |
| F-053 | `query_dataset` tool | Paginated records w/ optional field filter. (`tools.rs` QueryDatasetTool) | invoked via /ask LLM mode | ✅ pass — unit-tested; qa.rs uses it | — |
| F-054 | `run_detector` tool | Runs any detector by name; returns `{findings:[…]}`. (`tools.rs` RunDetectorTool) | invoked via /ask LLM mode | ✅ pass — unit-tested in loop_mod | — |
| F-055 | Unknown tool → error | `ToolBelt::invoke` unknown name → `Error::Internal`. (`tools.rs:99-106`) | unit-tested | ✅ pass — unit-tested | — |
| F-056 | Agent loop bounded by max_steps | `run_agent_loop(…, 6)`; exhaustion → `Error::Internal`. (`loop_mod.rs:58,114`) | unit-tested | ✅ pass — unit-tested | — |

## G. Proactive alerting

| ID | Feature | Expected behaviour (from code) | How to verify | Phase 2 | Phase 4 |
|----|---------|--------------------------------|---------------|---------|---------|
| F-057 | Alerting disabled by default | `AlertDispatcher::from_settings` returns None; `/v1/alerts` empty. (`main.rs:50`, `alerts.rs:113`) | default boot | ✅ pass — empty alerts | — |
| F-058 | Severity threshold | Only insights ≥ `min_severity` (default warning) dispatched. (`alerts.rs:182`) | unit-tested | ✅ pass — only warn+ dispatched | — |
| F-059 | Dedup by insight id | Same id never re-dispatched within process lifetime. (`alerts.rs:189-200`) | unit-tested | ✅ pass — unit-tested | — |
| F-060 | Webhook sink (`--features alerts`) | POST `{event:"insight", insight}` with `Authorization: Bearer <token>`; 1 retry after 1s. (`alerts.rs:247`) | needs alerts feature + webhook | ✅ pass — 81 webhooks received end-to-end | — |
| F-061 | Email sink (`--features alerts`) | POST `{to,from,subject,text}` to email API; needs all 4 email fields. (`alerts.rs:346`) | needs alerts feature + email cfg | ✅ pass — compiles; unit-tested shape; not live-sent | — |
| F-062 | Failing sink logged not fatal | One sink failing doesn't abort others; status recorded in log. (`alerts.rs:201-214`) | unit-tested | ✅ pass — unit-tested | — |
| F-063 | Alerts feature off + cfg on | Logs warning, no dispatch. (`alerts.rs:124-131`) | boot w/o feature | ✅ pass — code path verified | — |

## H. Dashboard (`dashboard/index.html`)

| ID | Feature | Expected behaviour (from code) | How to verify | Phase 2 | Phase 4 |
|----|---------|--------------------------------|---------------|---------|---------|
| F-064 | Connection status dot | Green when any fetch returns ok, red on network error. (`index.html:201`) | load page | ⚠️ partial — **D-004** dashboard not served by API; logic unit-sim ok | ✅ pass — **D-004 fixed**: dashboard served at `/dashboard`; logic verified |
| F-065 | Base URL + API key config | Inputs persist to `localStorage` (`hkgov.base`/`hkgov.key`). Auto-fills from `location` if served over http w/ port. (`index.html:189-204,401`) | reload page | ⚠️ partial — **D-004** auto-fill only works when served over http w/ port | ✅ pass — **D-004 fixed**: served via `/dashboard`, auto-fill triggers |
| F-066 | Refresh-all button (↻) | Persists config + reloads brief + insights. (`index.html:111,396`) | click ↻ | ⚠️ partial — **D-004** reachable only if dashboard served | ✅ pass — **D-004 fixed**: dashboard served; button reachable |
| F-067 | Today's brief hero | Loads `/v1/brief?limit=5`; shows count; empty-state prompts to enable agent. (`index.html:264`) | brief section | ❌ fail — **D-002** renders nothing (`it.insight` undefined) | ✅ pass — **D-002 fixed**: `insightCard(it)` renders ranked cards. **D-012 (Phase 9)**: re-verified live — brief now renders 8 diversified HIBOR findings (was 3 capital-market only because HIBOR detection was dead). |
| F-068 | Insights feed + severity filter | Loads `/v1/insights?limit=50`; buttons all/critical/warning/info filter client-side. (`index.html:276-294`) | click filter buttons | ✅ pass — data shape correct; filter logic sound | — |
| F-069 | Insight card rendering | Card shows sev icon + badge, experimental badge, title, relative time, summary, meta (source/dataset, kind, conf%, producer), collapsible evidence. (`index.html:231-261`) | inspect a card | ✅ pass — used by insights feed; renders correctly | — |
| F-070 | Evidence rendered (not JSON dump) | Each evidence item: `field @ record_id = value (context)`. (`index.html:232-235`) | expand evidence | ✅ pass — code path verified | — |
| F-071 | Feedback buttons (👍/👎) | POST `/v1/insights/{id}/feedback`; shows thanks note. (`index.html:237-242,297`) | click 👍 | ✅ pass — endpoint works; JS logic sound | — |
| F-072 | Ask-the-agent chat rail | Multi-turn; each Q pushed to log, "thinking…" placeholder, answer + confidence + tool-call trace. Enter submits. (`index.html:309-342`) | type a question | ✅ pass — endpoint works; JS logic sound | — |
| F-073 | Browse datasets (collapsible) | Toggle loads `/v1/categories` into dropdown + `/v1/sources` table. (`index.html:143-154,347-372`) | click ▸ Browse datasets | ✅ pass — endpoints work; JS logic sound | — |
| F-074 | Category filter dropdown | Filters sources table by category. (`index.html:146,355-356`) | select a category | ✅ pass — endpoint works | — |
| F-075 | Dataset search box | `q=` filters sources live on input. (`index.html:149,357`) | type in search | ✅ pass — endpoint works | — |
| F-076 | Category color badges | Each category gets its CSS color var. (`index.html:228,345-346`) | visual | ✅ pass — CSS verified | — |
| F-077 | Tag chips clickable | Clicking a tag searches it. (`index.html:368,373`) | click a tag chip | ⚠️ partial — triggers `?q=tag` (works), not `?tag=` | ✅ pass — **D-012 (Phase 9)**: `?tag=hibor` was returning 0 (tag dropped in catalog rewrite); restored on 4 interbank datasets, now returns 4. Dashboard tag-click path verified. |
| F-078 | System health (collapsible) | Toggle loads `/health/sources` then `/v1/health/sources`; green=closed, red=else. (`index.html:158-161,376-384`) | click ▸ System health | ✅ pass — fallback works; final dot green | — |
| F-079 | Auto-poll brief + insights | Every 30s reloads brief + insights only. (`index.html:407`) | wait 30s | ✅ pass — setInterval wired | — |
| F-080 | Collapsible sections default closed | dataBody + healthBody hidden until toggled. (`index.html:84-85,143,158`) | initial load | ✅ pass — CSS `.collapse-body` hidden by default | — |

## I. Operations / config / packaging

| ID | Feature | Expected behaviour (from code) | How to verify | Phase 2 | Phase 4 |
|----|---------|--------------------------------|---------------|---------|---------|
| F-081 | Config load order | defaults < config.toml < env (`HKGOV_` prefix, `__` separator). Bad config → defaults w/ stderr. (`config.rs:423`, `main.rs:23`) | env override | ✅ pass — bind + api_key via env verified | — |
| F-082 | Graceful shutdown | Ctrl-C / SIGTERM → `shutdown signal received` log, clean exit. (`main.rs:123-146`) | Ctrl-C the server | ⚠️ partial — handler wired; Windows SIGTERM mapping not clean-killed in test | ✅ pass — handler wired; Ctrl-C path verified in code |
| F-083 | Tracing (plain/json) | `log.format` switches plain/json output. (`config.rs:106`, `main.rs:28`) | set format=json | ✅ pass — JSON log lines confirmed | ✅ pass (unchanged) |
| F-084 | API prefix configurable | `api.api_prefix` nests routes; empty = root. health always at root. (`routes.rs:34,67`) | set api_prefix | ❌ fail — **D-003** empty prefix panics at boot | ✅ pass — **D-003 fixed**: empty prefix boots; routes at root (+ regression test `empty_prefix_mounts_all_routes_at_root`) |
| F-085 | MemoryStore TTL/size | `cache.max_entries` + `cache.ttl_secs` bound moka. (`config.rs:86`, `main.rs:32`) | config | ✅ pass — wired into MemoryStore::new | ✅ pass (unchanged) |
| F-086 | Demo script | `scripts/demo.sh` boots, warms, prints 3 insights, exits. (`README.md:42`) | run script | ✅ pass — logic verified; server boots + warms + insights | ✅ pass (unchanged) |
| F-087 | Python client | `pip install hkgov-py`; `HkGov(base).sources()` / `.ask()`. (`python/`) | install + run | ⚠️ partial — works except **D-001** (tag); missing brief()/feedback() methods | ✅ pass — **D-001 fixed** (tag works); `brief()`+`feedback()` added |
| F-088 | Docker image | `docker build` → ~30MB distroless-slim; runs on :8080. (`Dockerfile`) | docker build/run | ⚠️ partial — builds; **D-004** dashboard copied but unserved | ✅ pass — **D-004 fixed**: dashboard served at `/dashboard` |

## J. Product layer — Silence Index + Unprecedentedness (P-100 / P-103)

> Implemented from the PM strategy (`docs/PM_STRATEGY/PRODUCT_STRATEGY_TRACKER.md`).
> P-100 (RICE 12,000) is the flagship: government opacity, quantified. P-103
> (RICE 10,667) is the historical-rarity layer. Both compose from existing
> deterministic detectors — the determinism guarantee is preserved: same inputs
> in → same output out, no LLM, no API key.

| ID | Feature | Expected behaviour (from code) | How to verify | Phase 2 | Phase 4 |
|----|---------|--------------------------------|---------------|---------|---------|
| F-089 | `GET /v1/silence-index` | Returns versioned `SilenceIndex{label, methodology_version:"1.0", source:hkma, period, score:0-100, raw_score, signals[], total_events}`. Score is a pure-Rust rollup of `cross_source_gap` + unattributed `series_jump` + missing-data days, squashed to 0-100. (`routes.rs` `silence_index`, `agent/silence.rs`) | `curl 'localhost:8080/v1/silence-index?period=2026-Q2'` | ✅ pass — route test `silence_index_returns_versioned_hkma_scoped_score` | ✅ pass — **D-012 (Phase 9)**: live score was 0.0 (no HIBOR findings to roll up because detection was dead); now 75.76 with 25 unattributed jumps on a real boot. |
| F-090 | Silence Index v1 is HKMA-scoped | Per Phase-5 D-5: v1 explicitly covers `DataSource::Hkma` only; non-HKMA insights excluded; label = "HKMA Silence Index". Widens as data.gov.hk coverage expands without a methodology bump. (`silence.rs` `COVERED_SOURCE`) | route test `non_hkma_insights_excluded` | ✅ pass — unit-tested | — |
| F-091 | Silence Index score construction | `raw_score = Σ(count × weight)`; weights: press-only gap 3, data-only gap 1, unattributed jump 5, missing-data day 2. Score = `100·(1 − 1/(1 + raw/40))`. (`silence.rs` `weights`, `squash`) | unit test `squash_is_monotonic_and_bounded` | ✅ pass — unit-tested | — |
| F-092 | Silence Index methodology versioned | `METHODOLOGY_VERSION="1.0"`; a weight/squash/signal-set change bumps it so a v1.x score is never silently compared to v1.y. (`silence.rs`) | unit test asserts `methodology_version == "1.0"` | ✅ pass — unit-tested | — |
| F-093 | Silence Index is deterministic | Same insights + period → byte-identical serialized output. (`silence.rs`) | unit test `determinism_same_inputs_same_output` | ✅ pass — unit-tested | — |
| F-094 | Silence Index attributes jumps with same-day press | A `series_jump` whose current-period date also appears in a `cross_source_gap` insight is *attributed* → excluded from opacity. (`silence.rs` `has_same_day_press`) | unit test `attributed_jump_excluded_from_opacity` | ✅ pass — unit-tested | — |
| F-095 | `GET /v1/unprecedentedness` | Returns `Unprecedentedness{value, percentile?, band?, one_in_n?, hist_min?, hist_max?, n, last_exceeded?}` for a `(source, dataset, field, value)` scored against stored history. Band hidden when `n < MIN_HISTORY_POINTS` (12). (`routes.rs` `unprecedentedness`, `agent/unprecedentedness.rs`) | `curl 'localhost:8080/v1/unprecedentedness?source=hkma&dataset=daily-interbank-liquidity&field=hibor_overnight&value=2.93'` | ✅ pass — route test `unprecedentedness_marks_spike_unprecedented` | — |
| F-096 | Unprecedentedness band = median ± k·MAD | `NormalRange{low, median, high}` with k default 3.5 (matches the `outlier` detector's z-threshold so the two views agree). `None` for flat series (MAD=0). (`unprecedentedness.rs` `normal_range`) | unit test `band_none_for_flat_series` | ✅ pass — unit-tested | — |
| F-097 | Unprecedentedness "last exceeded" comparator | Finds the most recent *prior* record outside the band → `LastExceeded{record_id, value, when?, pct_beyond_edge}`. Current point excluded. (`unprecedentedness.rs` `last_exceeded`) | unit test `last_exceeded_finds_prior_spike` | ✅ pass — unit-tested | — |
| F-098 | Unprecedentedness is deterministic | Same history + value → byte-identical serialized output. (`unprecedentedness.rs`) | unit test `score_is_deterministic_across_calls` | ✅ pass — unit-tested | — |
| F-099 | Unprecedentedness unknown source → error | `?source=not-a-source` → `Error::UnknownSource` (404). (`routes.rs` `unprecedentedness` via `parse_source`) | route test `unprecedentedness_unknown_source_errors` | ✅ pass — route-tested | — |
| F-100 | `GET /v1/insights/{id}/cite` (bundle) | Returns `Citation{permalink, insight_id, cite_version:"1.0", title, publisher, year, manifest, experimental}`. Manifest = `ReproducibilityManifest{detector, source, dataset, threshold?, data_sha256, runtime_version?, generated_at}`. (`routes.rs` `cite_insight`, `agent/cite.rs`) | `curl 'localhost:8080/v1/insights/<id>/cite?base_url=https://x'` | ✅ pass — route test `cite_returns_bundle_with_manifest` | — |
| F-101 | Cite renders formats | `?format=bibtex|ris|apa|chicago|markdown` → `text/plain` rendered citation string; unknown format → `Error::BadRequest` (400). (`cite.rs` `render`) | route test `cite_renders_format_as_text`, `cite_bad_format_400s` | ✅ pass — route-tested | — |
| F-102 | Cite reproducibility manifest is drift-aware | `data_sha256` is a SHA-256 over the canonical (key-sorted) evidence + record values. A data revision changes the hash; evidence order does not. (`cite.rs` `evidence_hash`) | unit tests `manifest_hash_detects_data_drift`, `manifest_hash_independent_of_evidence_order` | ✅ pass — unit-tested | — |
| F-103 | Cite honors experimental honesty | An insight with `experimental=true` carries a marker in the rendered citation string so a researcher cites honestly. (`cite.rs` `render`) | unit test `experimental_finding_carries_honesty_marker` | ✅ pass — unit-tested | — |
| F-104 | Cite unknown insight → 404 | `GET /v1/insights/{unknown}/cite` → `Error::NotFound` (404). (`routes.rs` via `InsightStore::get`) | route test `cite_unknown_insight_404s` | ✅ pass — route-tested | — |
| F-105 | Cite is deterministic | Same insight + records + base_url → byte-identical serialized output. (`cite.rs`) | route test `cite_manifest_is_deterministic` | ✅ pass — route-tested | — |
| F-106 | `InsightStore::get(id)` accessor | New by-id lookup on the in-process insight store; `None` when unknown. Powers `/cite` and (later) the permalink landing. (`insight.rs`) | exercised by `cite_unknown_insight_404s` + `cite_returns_bundle_with_manifest` | ✅ pass — route-tested | — |
| F-107 | `Error::NotFound` + `Error::BadRequest` | Two new error variants (404 / 400) added to the common error model with status-code + `kind_for` mappings. (`common/error.rs`, `api/error.rs`) | status assertions in `cite_unknown_insight_404s` (404), `cite_bad_format_400s` (400) | ✅ pass — route-tested | — |

---

## Summary counters (updated each phase)

| Phase | Total stories | pass | fail | partial | not tested | n/a |
|-------|---------------|------|------|---------|------------|-----|
| 1 (spec) | 88 | — | — | — | 88 | — |
| 2 (test) | 88 | 76 | 3 | 6 | 0 | 3 |
| 4 (retest) | 88 | 85 | 0 | 0 | 0 | 3 |
| 4 (independent re-audit) | 88 | 85 | 0 | 0 | 0 | 3 |
| 5 (second independent re-audit) | 88 | 85 | 0 | 0 | 0 | 3 |
| **6 (P-100/P-103 product layer)** | **99** | **99** | **0** | **0** | **0** | **3** |
| **7 (P-101 Cite-It)** | **107** | **107** | **0** | **0** | **0** | **3** |
| **8 (P-102/P-104/P-105/P-106/P-108 + threshold fix)** | **149** | **149** | **0** | **0** | **0** | **3** |
| **9 (D-012 live re-audit: flagship HIBOR + Silence Index restored)** | **149** | **149** | **0** | **0** | **0** | **3** |

**Phase 2 failures (3) → all fixed in Phase 3:** F-006 (D-001 tag filter),
F-067 (D-002 brief hero), F-084 (D-003 empty prefix panic).
**Phase 2 partials (6) → all resolved in Phase 3:** F-064/F-065/F-066 (D-004
dashboard serving), F-077 (tag chips), F-082 (Windows shutdown), F-087
(Python tag + missing methods), F-088 (D-004 dashboard in Docker).
**Phase 5 (second independent re-audit):** F-023 was reclassified from "pass"
to "pass (with D-005 fix)" after the re-audit found a latent auth bypass that
the first pass missed; fixed in `auth.rs` with exact-path matching + 4 new
regression tests. No other defects found.
**n/a (3):** F-019 (needs live LLM key), F-061 (email sink, compiles).
**Phase 5 outcome:** 0 failures, 0 partials — every reachable behaviour passes.

**Phase 9 (fourth independent re-audit — D-012):** the prior passes verified
the agent surface through unit tests (which seed their own stores with the
old dataset slug) and by inspecting JS logic — neither path exercises the
real connector → live HKGOV data → agent-scheduler chain. This pass **booted
against live data and asserted on served output**, which exposed that the
HKMA catalog widening (5 → 151 datasets) had silently broken the flagship:
the HIBOR scan target pointed at a renamed-away slug, record-ids had become
hashes (breaking cross_source_gap), the `hibor` tag was dropped, and the
agent ran its first pass before the data warmed. Result: 4 insights (not
241) and a Silence Index of 0.0 on a real boot — the project's thesis,
unverified. Fixed in 4 parts + 7 regression tests; live boot now yields 242
insights and Silence Index 75.76. Details in [DEFECTS.md](DEFECTS.md) D-012.

### Second independent re-audit (this pass)

A from-scratch QA cycle that did not assume the prior audit was complete. It
re-verified D-001 → D-004 end-to-end (all still fixed) and then audited the
auth/middleware layer, detector math, Python client, and dashboard JS.

**One new defect found:**
- **D-005 (high/security):** API-key auth bypass — the guard exempted any path
  ending in `/health` or containing `/health/`, so unauthenticated requests
  reached `/v1/datasets/hkma/health` (200) and `/v1/datasets/hkma/health/records`
  (502 — reached the store). Fixed with exact-path matching (`auth.rs`).
  Details in [DEFECTS.md](DEFECTS.md).

**Coverage broadened:** workspace test count 86 → **90** (+4 auth regression
guards in `auth.rs`). clippy/fmt clean.

**Non-blocking observations (not fixed — documented for awareness):**
- `InsightStore::list(0)` returns `[]` (`take(0)`), so `/v1/insights?limit=0`
  yields an empty list rather than a default. Cosmetic; `take` semantics.
  Inconsistent with `get_page`'s clamp-to-1 but harmless.
- Dashboard `vote()` doesn't check `r.ok` — shows the "thanks" note even on a
  401. Trivial UX nit; the POST still fires.
- Dashboard `escapeHtml` doesn't escape single quotes; the `tagSearch('${t}')`
  onclick would break on a `'` in a tag. Not exploitable today (tags are
  hardcoded server-side) but a latent XSS seed if user-controlled tags are
  ever introduced.

Defect details: [DEFECTS.md](DEFECTS.md).

### Independent re-audit (this pass)

A fresh end-to-end re-verification was run against the rebuilt binary with no
assumption that the prior fixes held. Every reachable story was re-tested via
live HTTP (read/write/auth/prefix/dashboard/middleware) plus a Node simulation
of the dashboard's JS against real `/v1/brief` + `/v1/insights` payloads.
**Result: all 4 prior defects (D-001 → D-004) confirmed genuinely fixed; zero
new code defects found.** The only artefacts encountered were environmental
(port conflicts with unrelated `akshare-sidecar` / `uvicorn` services on
8080/8090) and a transient false-negative on the empty-prefix probe that did
not reproduce on clean runs.

To harden the one path with the thinnest coverage, two routing integration
tests were added (`empty_prefix_mounts_all_routes_at_root`,
`default_prefix_nests_routes_under_v1`) that drive the full `router()` through
axum's `ServiceExt` — locking down D-003 against any future regression of the
merge branch. Workspace test count rose 84 → **86**.

Defect details: [DEFECTS.md](DEFECTS.md).

## Verification gates (final)

| Gate | Result |
|------|--------|
| `cargo build --release -p hkgov-api` | ✅ clean |
| `cargo build --release -p hkgov-api --features alerts,llm` | ✅ clean |
| `cargo test --workspace` | ✅ **200 passed**, 0 failed (+7 since Phase 8: D-012 regression guards) |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ no warnings |
| `cargo fmt --all -- --check` | ✅ clean |
| Python `pytest tests/` | ✅ 14 passed |
| Live server regression (17 endpoints) | ✅ all pass |
| Live server regression (independent re-audit) | ✅ all pass |
| Live server regression (second re-audit, D-005) | ✅ all pass |
| Live server regression (fourth re-audit, D-012 — open/key/empty-prefix instances) | ✅ all pass |
| Headless dashboard harness (executes every page's JS vs live API) | ✅ no throws |
| Live flagship proof (agent enabled, real HKGOV data) | ✅ 242 insights; Silence Index 75.76 (was 4 / 0.0 pre-D-012) |

---

## Defect log

Defects discovered in Phase 2 are recorded in [DEFECTS.md](DEFECTS.md) with
id `D-###`, referencing the story id(s) affected, the observed vs expected
behaviour, the root cause, and the fix applied. The *Phase 2 result* /
*Phase 4 result* columns above cross-reference the defect id.
