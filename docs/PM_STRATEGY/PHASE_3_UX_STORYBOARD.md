# Phase 3 — Product Experience & UX Storyboarding

> **Status:** Phase 3 complete. UX flows designed for all 8 approved features
> (P-100 → P-107). Canonical spreadsheet §D populated with step-by-step
> `UX Flow Summary` cells.
> Grounded in the real surface: `dashboard/index.html` (header → brief → feed →
> evidence → chat rail → collapsible browse/health) + the existing `/v1` API.

---

## 3.0 Design system & global UX principles

These apply to *every* flow below — stated once, referenced everywhere.

### Information architecture (evolved, not rewritten)
The current single-screen IA is right (insights-first, secondary panels
collapsed). The 8 features layer **onto** it without a restructure:

```
┌─ HEADER ─────────────────────────────────────────────────────────┐
│  ● status   HK City Pulse — …   [🔗Silence Index]  [zh/en] [↻]   │  ← P-100 nav pill, P-106 toggle
├──────────────────────────────────────────┬──────────────────────┤
│  TODAY'S BRIEF  (rank-ordered)           │  ASK THE AGENT       │
│   [cards… Unprecedentedness band P-103]  │  (chat rail, +Invest.)│ ← P-105 entry
│   ┌─ "what's new since Fri" banner P-104 │                      │
│  ALL INSIGHTS  [all|new|watching|sev…]   │                      │ ← P-104 tabs
│   [card… 📎 Cite · 🔔 Watch · 🔍 Inv. ]  │                      │ ← P-101/P-104/P-105 card actions
│   evidence drawer                        │                      │
│  ▸ MY SIGNALS  (P-102)                   │                      │
│  ▸ BROWSE DATASETS                       │                      │
│  ▸ SYSTEM HEALTH                         │                      │
└──────────────────────────────────────────┴──────────────────────┘
   dedicated pages (modal/inline, not a new SPA):
   /silence-index  (P-100)   /cite/{id}  (P-101)   /studio  (P-107)
```

### Five UX invariants (every flow obeys these)
1. **Determinism stays visible.** Any LLM-framed text (P-102 signal, P-106 zh
   translation) is tagged "framed by AI"; the underlying detector + evidence is
   always one click away. Never present an un-attributed claim.
2. **Evidence-first reading rhythm.** Brief → card → evidence drawer, preserved.
   No new top-level section competes with the brief for the first scroll.
3. **Progressive disclosure.** New power features (cite, investigate, studio)
   live behind a single card action or a collapsed panel — zero added cognitive
   load for Priya, full depth for Maya/Chen.
4. **Friction-budget: ≤2 clicks to value** for any persona's primary action.
   Maya to a citation: card → Cite (1). Marcus to a signal: Ask box → "save as
   signal" (1). Priya to the Silence Index: header pill (1).
5. **Graceful degradation to heuristic mode.** Every feature works with no LLM
   key (P-102 falls back to a form-based signal builder; P-106 falls back to
   bilingual templating). The empty-state copy always tells the user *which*
   mode they're in.

### Reusable component library (named once, reused below)
- **InsightCard** (exists) — extended with: `UnprecedentednessBand`, and a
  `CardActionBar` holding `[📎 Cite] [🔔 Watch] [🔍 Investigate] [👍/👎]`.
- **EvidenceDrawer** (exists) — unchanged; all new features point into it.
- **EmptyState** — standardized: icon + one-line headline + one-line CTA +
  mode-tag. Used by every empty state below.
- **Toast** — transient confirmations (e.g. "signal saved", "citation copied").
- **SkeletonRow** (exists) — used for every async load (preserves the current
  shimmer pattern).
- **ModeBadge** — a small "heuristic"/"LLM" chip in the header showing current
  agent mode (resolves G1's opacity: tells the user *why* the wait).

---

## 3.1 P-100 — Silence Index (flagship)

**Goal:** make "how opaque was HKGOV this period" a single, public, citable
number, trended over time, with drill-down to the exact missing dates.

### End-to-end UX flow
1. **Entry:** user clicks the `🔗 Silence Index` pill in the header (1 click
   from anywhere). Opens a dedicated inline view (not a new tab): a hero gauge
   + a time-series sparkline.
2. **Hero gauge:** a big number `0–100` (higher = more opaque) for the current
   quarter, with a delta vs. previous quarter (`▲ +12` in red, `▼ −8` in green)
   and a one-line plain-English read: *"HKMA left 14 events unexplained this
   quarter — 3rd-highest since 2023."*
3. **Time-series sparkline:** quarterly Silence Index over the available
   history; hover shows the value + the underlying event count. Click a bar →
   drills into that quarter's breakdown (below).
4. **Breakdown table:** for the selected period, the structured events feeding
   the score, each a clickable row linking to its source `Insight`:
   - `cross_source_gap` count (press dates w/ no data row, and vice-versa)
   - unattributed `series_jump` count (big move, no same-day attributing release)
   - missing-data days
   Each row → click → **EvidenceDrawer** (reused) showing the exact dates.
5. **Export:** `📥 Quarterly transparency report` button → generates a
   deterministic PDF (P-101's cite engine reused) with the gauge, sparkline,
   breakdown, and a reproducibility manifest.
6. **Share:** the Silence Index view has a stable permalink
   (`/silence-index?period=2026-Q2`) — the public citable artifact.

### Empty state
- **No findings yet (agent off / cold cache):** EmptyState →
  *"Silence Index is warming up. It's built from cross-source gap + unattributed
  move findings — enable the agent (`HKGOV_AGENT__ENABLED=true`) and it appears
  within ~30s."* + ModeBadge shows current agent state.
- **Agent on, but zero gaps in the period (genuinely transparent):** not an
  error — celebratory EmptyState → *"✨ 0 unexplained events this quarter — every
  press release had a matching data row, every move was attributed. Here's the
  evidence."* (links the clean scan log). This is a *feature*, not a hole.

### Edge cases & error recovery
- **History < 4 quarters:** sparkline hidden, gauge shown with note
  *"trend appears after 4 quarters of history."* No broken/empty chart.
- **Source circuit open (HKMA unreachable):** gauge greys out with a banner →
  *"HKMA feed is down (circuit open). Last computed 2h ago from 142 available
  records — [recompute now] / [view stale]."* Never silently shows stale.
- **Score methodology change:** versioned; the view shows
  `Silence Index v1.2` so a methodology bump doesn't invalidate prior cites.
- **A period with partial data (mid-quarter):** current quarter labeled
  `(partial, 47 days)` and excluded from the historical sparkline until complete.

---

## 3.2 P-101 — Cite-It (citation-grade export)

**Goal:** from any insight → a permalink + BibTeX/RIS + PDF/OG card + a
CI-reproducible manifest, in ≤2 clicks.

### End-to-end UX flow
1. **Entry:** on any InsightCard, the `CardActionBar` gains a `📎 Cite` button.
   Click → opens a **Cite drawer** (slide-over, not a modal) anchored to that
   insight. (1 click.)
2. **Cite drawer contents:**
   - **Permalink:** `https://…/cite/{insight_id}` — copy button, with a
     "expires: never (deterministic)" reassurance line.
   - **Citation format selector:** tabs `[BibTeX] [RIS] [APA] [Chicago] [Markdown]`
     — each renders a copyable string prefilled from the insight's metadata +
     the data-version stamp.
   - **Card preview:** a rendered share card (PDF + PNG + social OG image) —
     severity color, title, one-line summary, the top evidence row, the
     permalink QR, and the reproducibility manifest hash. `[Download PDF] [PNG]
     [Copy social card]`.
   - **Reproducibility manifest:** a collapsible block showing
     `{dataset_version, detector, threshold, data_sha256, runtime_version}` —
     the exact recipe to reproduce this finding in CI. "Copy manifest" → a
     self-contained block a reviewer can paste.
3. **Confirmation:** on copy/download → Toast →
   *"Citation copied — manifest reproduces in CI."*

### Empty state
- **Insight not yet persisted** (pre-Postgres tier): Cite drawer shows
  *"This insight is in-process only. Persistence lands with the Postgres tier —
  for now, copy the evidence directly."* + a fallback "copy evidence JSON" so
  the user isn't blocked. Graceful, not a dead-end.

### Edge cases & error recovery
- **Manifest reproduction fails in CI** (data drifted): the cite drawer shows a
  warning chip `⚠ reproduces on data as of {ts}` — never claims false
  reproducibility. Links the diff.
- **Dataset version unknown** (uncached): cite disabled on that card with
  tooltip *"underlying dataset is uncached — reload once data warms."*
- **Citation string for an experimental insight:** auto-suffixed
  `(experimental detector — not yet validated on real data)` so a researcher
  cites honestly. Reuses the existing `experimental` flag.
- **Copy-to-clipboard unsupported** (older browser): falls back to a
  select-all text box. No silent failure.

---

## 3.3 P-102 — Signal Subscriptions (consumer-grade push)

**Goal:** author a signal in plain English, get it pushed to email/Telegram/
Slack/RSS — no webhook server, no ops.

### End-to-end UX flow
1. **Entry (two paths):**
   - **From the Ask box:** after any `/ask` answer, a button `🔔 Save as signal`
     appears under the answer. (1 click.)
   - **From the My Signals panel:** a new collapsed `▸ My Signals` panel (under
     All Insights) with a `+ New signal` composer.
2. **Compose (NL → scan-target, LLM frames):** user types a sentence —
   *"tell me when overnight HIBOR breaks 2.5%"*. The agent compiles it to a
   deterministic scan target and shows a **plain-English confirmation card**:
   *"I'll watch `hkma/daily-interbank-liquidity.hibor_overnight` and alert you
   when it exceeds 2.5. [detector: series_jump, threshold derived]. Sound right?"*
   `[Yes, create] [Adjust]`. **The detector stays deterministic** — the LLM only
   translates intent → config.
3. **Preview (deterministic):** before saving, a `👁 Preview matches` button runs
   the compiled detector against the last 90 days and shows *"this would have
   fired 3 times in the last 90 days"* with the dates — so the user calibrates
   sensitivity before subscribing. *(This is P-107's threshold-preview UX,
   folded in here per the Phase-2 recommendation.)*
4. **Channel selection:** `[✉ Email] [💬 Telegram] [Slack] [📡 RSS]` — user
   picks ≥1; for email/Telegram a one-time verify flow (click-confirm link).
5. **Save:** Toast → *"Signal saved — you'll hear from me when it fires."* The
   My Signals panel lists it with `[on/off] [✏] [▾ history]`.
6. **Fire:** when the detector fires on a fresh scan, the configured sinks
   receive a push carrying the insight + evidence + permalink. The My Signals
   panel's `▾ history` shows dispatch log (reuses `/v1/alerts`).

### Empty state
- **No signals yet:** My Signals panel → EmptyState →
  *"No signals yet. Tell me what to watch — e.g. 'buzz me when HIBOR breaks 2.5'
  — and I'll push it to your inbox or phone."* + a composer prefilled with an
  example.

### Edge cases & error recovery
- **NL → scan-target ambiguous** (LLM unsure): the confirmation card offers 2–3
  candidate interpretations as radio options ("did you mean overnight or
  3-month HIBOR?") rather than guessing. Heuristic mode: falls back to a
  **form-based builder** (pick dataset → field → detector → threshold) — fully
  usable with no LLM key (invariant 5).
- **Detector never fires in 90-day preview:** warning line → *"This hasn't fired
  in 90 days — you may want a lower threshold. [adjust]"*. Prevents dead signals.
- **Channel verify pending:** signal shows `⏳ pending verification` and won't
  fire until the user clicks the confirm link; a "resend" affordance after 24h.
- **Sink failure at fire-time:** reuses the existing F-062 guarantee (one sink
  failing doesn't abort others); the My Signals history row shows `⚠ email
  bounced, telegram ok` so the user can self-recover.
- **Rate-abuse guard:** a user can't create 500 signals; soft cap with an
  explanation, and a "merge similar signals?" suggestion when two overlap >90%.

---

## 3.4 P-103 — Unprecedentedness Score (historical context)

**Goal:** every numeric insight carries a percentile rank, a normal-range band,
and a "last time this happened" comparator.

### End-to-end UX flow
1. **Render (zero extra clicks):** on every numeric InsightCard, a new
   **UnprecedentednessBand** renders inline above the summary:
   - a horizontal range bar showing the normal envelope (rolling median ± MAD),
     the historical min/max, and a marker for the current value with its
     **percentile rank** chip (`top 2% · 1-in-8-yr`).
   - a one-line comparator: *"Last exceeded: 2018-Q2 (+112%) — [view that
     finding ⟩]"* → links the prior event (reuses P-104 persistence).
2. **Interaction:** clicking the comparator link opens that prior insight's card
   inline (threaded), so the user sees the parallel without losing place.
3. **Brief integration:** the brief ranker already discounts experimental
   findings; it now also weights `unprecedentedness` so truly rare events float
   up — surfaced transparently as a `rarity` factor in the (optional) "why am I
   seeing this" tooltip.

### Empty state
- **Insufficient history (<12 data points):** band hidden, replaced by a muted
  line *"Not enough history to rank yet (n=8; appears after 12 points)."* No
  misleading band.
- **Non-numeric / categorical finding:** band gracefully absent (e.g. a pure
  `cross_source_gap` on dates) — no broken UI.

### Edge cases & error recovery
- **History window configurable** (90d default); if a user wants longer, the
  tooltip exposes *"based on 90-day window [change]"* — links P-107's studio.
- **Regime change invalidates the envelope:** if the MAD envelope itself jumps
  (detected via the existing outlier math), the band shows a `⚠ regime shift —
  envelope recalculated as of {date}` note rather than silently widening.

---

## 3.5 P-104 — Insight Lifeline (persistent, delta-aware feed)

**Goal:** persist insights, add read/unread + "what's new since you left" +
evolution tracking. The retention engine.

### End-to-end UX flow
1. **Return visit banner:** on any return to the dashboard (last-visit > 1h
   ago), a dismissible banner tops the feed: *"4 new since you left (Fri 18:00)
   · 1 evolved · [show only new]"*. Clicking `show only new` toggles the feed
   to a `new` filter tab.
2. **Feed tabs:** the severity filter row gains tabs:
   `[all] [🆕 new] [🔔 watching]` alongside the existing severity pills. The
   `new` tab auto-selects when the banner is clicked.
3. **Read/unread:** new insights show a left-border accent + a dot; opening the
   EvidenceDrawer (or scrolling past with a 1s dwell) marks them read. Unread
   count surfaces in the document title (`(4) HK City Pulse`).
4. **Watch:** the `CardActionBar` gains `🔔 Watch` — a watched insight tracks
   evolution and appears in the `watching` tab. Watched insights that evolve get
   a `🔺 evolved` badge.
5. **Evolution detection:** when a scan re-fires the same finding id with
   *changed* evidence (e.g. the March HIBOR cluster extends another day), the
   existing insight is updated (not duplicated) and stamped `evolved {date}`,
   with a collapsible diff ("+1 day: 2026-03-24, 1.16").

### Empty state
- **First visit (no last-visited):** no banner; the brief hero is the welcome.
- **No new since last visit:** banner → *"Nothing new since Fri 18:00 — you're
  caught up. ✨"* (positive reinforcement, not emptiness).

### Edge cases & error recovery
- **Clock skew / cross-device:** last-visited is server-stamped (per-user, once
  persistence lands) so it's consistent across devices; client-only fallback if
  unauthenticated.
- **Evolution vs. duplicate:** dedup by finding id (already in F-059); an
  evolved insight never creates a second card.
- **Mass evolution event** (e.g. backfill changes many): collapse into a single
  "12 insights evolved" row to avoid feed flooding.

---

## 3.6 P-105 — Drill-In Investigation (case file from a card)

**Goal:** from any insight → a saved, resumable, shareable multi-step
investigation thread.

### End-to-end UX flow
1. **Entry:** `CardActionBar` → `🔍 Investigate`. Opens a case-file view
   (replaces the main column; chat rail stays). The insight is pre-seeded as
   **step 0**.
2. **Guided first steps:** the agent immediately offers 3 suggested next steps
   as one-click chips: `[show related series] [find historical parallels] [run
   cross-source check]` — each runs a deterministic tool (via the existing
   ToolBelt) and appends its result as the next numbered step with evidence.
3. **Free-form:** the user can also type (reuses the Ask rail semantics) — every
   Q&A becomes a numbered step in the case file.
4. **Notes:** each step has an `add note` affordance — Maya's byline draft, or
   Chen's citation stub.
5. **Save/resume/share:** the case file has a stable permalink
   (`/investigate/{case_id}`); `💾 Save` persists; the header gains a
   `📁 Cases` dropdown listing resumable investigations. `Share` → read-only
   link (reuses P-101 cite plumbing for the permalink + manifest).
6. **Branch:** from any step, `⑃ Branch` creates a new case file seeded from
   that step — lets Maya pursue two angles without clobbering.

### Empty state
- **No cases yet:** `📁 Cases` dropdown → *"No saved investigations yet. Hit
  🔍 Investigate on any insight to start a case file."*

### Edge cases & error recovery
- **Agent loop hits max_steps** (F-056): the case file shows a soft stop →
  *"Reached the step budget for this turn — [continue investigation] starts a
  fresh bounded turn."* No dead-end.
- **A step's dataset goes uncached:** that step shows `⚠ data unavailable —
  [retry]` inline; other steps unaffected.
- **Shared (read-only) link opened by a non-user:** renders as a static
  evidence-backed narrative (no edit affordances) — the case file *as a
  publishable artifact*.

---

## 3.7 P-106 — Bilingual Surface (zh-HK)

**Goal:** a zh-HK surface — localized dashboard, insight summaries, evidence,
exports — unlocking the HK public audience.

### End-to-end UX flow
1. **Toggle:** header gains a `[中 / EN]` toggle; preference persists
   (localStorage + server-side per-user once auth lands). Default = browser
   `Accept-Language`.
2. **Localized chrome:** all UI strings (brief, feed, evidence labels, empty
   states, toasts) localized via an i18n dictionary. The evidence values
   themselves stay as-is (numbers/dates are language-neutral).
3. **Localized insight summaries:** the LLM frames the summary in zh-HK;
   heuristic mode falls back to **bilingual templating** (deterministic
   templates with zh-HK strings, so summaries exist with no LLM key — invariant
   5). Every zh summary carries the ModeBadge so the reader knows the framing
   source.
4. **Bilingual exports:** P-101's cite artifacts (PDF card, social OG) render
   in the active language; the citation string includes a bilingual title field.
5. **Bilingual Silence Index:** the flagship (P-100) reads in zh-HK — critical
   for the civic-accountability audience.

### Empty state
- **Translation missing for a string:** falls back to English with a tiny `(en)`
  marker rather than a blank — never a broken string.

### Edge cases & error recovery
- **LLM framing fails / heuristic mode:** bilingual template renders — the user
  always gets *a* zh summary. ModeBadge shows which.
- **Mixed-language evidence** (a press release title that's itself bilingual):
  rendered verbatim with both scripts, not machine-translated (we don't
  translate source text — we surface it).
- **Toggle mid-session:** no page reload; React-style re-render of strings,
  evidence values untouched.

---

## 3.8 P-107 — Detector Studio (self-serve scan authoring)

**Goal:** author/tune/preview a scan target in-product; remove the
config.toml+restart barrier (G8). Per Phase-2 note, its preview UX is shared
with P-102; the standalone studio is the power-user surface.

### End-to-end UX flow
1. **Entry:** `▸ Detector Studio` collapsed panel (or from P-102's "Advanced"
   link). Opens a 3-pane composer: `[dataset/field picker] [detector + threshold
   controls] [live preview pane]`.
2. **Compose (form, fully heuristic — no LLM needed):** pick dataset → field →
   detector (dropdown of the 8) → drag the threshold slider. As the slider
   moves, the **preview pane** live-updates: *"this would fire N times in the
   last 90 days"* with the matched dates plotted on a mini time-series.
3. **Save as:** `[🔔 Save as my signal]` (→ P-102) or `[💾 Save as default scan]`
   (→ persists to the user's scan config; for multi-user, their profile).
4. **Share:** `[🔗 Share this detector]` → a permalink encoding the config
   (`/studio?cfg=…`) so Chen can hand Marcus a tuned detector.

### Empty state
- **Dataset not yet cached:** preview pane → *"Pick a warmed dataset to preview.
   [browse datasets]"* — links the existing Browse panel.

### Edge cases & error recovery
- **Threshold produces 0 matches:** preview shows 0 with a hint → *"try a lower
   threshold"*, never a silent empty chart.
- **Threshold produces >500 matches:** caps the preview plot + warns → *"matches
   capped at 500 for preview — refine your threshold."*
- **Invalid detector+field combo** (e.g. `correlation` needs 2 fields): the form
  disables Save and shows inline validation → *"correlation needs a second field
  (field_b)."*

---

## 3.9 Cross-feature interactions (the cohesive narrative)

These interactions are *designed*, not accidental — they are why the 8 features
form one product, not eight widgets:

| From | To | Interaction |
|:---|:---|:---|
| P-103 Unprecedentedness comparator | P-104 Lifeline | the "last exceeded" link opens a persisted prior insight |
| P-102 Signal preview | P-107 Studio | the signal composer's preview *is* the studio preview; "Advanced →" opens the full studio |
| P-100 Silence Index breakdown row | P-101 Cite / P-105 Investigate | each breakdown event is an insight → citable / investigable |
| P-105 Investigation step | P-103 Unprecedentedness | a parallel found in an investigation carries its own rarity band |
| P-101 Cite manifest | P-100 Silence Index report | the quarterly report is one big cite artifact (same manifest engine) |
| P-106 Bilingual | P-100 / P-101 | the flagship index + cite cards render zh-HK — the civic reach loop closes |

**Net cognitive-load effect:** a first-time Priya sees the *same* dashboard she
sees today (new actions are collapsed/secondary). Maya/Chen discover depth as
they need it (Cite → Investigate → Studio). Marcus lives in Signal Subscriptions.
No persona's first screen is heavier than today's.

---

## 3.10 Friction-elimination scorecard (vs Phase-1 friction catalog)

| Gap | Resolved by | How |
|:---|:---|:---|
| G1 cold-start opacity | (global) ModeBadge + warming states | every empty state names the mode + ETA |
| G2 ephemeral insights | P-104 Lifeline | persists; roadmap Postgres tier |
| G3/G4 no personalization / alert-needs-infra | P-102 Signal Subs | NL signals + consumer push |
| G5 no "how unprecedented" | P-103 Unprecedentedness | percentile + band + comparator |
| G6 no export/share/cite | P-101 Cite-It | permalink + BibTeX + manifest |
| G7 /ask ephemeral | P-105 Drill-In | saved case files |
| G8 detectors not self-serve | P-107 Studio + P-102 preview | in-product authoring |
| G9 English-only | P-106 Bilingual | zh-HK surface |
| G10 no "what's new" | P-104 Lifeline | new-since-last-visit banner |
| G11 no collaboration | P-105 shared case files | resumable + shareable |
| G12 coarse severity | P-103 rarity + P-100 score | richer ranking surfaced to user |

**All 12 baseline friction points are addressed** by the designed UX.

---

## 3.11 Exit-criteria check

- [x] All 8 approved features have a complete, step-by-step UX flow (§3.1–§3.8).
- [x] Empty states designed for every feature (each § has an "Empty state").
- [x] Error-recovery / edge-case patterns designed for every feature (each § has
      "Edge cases & error recovery").
- [x] Cross-feature interactions specified (§3.9) — the product is cohesive,
      not eight widgets.
- [x] Major UX friction resolved — all 12 baseline gaps mapped (§3.10).
- [x] Determinism guarantee preserved in every flow (invariant 1 + per-feature
      notes); heuristic-mode fallback specified for every LLM-dependent feature.
- [x] Canonical spreadsheet §D populated with step-by-step `UX Flow Summary`.

---

> Phase 3 is a design phase (no checkpoint mandated by the prompt between 3 and
> 4), so I'm proceeding directly into Phase 4: translating these flows into
> formal User Stories with Acceptance Criteria, RICE-scoring the full backlog,
> and finalizing KPIs. The Phase-4 checkpoint (the final PRD/spreadsheet) is the
> next approval gate.
