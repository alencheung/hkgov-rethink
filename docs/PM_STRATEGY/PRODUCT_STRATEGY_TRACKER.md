# Product Strategy Tracker — Canonical Spreadsheet

> **One source of truth** for every product feature in `hkgov-rethink`.
> Schema is fixed across all PM phases. New features get the next `P-###` id.
> Seed rows (`P-001`–`P-018`, `Status = shipped`) are the base inventory from
> `FEATURES_TRACKER.md`; they anchor every later comparison.
>
> This file **complements** the QA-focused `FEATURES_TRACKER.md` (which tracks
> test status of *implemented* behaviour). This file tracks *product strategy*
> (why a feature exists, its KPI, its priority). The QA tracker answers "does
> it work?"; this answers "should it exist & did it move the metric?"

## Schema

| Col | Meaning |
|---|---|
| **Feature ID** | Stable `P-###`. `P-001`+ = base inventory; `P-100`+ reserved for Phase-2 ideation. |
| **Persona** | Maya=Journalist · Chen=Researcher · Marcus=Analyst · Priya=Watchdog · Ops=Deployer · All |
| **Use Case** | The situation that creates demand. |
| **Feature Name** | Short, shippable name. |
| **User Story** | "As a __, I want __, so that __." |
| **UX Flow Summary** | Step-by-step journey (populated in Phase 3; `—` until then). |
| **Innovation (1-5)** | 1=incremental, 5=blue-ocean/defensible moat. |
| **Effort** | S / M / L (eng + design). |
| **Impact** | L / M / H on the core KPI (verified insights acted on). |
| **KPI** | The success metric tied to this feature. |
| **Status** | `shipped` · `ideated` · `approved` · `designed` · `specified` · `backlog` |

---

## A. Base inventory — shipped features (Phase 1 seed)

> Source: `README.md` Features §, `FEATURES_TRACKER.md` F-001–F-088.
> Innovation scores reflect *today* (these were novel when shipped; recorded
> against current market, not their v1 launch state).

| Feature ID | Persona | Use Case | Feature Name | User Story | UX Flow Summary | Innovation (1-5) | Effort | Impact | KPI | Status |
|:---|:---|:---|:---|:---|:---|:---:|:---:|:---:|:---|:---|
| P-001 | All | Know if the API is up | Health & source-circuit endpoints | As any user, I want a liveness + per-source circuit probe, so that I know whether data is flowing. | `GET /health`, `/v1/health/sources` → green/red pills (F-002,F-003,F-078) | 1 | S | M | p99 uptime ≥ 99.5%; circuit states visible | shipped |
| P-002 | Researcher, Analyst | Find what data exists | Catalogued, filterable dataset browser | As a user, I want to browse/search/filter datasets by category·tag·cadence·source·freetext, so that I find the series I need. | `/v1/sources`, `/v1/categories`; dashboard Browse panel (F-004→F-013, F-073→F-077) | 2 | S | M | dataset browse session ≥ 90s; filter usage > 40% of visits | shipped |
| P-003 | All | Read the records behind a finding | Paginated cached record reads | As a user, I want paginated records from cache with clamped limits, so that I can inspect source rows fast. | `/v1/datasets/{src}/{ds}/records?offset=&limit=` (F-013) | 1 | S | M | read p99 < 50ms; limit-clamp never errors | shipped |
| P-004 | All | Trust a finding's math | Deterministic anomaly detection (8 detectors) | As a user, I want findings produced by pure-Rust detectors (series_jump, outlier, seasonality, correlation, cross_source_gap, YoY, proxy_divergence, benchmark_deviation), so that they are reproducible & citable. | agent scan pass → `analysis.rs` → Insight (F-039→F-048) | 5 | L | H | 100% findings reproducible in CI; ≥3 detectors w/ real-finding (EXAMPLES.md) | shipped |
| P-005 | All | Verify a claim to source rows | Evidence pointers on every Insight | As a user, I want each insight to link back to its source record_ids/fields/values, so that I can verify before acting/publishing. | card evidence drawer (F-050,F-069,F-070) | 5 | M | H | evidence open-rate > 35%; 0 unverifiable claims shipped | shipped |
| P-006 | Journalist, Analyst | See the day's top stories first | Ranked daily brief (experimental-discounted) | As a user, I want a ranked top-N brief so I see the most important findings first, with experimental findings fairly discounted. | `/v1/brief?limit=` → brief hero (F-015,F-067) | 3 | M | H | brief CTR (click-thru to detail) > 50%; brief is first scroll for > 70% sessions | shipped |
| P-007 | All | Triages insights by urgency | Severity-filtered insights feed | As a user, I want to filter insights by critical/warning/info, so that I triage what matters now. | feed sev buttons (F-068) | 1 | S | M | filter used in > 30% of sessions | shipped |
| P-008 | Journalist, Researcher | Ask follow-up questions | Natural-language Q&A (`/v1/ask`) | As a user, I want to ask the agent a question in English and get an answer with a confidence + tool-call trace, so that I can investigate without curl. | chat rail → `POST /v1/ask` (F-017→F-019, F-072) | 4 | M | H | /ask used in > 25% sessions; answer confidence correlates w/ 👍 | shipped |
| P-009 | Journalist, Researcher | Understand how the agent reasoned | Agentic investigation loop w/ tool trace | As a user, I want to see which deterministic tools the agent called and in what order, so that I trust the reasoning. | trace drawer in chat (F-052→F-056) | 4 | M | M | trace expanded on > 20% of /ask answers | shipped |
| P-010 | Analyst | Get pushed when something breaks | Severity-filtered, deduped webhook alerting | As Marcus, I want insights ≥ my severity threshold pushed to my webhook (deduped by id), so that I never miss a liquidity event. | `AlertDispatcher` → `WebhookSink` (F-057→F-060) | 3 | M | H | alert delivery success ≥ 99.5%; alert-to-action latency < 60s post-finding | shipped |
| P-011 | Analyst, Watchdog | Receive alerts by email | Email alert sink | As a user, I want critical insights emailed, so that I get them without a webhook receiver. | `EmailSink` (F-061) | 2 | S | M | email alert open-rate > 40% | shipped |
| P-012 | All | Teach the system what's useful | Insight feedback (👍/👎 + note) | As a user, I want to mark an insight useful/not, so that ranking & detection quality improves over time. | card 👍/👎 → `POST /feedback` → `GET /feedback` net-useful (F-020,F-021,F-071) | 3 | S | M | feedback on > 8% of viewed insights; net-useful rising trend | shipped |
| P-013 | All | Read insights in a browser | Insights-first dashboard (brief·feed·evidence·chat·browse·health) | As a user, I want a single readable dashboard, so that I don't need curl to consume the product. | `dashboard/index.html` at `/dashboard` (F-064→F-080) | 2 | M | H | dashboard is primary surface for > 80% of non-API users; bounce rate < 25% | shipped |
| P-014 | Analyst, Researcher | Integrate findings into tooling | Typed Python client (`pip install hkgov-py`) | As a power user, I want a typed Python client over the HTTP API, so that I can script insights into my workflow. | `examples/`, `python/` (F-087) | 2 | S | M | client download/install growth; client-driven API calls > 20% | shipped |
| P-015 | All | Run without external services | Zero-config heuristic mode (no LLM key) | As a user, I want insights to work with zero external dependencies, so that the bar for quality doesn't depend on my API key. | `HeuristicClient` default (F-051) | 5 | M | H | ≥ 95% of features usable with no key; keyless parity measured | shipped |
| P-016 | Ops | Scale to fleet concurrency | Pluggable RecordStore (memory→redis→pg) | As an operator, I want to scale from one node to 100k concurrency by changing a config, so that I don't refactor. | `RecordStore` trait swap (A3) | 3 | L | H | linear scaling verified in k6; no refactor for 10× growth | shipped |
| P-017 | Ops | Secure the API | Optional API-key auth + middleware stack | As an operator, I want optional X-API-Key auth + timeout/gzip/CORS/load-shed, so that I can expose the API safely. | `auth.rs`, tower stack (F-022→F-026) | 2 | S | M | 0 auth bypasses (D-005 fixed); auth on in all multi-tenant deploys | shipped |
| P-018 | Ops | Try it in 60 seconds | One-shot demo + Docker image | As an evaluator, I want a one-command demo + a 30MB Docker image, so that I can try it without a Rust toolchain. | `scripts/demo.sh`, `Dockerfile` (F-086,F-088) | 2 | S | M | time-to-first-insight < 90s for a new evaluator | shipped |

---

## B. Phase-2 ideation pipeline (innovation candidates)

> Rationale + "Aha!" moments: [`PHASE_2_IDEATION.md`](./PHASE_2_IDEATION.md).
> Full step-by-step UX flows: [`PHASE_3_UX_STORYBOARD.md`](./PHASE_3_UX_STORYBOARD.md).
> All 8 candidates **approved** at the Phase-2 checkpoint and advanced to
> Phase-3 design — see §C/D for their `UX Flow Summary` (filled) and
> Phase 4 for Acceptance Criteria + RICE.

## C. Approved features (post Phase-2 checkpoint)

> All 8 candidates (`P-100`→`P-107`) approved by the user at the Phase-2
> checkpoint. UX flows designed in Phase 3; the `UX Flow Summary` column below
> is a one-line pointer to the full storyboard section.

| Feature ID | Persona | Use Case | Feature Name | User Story | UX Flow Summary | Innovation (1-5) | Effort | Impact | KPI | Status |
|:---|:---|:---|:---|:---|:---|:---:|:---:|:---:|:---|:---|
| P-100 | Priya, Maya, Chen (G2,G6) | Productize the thesis: quantify how much HKGOV didn't explain | **Silence Index** — opacity, scored | As Priya, I want a reproducible per-source/quarter "silence index" built from cross_source_gap + unattributed series_jump, so that transparency is a citable number, not a tagline. | Header pill → hero gauge (0-100 + Δqtr) + quarterly sparkline → breakdown table (gap/unattributed-jump/missing-days) → each row links EvidenceDrawer → export quarterly report (P-101) + permalink. [§3.1] | 5 | M | H | Silence Index becomes most-shared artifact; cited in ≥3 external analyses/yr; quarterly-report download growth | approved→designed |
| P-101 | Chen, Maya, Priya (G6) | Bridge "found" → "published/acted" with a verifiable artifact | **Cite-It** — citation-grade export | As Maya, I want a permalink + BibTeX + PDF card + CI-reproducible manifest from any insight, so that I can publish a defensible story fast. | Card `📎 Cite` → slide-over drawer: permalink + format tabs (BibTeX/RIS/APA/Chicago/MD) + card preview (PDF/PNG/OG+QR) + collapsible reproducibility manifest → copy/download Toast. [§3.2] | 5 | M | H | cite-action on >15% of detail views; ≥1 cited artifact in published paper/article within 2 qtrs | approved→designed |
| P-102 | Marcus, Priya, Maya (G3,G4) | Consumer-grade push alerts with no infra | **Signal Subscriptions** — NL signal authoring + multi-sink push | As Marcus, I want to author a signal in English and get it pushed to my phone/email/Slack, so that I never miss an event without standing up a webhook. | Ask-answer `🔔 Save as signal` OR My Signals `+ New` → NL→scan-target confirmation card → `👁 Preview` (90d match count, reuses P-107) → channel select (email/Telegram/Slack/RSS + verify) → save → fire pushes insight+evidence+permalink; history in panel. [§3.3] | 4 | L | H | ≥30% WAU with ≥1 active subscription; sub-triggered session return-rate >50% | approved→designed |
| P-103 | Marcus, Chen, Maya (G5) | Tell me how rare an event actually is | **Unprecedentedness Score** — historical-context layer | As Marcus, I want a percentile rank + normal-range band + "last time" comparator on each insight, so that I know whether to act. | Zero-click: UnprecedentednessBand renders on every numeric card (median±MAD envelope + min/max + value marker + percentile chip) + "last exceeded {date} — view⟩" comparator link (opens prior insight inline, P-104); brief ranker weights rarity. [§3.4] | 4 | M | M | band on >90% of numeric insights; comparator clicked on >20% of cards | approved→designed |
| P-104 | All (G2,G10) | Make the feed a living, delta-aware thread | **Insight Lifeline** — persistent, watched, "what's new" | As a returning user, I want read/unread + new-since-last-visit + evolution tracking, so that I triage deltas, not the whole feed. | Return-visit banner ("N new · M evolved · show only new") → feed tabs [all|🆕new|🔔watching] → unread accent+dot (open/dwell marks read, title-count) → `🔔 Watch` tracks evolution → re-fire same id with changed evidence stamps `evolved` + diff. [§3.5] | 3 | M | H | returning 7-day retention +20pts; "what's new" first interaction >60% return visits | approved→designed |
| P-105 | Maya, Chen (G7,G11) | Turn an insight into a resumable case file | **Drill-In Investigation** — guided, saved probe from a card | As Maya, I want to Investigate an insight and get a saved, shareable case file with related series + parallels, so that I work a story across days. | Card `🔍 Investigate` → case-file view, insight=step0 → 3 one-click suggested steps (related series/parallels/cross-source) → free-form Q&A appends numbered steps → per-step notes → permalink/save/branch; `📁 Cases` dropdown resumes; share=read-only. [§3.6] | 4 | L | M | investigations resumed/branched on >25% of created cases; avg steps ≥4 | approved→designed |
| P-106 | Priya (G9) | Unlock the bilingual HK-public audience | **Bilingual Surface** — zh-HK dashboard + insights | As Priya, I want the dashboard + summaries + exports in Chinese, so that the HK public can read the evidence. | Header `[中/EN]` toggle (persists, defaults to Accept-Language) → localized chrome via i18n dict → LLM-framed zh summaries (heuristic=bilingual templates) with ModeBadge → P-101 cite cards + P-100 index render zh-HK; evidence values untouched. [§3.7] | 3 | M | M | zh-HK surface >25% of dashboard sessions post-launch; bilingual-artifact share-rate | approved→designed |
| P-107 | Chen, Marcus (G8) | Remove the config+restart barrier to tuning | **Detector Studio** — self-serve scan authoring | As Chen, I want to drag a threshold and preview against live data, so that I tune detection without editing config.toml + restarting. | `▸ Detector Studio` panel → 3-pane composer (dataset/field · detector+threshold slider · live preview) → drag slider live-updates "fires N times / 90d" + matched-dates plot → save-as-signal (P-102) or default scan; `🔗 Share` permalink encodes config. [§3.8] | 2 | M | M | self-authored scans >15% of active targets; preview→save conversion >40% | approved→designed |
| P-108 | All (G1) | First-run onboarding + per-user state foundation + inbound share-landing *(surfaced in Phase-5 validation)* | **Identity Tier + Onboarding** — the unstated prerequisite | As Priya, I want a guided first-run + a per-user identity so my signals/cases/watch-state survive reload, so that the product works for me personally. | First-run 3-step coachmark (what this does → first insight → enable push) → progressive disclosure of Cite/Watch/Investigate until first insight viewed → email/magic-link identity (no OAuth yet) → inbound cite-permalink opened by a non-user renders read-only first-run view + "explore more" nudge (closes the virality loop). [§5.1 D-1/2/3] | 3 | M | H | first-run→insight-viewed rate >70%; inbound-landing→dashboard CTR >15%; per-user state retention | specified |
| P-109 | Marcus, Priya (G1) | Mobile parity for push-driven flows *(surfaced in Phase-5 validation)* | **Mobile Parity** — responsive storyboard for push recipients | As Marcus, I want to triage a pushed insight on my phone, so that I act where I was pinged, not at a desk. | Responsive storyboard for Silence Index gauge, Cite drawer, Signal composer; bottom-tab IA on <700px (Insights / Ask / Signals); evidence drawer stacks full-width; cite/social-card download native. [§5.1 D-4] | 2 | M | M | mobile sessions >30% post-launch; mobile signal-fire→open <60s median | specified |

## D. Designed features (post Phase-3 UX storyboard)

> The 8 approved features now carry full UX flows (above in §C). Phase 4 added
> formal Acceptance Criteria + RICE + finalized KPIs → see §E.

## E. Specified backlog (post Phase-4 RICE + Phase-5 validation)

> Final ranked backlog, **post-validation**. The Phase-5 recursive loop
> ([`PHASE_5_VALIDATION.md`](./PHASE_5_VALIDATION.md)) surfaced a hidden
> cross-cutting prerequisite (identity) and a mobile-parity gap, spawning
> **P-108** and **P-109**, re-scoping **P-100** (honestly HKMA-scoped v1),
> re-wording **P-006/P-012**'s KPI (feedback now honestly feeds the brief
> ranker), and re-ordering the plan. AC = Acceptance Criteria (full text in
> [`PHASE_4_PRD.md`](./PHASE_4_PRD.md) §4.2 + Phase-5 refinements §5.1).

| Rel | Order | ID | Feature | User Story (short) | AC count | RICE | Effort (PM) | KPI | Status |
|:---:|:---:|:---|:---|:---|:---:|---:|:---:|:---|:---|
| R1 | 1 | **P-108** | Identity + Onboarding + Inbound Landing *(Phase-5)* | first-run coachmark; email/magic-link identity; inbound cite-permalink first-run view | 6 | 8,000 | 4 | first-run→insight-viewed >70%; inbound→dashboard CTR >15% | specified |
| R1 | 2 | **P-104** | Insight Lifeline | persist; read/unread; "what's new"; watch+evolve (AC names identity dep; client-side fallback) | 6 | 10,000 | 4 | 7-day retention +20pts; "what's new" first interaction >60% | specified |
| R1 | 3 | **P-103** | Unprecedentedness Score | rarity band + percentile + "last exceeded" comparator on numeric insights | 5 | 10,667 | 3 | band on >90% numeric; comparator click >20% | specified |
| R2 | 4 | **P-101** | Cite-It | permalink + citations + PDF/PNG/OG + CI-repro manifest (+ inbound-landing AC) | 9 | 4,000 | 3 | cite-action >15% detail views; ≥1 pub cite/2qtrs | specified |
| R2 | 5 | **P-100** | **HKMA** Silence Index v1 *(re-scoped)* | per-quarter opacity score (HKMA-scoped) + sparkline + breakdown + report | 8 | 12,000 | 4 | most-shared artifact; cited ≥3 ext analyses/yr; report growth | specified |
| R3 | 6 | **P-106** | Bilingual Surface | zh-HK dashboard + summaries + exports | 4 | 5,333 | 3 | zh-HK >25% sessions; bilingual share-rate | specified |
| R3 | 7 | **P-102** | Signal Subscriptions | NL→scan-target + 90d preview + multi-sink push (AC names identity dep) | 9 | 4,571 | 7 | ≥30% WAU w/ ≥1 sub; sub→return >50% | specified |
| R3 | 8 | **P-109** | Mobile Parity *(Phase-5)* | responsive gauge/cite/signal; bottom-tab IA <700px | 5 | 5,333 | 3 | mobile >30% sessions; mobile signal→open <60s | specified |
| R3 | 9 | **P-107** | Detector Studio | 3-pane composer + live preview + save/share | 6 | 533 | 3 | self-authored >15% targets; preview→save >40% | specified |
| R3 | 10 | **P-105** | Drill-In Investigation | case file + guided steps + resume/branch/share (AC names identity dep) | 6 | 533 | 6 | resumed/branched >25% cases; avg steps ≥4 | specified |

**Release totals (post-validation):** R1 = 11 PM · R2 = 7 PM · R3 = 16 PM ·
**Total ≈ 34 PM** (was 27; +7 PM is the cost of the cohesion criteria passing).
**Parallel non-feature workstream:** data.gov.hk resource expansion (flagship
fuel) — chartered alongside R1/R2; gate of "≥10 datasets, ≥3 sources" before
P-100's public launch.
**North Star:** *verified insights acted on / week* (cite · push-signal ·
investigate · vote-useful). KPI tree in `PHASE_4_PRD.md` §4.5.
**Phase-5 honesty fixes:** P-012 feedback KPI reworded → "feeds the brief ranker
as a relevance signal" (not "improves detection quality"); P-006 brief ranker now
consumes net-useful as a tie-breaker (wired in R1).

---

## F. Verification cross-reference

> How this strategy doc relates to the QA tracker
> ([`../FEATURES_TRACKER.md`](../../FEATURES_TRACKER.md)). When a feature ships,
> its AC (§E) becomes one or more `F-###` rows there, and this tracker's
> `Status` moves `specified → shipped`.

- `FEATURES_TRACKER.md` = *does the implemented behaviour work?* (QA)
- `PRODUCT_STRATEGY_TRACKER.md` = *should it exist & did it move the metric?*
  (strategy)
- Both share the `P-###`/`F-###` convention; a shipped `P-###` decomposes into
  `F-###` stories at build time.

## C. Approved features (post Phase-2 checkpoint)

*(empty until checkpoint)*

## D. Designed features (post Phase-3 UX storyboard)

*(empty — UX Flow Summary column filled in Phase 3)*

## E. Specified backlog (post Phase-4 RICE prioritization)

*(empty — final ranked backlog with Acceptance Criteria in Phase 4)*
