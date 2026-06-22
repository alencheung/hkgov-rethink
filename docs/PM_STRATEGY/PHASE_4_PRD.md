# Phase 4 — Requirements Engineering & Prioritization (PRD)

> **Status:** Phase 4 complete — **🛑 CHECKPOINT: final PRD, awaiting approval.**
> This document converts the Phase-3 UX flows into formal User Stories with
> Acceptance Criteria (AC), RICE-scores the full backlog (base + new), finalizes
> KPIs, and produces the ranked release plan. The canonical spreadsheet §E
> carries the finalized rows.

---

## 4.1 RICE methodology & calibration

**RICE = (Reach × Impact × Confidence) / Effort.** Higher = ship sooner.

| Factor | Scale | This product's calibration |
|:---|:---|:---|
| **Reach** | users/quarter | Distinct users meaningfully exposed per quarter. Base scale: small product → estimated active-user quarters. Conservative bands: S=1k, M=5k, L=20k, XL=50k+. |
| **Impact** | 0.25 / 0.5 / 1 / 2 / 3 | 3=massive (retention/moat), 2=high, 1=medium, 0.5=low, 0.25=minimal. |
| **Confidence** | 50% / 80% / 100% | 100%=data-backed; 80%=strong analogue; 50%=speculative. Discounted for new-market risk. |
| **Effort** | person-months (PM) | S≤2, M=3–5, L=6–10, XL>10. Includes eng+design+test, grounded in the existing codebase (detectors/store/API already exist). |

> **Guiding rule:** the *detectors* and *determinism guarantee* are the spine —
> nothing in this backlog weakens them. RICE rewards features that **compose
> from existing atomic capabilities** (A1–A10); it penalizes anything that
> introduces a new opaque-LLM path.

---

## 4.2 Formal user stories + acceptance criteria

Each story: `US-{feature}-{n}`. AC items are testable (will map to
`FEATURES_TRACKER.md` rows when built).

### P-100 — Silence Index

**US-100-1** — As Priya, I want to open the Silence Index from anywhere, so that
I see HKGOV's transparency score without navigating.
- **AC1:** a `🔗 Silence Index` pill in the header opens the view in ≤1 click.
- **AC2:** the view renders within 500ms of cached data; skeleton otherwise.
- **AC3:** deep-link `/silence-index?period=YYYY-Qn` is stable & shareable.

**US-100-2** — As a user, I want a per-source/quarter score derived *only* from
deterministic findings, so that it's reproducible & citable.
- **AC1:** score = weighted rollup of `cross_source_gap` + unattributed
  `series_jump` + missing-data days, computed in pure Rust (no LLM).
- **AC2:** same data in → same score out (unit test with golden fixtures).
- **AC3:** methodology version stamped (`Silence Index v1.x`).

**US-100-3** — As Maya, I want to drill from a score into the exact missing
dates, so that I can verify the claim.
- **AC1:** each breakdown row links its source `Insight` → EvidenceDrawer.
- **AC2:** evidence lists the specific dates (capped display, full on expand).

**US-100-4** — As Priya, I want to export a quarterly transparency report, so
that I can publish/share it.
- **AC1:** `📥 Export` produces a PDF with gauge + sparkline + breakdown +
  reproducibility manifest (reuses P-101 engine).
- **AC2:** PDF is byte-deterministic for the same input (CI-reproducible).

---

### P-101 — Cite-It

**US-101-1** — As Maya, I want a stable permalink for any insight, so that I can
reference it in a published story.
- **AC1:** every persisted insight has `/{cite,id}` URL; returns 200 with the
  insight + evidence, or 404 if unknown.
- **AC2:** permalink survives server restart (persisted tier).

**US-101-2** — As Chen, I want citation strings in my reference format, so that
I can drop them into a paper.
- **AC1:** tabs render BibTeX / RIS / APA / Chicago / Markdown; each copyable.
- **AC2:** experimental insights auto-suffix the honesty marker.

**US-101-3** — As a user, I want a CI-reproducible manifest, so that my citation
is defensible.
- **AC1:** manifest = `{dataset_version, detector, threshold, data_sha256,
  runtime_version}`; reproducible from the manifest reproduces the finding.
- **AC2:** a `⚠ reproduces on data as of {ts}` chip appears if live data has
  drifted from the manifest hash; never false-claims reproducibility.

**US-101-4** — As Priya, I want a shareable card, so that I can post a finding.
- **AC1:** PDF + PNG + OG image render (severity color, title, summary, top
  evidence, permalink QR, manifest hash).
- **AC2:** OG image has correct social meta tags for link previews.

---

### P-102 — Signal Subscriptions

**US-102-1** — As Marcus, I want to author a signal in plain English, so that I
don't write code.
- **AC1:** NL input → LLM compiles to a scan target; shows a confirmation card
  with the derived detector/field/threshold in plain English.
- **AC2:** **detector stays deterministic** — LLM only translates intent→config.
- **AC3:** ambiguous intent → 2–3 radio-options, never a guess.

**US-102-2** — As Marcus, I want a heuristic-mode path, so that signals work
with no LLM key.
- **AC1:** with no LLM, a form-based builder (dataset→field→detector→threshold)
  is offered; fully functional.

**US-102-3** — As a user, I want to preview matches before subscribing, so that
I calibrate sensitivity.
- **AC1:** `👁 Preview` runs the compiled detector over last 90d; shows match
  count + dates (reuses P-107 preview).
- **AC2:** 0-match preview shows a "lower threshold" hint.

**US-102-4** — As Marcus, I want signals pushed to my channel, so that I act
fast.
- **AC1:** sinks: email / Telegram / Slack / RSS, each with one-time verify.
- **AC2:** a fired signal pushes insight + evidence + permalink to all verified
  sinks; dispatch logged (reuses `/v1/alerts`).
- **AC3:** a failing sink doesn't abort others (F-062 guarantee); status shown.

---

### P-103 — Unprecedentedness Score

**US-103-1** — As Marcus, I want a rarity band on every numeric insight, so that
I gauge severity.
- **AC1:** UnprecedentednessBand renders when history ≥12 points: median±MAD
  envelope, min/max, value marker, percentile chip.
- **AC2:** <12 points → muted "not enough history (n=…)" line; never a broken
  band.

**US-103-2** — As Maya, I want a "last exceeded" comparator, so that I find
prior parallels.
- **AC1:** comparator line links the prior insight (P-104 persistence); opens
  inline without losing place.

**US-103-3** — As a user, I want the brief to surface truly rare events, so that
the ranking reflects newsworthiness.
- **AC1:** brief ranker weights `unprecedentedness` transparently; "why am I
  seeing this" tooltip exposes the rarity factor.

---

### P-104 — Insight Lifeline

**US-104-1** — As a returning user, I want a "what's new" banner, so that I
triage deltas.
- **AC1:** on return (last-visit >1h), banner shows new + evolved counts since
  last-visit; `show only new` toggles the `new` feed tab.
- **AC2:** server-stamped last-visit (consistent cross-device); client fallback
  if unauthenticated.

**US-104-2** — As a user, I want read/unread state, so that I don't re-read.
- **AC1:** new insights show accent + dot; opening EvidenceDrawer or 1s dwell
  marks read; unread count in document title.

**US-104-3** — As Maya, I want to watch an insight and see it evolve, so that I
track a developing story.
- **AC1:** `🔔 Watch` adds to `watching` tab; re-fire of same id with changed
  evidence stamps `evolved {date}` + collapsible diff.
- **AC2:** dedup by finding id (F-059) — evolution never duplicates a card.

---

### P-105 — Drill-In Investigation

**US-105-1** — As Maya, I want to Investigate from a card, so that I start a
case file in one click.
- **AC1:** `🔍 Investigate` opens a case-file view with the insight as step 0.

**US-105-2** — As Maya, I want guided next steps, so that I don't stare at a
blank thread.
- **AC1:** agent offers 3 one-click suggested steps (related series / parallels
  / cross-source), each runs a deterministic tool (ToolBelt) and appends a
  numbered evidence-backed step.
- **AC2:** hitting `max_steps` shows a soft stop with `[continue]` (F-056).

**US-105-3** — As Chen, I want to resume/share/branch a case file, so that I
work across days.
- **AC1:** permalink `/investigate/{case_id}`; `💾 Save` + `📁 Cases` dropdown;
  `⑃ Branch` from any step; `Share` = read-only link.

---

### P-106 — Bilingual Surface

**US-106-1** — As Priya, I want the dashboard in Chinese, so that I can read it.
- **AC1:** `[中/EN]` toggle persists; defaults to `Accept-Language`; no reload
  on toggle.
- **AC2:** all chrome strings localized via i18n dict; missing string → English
  fallback with `(en)` marker (never blank).

**US-106-2** — As Priya, I want insight summaries in zh-HK, so that I understand
the finding.
- **AC1:** LLM frames zh summary; heuristic mode → bilingual templates
  (deterministic; no LLM key needed). ModeBadge shows source.
- **AC2:** source evidence values (numbers/dates/bilingual press titles) are
  never machine-translated — rendered verbatim.

---

### P-107 — Detector Studio

**US-107-1** — As Chen, I want to compose a scan target in-product, so that I
avoid config+restart.
- **AC1:** 3-pane composer (dataset/field · detector+threshold slider · live
  preview); form-only, fully heuristic (no LLM).
- **AC2:** invalid combos (e.g. `correlation` w/o `field_b`) disable Save with
  inline validation.

**US-107-2** — As Chen, I want live preview, so that I calibrate before saving.
- **AC1:** dragging the slider live-updates "fires N times / 90d" + matched-dates
  plot; 0-match → hint, >500 → cap+warn.

**US-107-3** — As Chen, I want to save/share my detector, so that peers reuse it.
- **AC1:** `Save as signal` (→ P-102) or `Save as default scan`; `🔗 Share`
  permalink encodes config (`/studio?cfg=…`).

---

## 4.3 RICE scoring — full backlog (base + new)

> Base features (P-001–P-018) are scored at their *incremental* value today
> (already shipped → Reach/Impact reflect maintenance/optimization, not new
> ships). New features (P-100+) reflect projected first-year impact.

| ID | Feature | Reach (qtr) | Impact | Conf. | Effort (PM) | **RICE** | Tier |
|:---|:---|---:|:---:|:---:|---:|---:|:---|
| **P-100** | Silence Index | 20,000 | 3 | 80% | 4 | **12,000** | 🥇 flagship |
| **P-101** | Cite-It | 5,000 | 3 | 80% | 3 | **4,000** | 🥈 moat |
| **P-102** | Signal Subscriptions | 20,000 | 2 | 80% | 7 | **4,571** | 🥉 retention |
| **P-103** | Unprecedentedness Score | 20,000 | 2 | 80% | 3 | **10,667** | 🥇 |
| **P-104** | Insight Lifeline (persistence) | 20,000 | 2 | 100% | 4 | **10,000** | 🥇 |
| **P-105** | Drill-In Investigation | 2,000 | 2 | 80% | 6 | **533** | power-user |
| **P-106** | Bilingual Surface | 20,000 | 1 | 80% | 3 | **5,333** | reach |
| **P-107** | Detector Studio | 2,000 | 1 | 80% | 3 | **533** | power-user |
| P-004 | Deterministic detectors (spine) | 20,000 | 3 | 100% | (sunk) | — | protect |
| P-005 | Evidence pointers (spine) | 20,000 | 3 | 100% | (sunk) | — | protect |
| P-015 | Heuristic mode (spine) | 20,000 | 3 | 100% | (sunk) | — | protect |
| P-006 | Daily brief | 20,000 | 2 | 100% | (sunk) | — | protect |
| P-008 | `/v1/ask` NL Q&A | 20,000 | 2 | 100% | (sunk) | — | protect |
| P-013 | Dashboard | 20,000 | 2 | 100% | (sunk) | — | protect |
| P-016 | RecordStore scaling | 5,000 | 2 | 100% | (sunk) | — | protect |
| P-010 | Webhook alerting | 2,000 | 2 | 100% | (sunk) | — | protect |
| P-002 | Dataset browser | 5,000 | 1 | 100% | (sunk) | — | protect |
| P-012 | Feedback | 20,000 | 1 | 100% | (sunk) | — | protect |
| P-014 | Python client | 2,000 | 1 | 100% | (sunk) | — | protect |
| P-017 | Auth + middleware | 5,000 | 1 | 100% | (sunk) | — | protect |
| P-001 | Health endpoints | 20,000 | 0.5 | 100% | (sunk) | — | protect |
| P-003 | Record reads | 5,000 | 1 | 100% | (sunk) | — | protect |
| P-007 | Severity filter | 20,000 | 0.5 | 100% | (sunk) | — | protect |
| P-009 | Agent loop + trace | 2,000 | 1 | 100% | (sunk) | — | protect |
| P-011 | Email sink | 2,000 | 1 | 100% | (sunk) | — | protect |
| P-018 | Demo + Docker | 5,000 | 0.5 | 100% | (sunk) | — | protect |

> RICE work-sheet (so the numbers are auditable):
> P-100 = (20,000 × 3 × 0.8) / 4 = 12,000 · P-103 = (20,000 × 2 × 0.8) / 3 = 10,667
> · P-104 = (20,000 × 2 × 1.0) / 4 = 10,000 · P-106 = (20,000 × 1 × 0.8) / 3 = 5,333
> · P-102 = (20,000 × 2 × 0.8) / 7 = 4,571 · P-101 = (5,000 × 3 × 0.8) / 3 = 4,000
> · P-105 = (2,000 × 2 × 0.8) / 6 = 533 · P-107 = (2,000 × 1 × 0.8) / 3 = 533.

---

## 4.4 Ranked release plan (the backlog, ordered)

Three releases, each independently shippable & each a coherent narrative beat.
**Dependencies drive the ordering** as much as RICE: P-101/P-105/P-100-export
depend on P-104 persistence; P-106 benefits from P-100 existing.

### Release R1 — "Make it stick" (persistence + context) — 7 PM
*The spine for everything else. Turns ephemeral findings into a citable, rarity-
ranked, delta-aware record.*

| Order | ID | Feature | PM | Why this order |
|:---:|:---|:---|:---:|:---|
| 1 | **P-104** | Insight Lifeline (persist + read/unread + evolve) | 4 | **Unblocks** P-101 cites, P-103 comparator, P-105 cases, P-100 reports. Highest-confidence (100%). |
| 2 | **P-103** | Unprecedentedness Score | 3 | Composes from existing MAD math + P-104 history; zero-click delight on every numeric card. |

**R1 exit metric:** returning 7-day retention +20pts; band renders on >90% of
numeric insights.

### Release R2 — "Make it citable & quantified" (the flagship) — 7 PM
*The brand-defining release. Productizes the thesis and builds the citation moat.*

| Order | ID | Feature | PM | Why this order |
|:---:|:---|:---|:---:|:---|
| 3 | **P-101** | Cite-It (permalink + citations + manifest + card) | 3 | Depends on P-104 persistence. The moat; also the export engine P-100 reuses. |
| 4 | **P-100** | Silence Index (score + sparkline + breakdown + report) | 4 | Depends on P-101's export engine. The flagship — makes the thesis a citable number. |

**R2 exit metric:** ≥1 cited artifact in a published paper/article within 2
quarters; Silence Index is the most-shared artifact; cited in ≥3 external
analyses/yr (trajectory).

### Release R3 — "Make it reach & recur" (growth + depth) — 13 PM
*Retention engine + audience multiplier + power-user depth.*

| Order | ID | Feature | PM | Why this order |
|:---:|:---|:---|:---:|:---|
| 5 | **P-106** | Bilingual Surface (zh-HK) | 3 | Multiplies P-100's civic reach; no deep deps. |
| 6 | **P-102** | Signal Subscriptions (NL + multi-sink push) | 7 | Highest-impact retention; largest effort — parallelize with R3's other two. Embeds P-107's preview. |
| 7 | **P-107** | Detector Studio (standalone) | 3 | Its preview already shipped inside P-102; the standalone studio is the power-user capstone. Low RICE but completes the workbench. |
| 8 | **P-105** | Drill-In Investigation | 6 | Power-user depth; depends on P-104 + ToolBelt (exists). Ships last (lowest RICE among new). |

**R3 exit metric:** ≥30% WAU with ≥1 active signal; zh-HK >25% of sessions;
investigations resumed on >25% of created cases.

**Total: ~27 PM across 3 releases.** Spine features (P-004/P-005/P-015/…) are
protected, not re-shipped.

---

## 4.5 KPI tree (every feature ties to a metric)

**North Star:** *verified insights acted on per week* (acted on = cited,
exported, pushed-as-signal, investigated, or voted useful). This captures the
whole product thesis in one number.

```
                verified insights acted on / week  (North Star)
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
   DISCOVERY             VERIFICATION           ACTION/REACH
 brief CTR               evidence open-rate     cite-action rate
 feed sev-filter use     manifest-repro %       signal subscription rate
 unprecedentedness band  "last exceeded" click  signal fire→return
                              │                     │
                              ▼                     ▼
                         RETENTION              BRAND MOAT
                         7-day retention        Silence Index shares
                         "what's new" use       external citations/qrtr
                         watch→evolved CTR      zh-HK session share
```

| Feature | Primary KPI (from spreadsheet) | North-Star contribution |
|:---|:---|:---|
| P-100 Silence Index | most-shared artifact; cited in ≥3 ext analyses/yr | brand moat → discovery |
| P-101 Cite-It | cite-action >15% of detail views; ≥1 pub cite/2qtrs | **direct** — a cite = an "act" |
| P-102 Signal Subs | ≥30% WAU w/ ≥1 sub; sub→return >50% | **direct** — a push = an "act" |
| P-103 Unprecedentedness | band on >90% numeric; comparator click >20% | verification |
| P-104 Insight Lifeline | 7-day retention +20pts; "what's new" >60% first | retention |
| P-105 Drill-In | resumed/branched >25% of cases; avg steps ≥4 | **direct** — an investigation = an "act" |
| P-106 Bilingual | zh-HK >25% sessions; bilingual share-rate | reach |
| P-107 Detector Studio | self-authored >15% targets; preview→save >40% | discovery (coverage) |

---

## 4.6 Risk register (what could sink a release)

| Risk | Likelihood | Impact | Mitigation |
|:---|:---:|:---:|:---|
| P-104 persistence (Postgres tier) is on the roadmap but **not shipped** — blocks R1. | High | High | R1 *is* P-104; sequence nothing ahead of it. Reuse the existing `--features pg` PgStore. |
| Silence Index methodology is contested ("your opacity score is biased"). | Med | High | Ship the methodology version + full evidence drill-down (AC2/AC3); let critics *reproduce* it. Determinism is the defense. |
| NL signal authoring (P-102) mis-compiles intent → wrong alerts. | Med | Med | Mandatory confirmation card + 90-day preview before save (US-102-1/3); deterministic execution means the *preview is exactly what will fire*. |
| zh-HK framing quality (P-106) uneven in heuristic mode. | Med | Low | Bilingual templates reviewed by a native speaker; ModeBadge sets expectations; LLM mode is the upgrade path. |
| Cite manifest can't reproduce after upstream data revision. | Med | Med | `⚠ reproduces on data as of {ts}` chip (US-101-3) — honest degradation, never false reproducibility. |
| P-105/P-107 low RICE — opportunity cost vs other work. | High | Low | Sequenced last (R3.7/3.8); can be deferred a cycle without breaking the narrative. |

---

## 4.7 Exit-criteria check

- [x] All 8 approved features have formal User Stories with testable AC (§4.2).
- [x] Full backlog RICE-scored (§4.3); base features marked protected/sunk.
- [x] Ranked release plan with dependency-driven ordering (§4.4).
- [x] Every feature tied to a KPI, all rolling up to a North Star (§4.5).
- [x] Risk register with mitigations (§4.6).
- [x] Determinism guarantee preserved across every AC (no black-box claims;
      heuristic-mode fallback specified for P-102, P-106).
- [x] Canonical spreadsheet §E finalized (next edit).

---

> **🛑 CHECKPOINT — Phase 4 (final PRD).** Pausing for approval. Specifically:
> 1. **Release plan** — 3 releases (R1 persistence+context / R2 flagship cite+
>    silence-index / R3 growth+depth), ~27 PM. Does this sequencing & cadence
>    match your capacity & priorities?
> 2. **North-Star metric** — "verified insights acted on / week" (cite · push ·
>    investigate · vote-useful). Is this the right top-line number?
> 3. **Silence Index methodology** — comfortable shipping a public "opacity
>    score" given it will be contested? (Mitigation = full evidence drill-down
>    + determinism.)
> 4. **P-105/P-107** — keep them in R3, or cut to tighten the narrative?
>
> On approval, Phase 5 runs the recursive validation loop (review UX → find
> delight gaps → refine → re-prioritize) and emits the post-iteration health
> summary.
