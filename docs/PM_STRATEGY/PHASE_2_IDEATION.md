# Phase 2 — Innovative Ideation & Use-Case Discovery

> **Status:** Phase 2 complete — **🛑 CHECKPOINT: awaiting approval before Phase 3.**
> Candidate rows added to the canonical spreadsheet:
> [`PRODUCT_STRATEGY_TRACKER.md`](./PRODUCT_STRATEGY_TRACKER.md) §B, `P-100`–`P-108`.
> On approval, survivors are promoted to §C (`Status = approved`) and advanced
> to Phase 3 UX storyboarding.

---

## 2.0 Ideation lens

The product's superpower is narrow and specific: **it deterministically finds
where the official narrative and the published data diverge — with evidence a
human can verify.** That is a *credibility engine*, not a dashboard.

So the blue-ocean question is not *"what else can we visualize?"* — that is the
contested, crowded space (Trading Economics, CEIC, Bloomberg, HKMA's own
dashboards). The uncontested moves all exploit one of three properties no
incumbent combines:

1. **Turn verified findings into shareable / citable / actionable artifacts**
   (bridge "the product found it" → "I published / acted on it").
2. **Productize the thesis itself** — make "the press room left this untold" a
   *measurable, comparable, public* thing rather than a tagline.
3. **Extend the divergence lens beyond monetary data** to fiscal, property,
   livability, and population — the cross-source gap detector is source-
   agnostic by construction.

Every candidate below is scored against three risks before it earns an "Aha!":
(a) does it preserve the **determinism guarantee** (no black-box claims)?,
(b) does it serve a *core* persona's JTBD (not a nice-to-have)?, and
(c) is it **defensible** (an incumbent can't copy it by repainting their dashboard)?

---

## 2.1 The ideation pipeline (7 candidates + 1 bonus)

### 🥇 Candidate 1 — **"The Silence Index"** *(P-100)* — government opacity, quantified
- **Innovative use case:** Productize the project's thesis. Every
  `cross_source_gap` (press release w/ no data row, or data w/ no press), every
  unattributed `series_jump` (big move, no same-day attributing release), and
  every missing-data day becomes a structured data point feeding a
  **per-source / per-quarter "Silence Index"** — a single, public, citable
  number for "how much did HKGOV *not* explain this period." Trend it over
  time; rank sources against each other.
- **Features required:** (a) a deterministic `silence_score` rollup over
  existing findings (pure Rust — no new detection, just aggregation over
  `cross_source_gap` + unattributed `series_jump`); (b) `GET /v1/silence-index`
  + a public dashboard surface; (c) a quarterly "transparency report" export.
- **The "Aha!" moment:** *"Wait — I can see a NUMBER for how transparent HKMA
  was this quarter, and it dropped 40%, with the exact missing dates as
  evidence. That's not an opinion, it's reproducible."*
- **Blue-ocean rationale:** Nobody quantifies government data-opaqueness this
  way. It converts the project's tagline into a defensible, ownable metric —
  the metric *becomes* the brand. Incumbents have no cross-source gap detector
  to build it from.
- **Personas:** Priya (primary — her whole JTBD), Maya (story factory),
  Chen (citable). **Attacks:** G2, G6.
- **Innovation 5 · Effort M · Impact H**

### 🥈 Candidate 2 — **"Cite-It"** *(P-101)* — one-click citation-grade artifact
- **Innovative use case:** From any insight → generate a **citation-grade
  artifact bundle**: a stable permalink, a BibTeX/RIS citation string, a
  shareable card (PDF + OG image), and a **reproducibility manifest** (dataset
  version + detector + threshold + data hash → "reproduce in CI"). The bridge
  between "found" and "published."
- **Features required:** (a) persisted, permalinked insights (needs the roadmap
  Postgres-tier persistence); (b) `/v1/insights/{id}/cite` returning the bundle;
  (c) a deterministic manifest hash so any artifact is independently verifiable.
- **The "Aha!" moment:** *"I clicked 'Cite' on the HIBOR finding and got a
  permalink + BibTeX + a manifest that reproduces in CI. My editor and legal
  verified it in one click — I filed in 20 minutes."*
- **Blue-ocean rationale:** No data/analytics tool ships *citation-grade
  reproducibility* from a finding. LLM tools can't (non-deterministic);
  dashboards don't (no provenance). This makes the product the citation source
  of record — a moat with researchers and newsrooms.
- **Personas:** Chen (citable papers), Maya (fast + legally safe filing),
  Priya (shareable link). **Attacks:** G6 (the headliner).
- **Innovation 5 · Effort M · Impact H**

### 🥉 Candidate 3 — **"Signal Subscriptions"** *(P-102)* — push alerts without infra
- **Innovative use case:** Let any individual user (no webhook server, no ops)
  create a custom signal in plain English — *"tell me when overnight HIBOR
  breaks 2.5%"*, *"when a monetary press release has no same-day data row"* —
  and receive it via **email / Telegram / Slack / RSS**, managed entirely by
  the platform. Turns alerting from an ops feature into a consumer feature.
- **Features required:** (a) NL → scan-target compiler (LLM frames; the
  *detector stays deterministic*); (b) per-user subscription store; (c) extra
  push sinks (Telegram/Slack/RSS) beyond webhook+email; (d) a self-service
  "manage my signals" surface.
- **The "Aha!" moment:** *"I typed 'buzz me when HIBOR breaks 2.5' and three
  days later my phone lit up with the evidence rows — I never wrote a line of
  code or stood up a server."*
- **Blue-ocean rationale:** Most "alerting" tools make the user build the
  receiver. Democratizing *signal authoring* in plain language, with
  deterministic execution underneath, is uncontested — and it converts passive
  readers (Priya, even Maya) into recurring, retained users.
- **Personas:** Marcus (primary — his exact JTBD), Priya, Maya. **Attacks:**
  G3, G4.
- **Innovation 4 · Effort L · Impact H**

### Candidate 4 — **"Unprecedentedness Score"** *(P-103)* — how rare is this, really?
- **Innovative use case:** Give every insight a **historical-context layer**:
  a percentile rank vs. its own history, a normal-range band (rolling median +
  MAD envelope), and a *"last time this happened was ___"* comparator with a
  link to that prior event. Turns a flat "moved 99%" into *"a 1-in-8-year
  event; last exceeded 2018-Q2 (link)."*
- **Features required:** (a) a deterministic `unprecedentedness` derivation
  reusing the existing MAD math (no new detector, a scoring layer over stored
  history); (b) surface the score + band + comparator on the insight card.
- **The "Aha!" moment:** *"'99% move' sounds scary — but the score says it's
  only the 3rd-biggest in a decade, with the two bigger ones linked. Now I know
  whether to act."*
- **Blue-ocean rationale:** Contextual *rarity* delivered deterministically &
  evidence-backed — not a "vibe" ranking. De-risks Marcus's trades and Chen's
  claims; gives Maya a "is this actually unprecedented?" gut-check for
  headlines.
- **Personas:** Marcus, Chen, Maya. **Attacks:** G5.
- **Innovation 4 · Effort M · Impact M**

### Candidate 5 — **"Insight Lifeline"** *(P-104)* — persistent, watchable, delta-aware feed
- **Innovative use case:** Persist insights (roadmap already wants this) and
  add a per-user **"what's new since you left"** layer: read/unread state,
  new-since-last-visit badges, delta highlighting when an insight *evolves*
  (e.g., a regime extends another day), and per-insight "watch" to track its
  evolution. Turns the feed from a snapshot into a living thread.
- **Features required:** (a) Postgres-tier insight persistence; (b) per-user
  read-state + "last visited" store; (c) insight-evolution detection (same
  finding id re-fires with changed evidence → "updated"); (d) the dashboard
  "what's new" banner.
- **The "Aha!" moment:** *"I came back Monday and the dashboard said '4 new
  insights since Friday, 1 evolved' — I triaged in 30 seconds instead of
  re-scanning everything."*
- **Blue-ocean rationale:** Persistence is table-stakes (on roadmap); the
  **delta-aware, evolution-tracking** UX is the novel, sticky layer that turns
  a tool into a daily habit. Retention engine.
- **Personas:** All (retention). **Attacks:** G2, G10.
- **Innovation 3 · Effort M · Impact H**

### Candidate 6 — **"Drill-In Investigation"** *(P-105)* — from a card to a saved case file
- **Innovative use case:** From any insight card → **"Investigate"** launches a
  guided, persistent, multi-step probe: the agent pre-seeds the insight's
  context as the starting point, then walks the user through related series,
  historical parallels (links to the Unprecedentedness comparator), and
  cross-source checks — **each step saved as a resumable, shareable case file**.
  Turns the ephemeral `/ask` into a saved investigation thread.
- **Features required:** (a) a persisted "investigation" object (insight +
  ordered agent steps + user notes); (b) a one-click "Investigate" action on
  cards; (c) a case-file view (resume / share / branch).
- **The "Aha!" moment:** *"I hit Investigate on the HIBOR jump and the agent
  handed me a case file: the move, the silence, the last 3 parallels, the
  related liquidity series — one thread I can resume tomorrow and hand to my
  editor."*
- **Blue-ocean rationale:** Saved, resumable, evidence-grounded case files from
  a *finding* (not from a blank chat) is uncontested. It makes the product the
  journalist's/researcher's workbench, not just a feed.
- **Personas:** Maya, Chen. **Attacks:** G7, G11.
- **Innovation 4 · Effort L · Impact M**

### Candidate 7 — **"Bilingual Surface"** *(P-106)* — the HK public, in their language
- **Innovative use case:** The press releases are already bilingual zh/en. Add a
  **zh-HK surface**: a localized dashboard, localized insight summaries
  (LLM-framed, so trivially localizable while detection stays deterministic),
  bilingual evidence rendering, and bilingual export artifacts. Unlocks the
  *actual* HK public audience — not just expat analysts.
- **Features required:** (a) i18n layer in the dashboard; (b) zh-HK framing
  via the LLM (heuristic mode falls back to bilingual templating); (c) a
  language toggle + persisted preference.
- **The "Aha!" moment:** *"My aunt in HK opened the same finding in Chinese and
  clicked straight through to the evidence — this isn't a tool just for
  English-speaking analysts anymore."*
- **Blue-ocean rationale:** Localization alone isn't blue-ocean, but **unlocking
  the civic-accountability audience in their own language** is what turns the
  Silence Index (Candidate 1) into a public-force multiplier. Strategic pair
  with #1.
- **Personas:** Priya (primary — reach), and the latent HK-public audience.
  **Attacks:** G9.
- **Innovation 3 · Effort M · Impact M**

### 🅱️ Bonus Candidate 8 — **"Detector Studio"** *(P-107)* — self-serve scan tuning *(recommend deprioritize)*
- **Innovative use case:** Author/tune a scan target in-product (drag the
  threshold, pick a field/detector), preview it against live data, save it as
  your own watch, and share it as a link. Removes the `config.toml` + restart
  barrier (G8).
- **Why bonus, not core:** High value to Chen/Marcus but the *lowest* blue-ocean
  score of the set — self-serve config is an incremental UX win, not a new
  market. **Recommend:** fold its threshold-preview UX into Candidate 3's
  signal-authoring flow rather than ship it standalone, at least until the
  core narrative (Silence Index + Cite-It + Subscriptions) is established.
- **Personas:** Chen, Marcus. **Attacks:** G8.
- **Innovation 2 · Effort M · Impact M**

---

## 2.2 The strategic narrative these candidates form

Individually each is a feature; **together they reposition the product** from
"a dashboard of findings" into *"the system of record for government
data-transparency"*:

```
   Deterministic detectors (the spine — P-004, unchanged)
            │
            ▼
   ┌─────────────────────────────────────────────────────────┐
   │  The Silence Index (P-100)  ← productizes the THESIS     │
   │  Unprecedentedness (P-103) ← adds "how rare" context     │
   │  Insight Lifeline  (P-104) ← makes it a daily habit     │
   └─────────────────────────────────────────────────────────┘
            │                                          │
            ▼                                          ▼
   ┌─────────────────────────┐         ┌──────────────────────────┐
   │  CONSUME  (retention)   │         │  ACT  (monetizable moat) │
   │  Bilingual (P-106)      │         │  Signal Subs (P-102)     │
   │  Drill-In   (P-105)     │         │  Cite-It     (P-101)     │
   └─────────────────────────┘         └──────────────────────────┘
```

- **P-100 (Silence Index)** is the brand-defining move; it makes the thesis
  ownable and citable, and it *needs* P-103 + P-104 to feel alive.
- **P-101 (Cite-It)** is the moat for the professional personas (newsroom +
  academe); it makes findings the *citation source of record*.
- **P-102 (Signal Subscriptions)** is the retention + future-monetization
  engine (consumer-grade push, no infra).
- **P-106 (Bilingual)** is the reach multiplier that turns P-100 into a civic
  force.
- P-105, P-107 deepen the workbench for power users but are secondary to the
  narrative.

---

## 2.3 "Aha!"-moment summary table

| ID | Candidate | The "Aha!" moment (the user's exact realization) |
|:---|:---|:---|
| P-100 | Silence Index | "There's a *number* for how opaque HKMA was this quarter — and it's reproducible." |
| P-101 | Cite-It | "One click → permalink + BibTeX + a CI-reproducible manifest. Legal signed off in minutes." |
| P-102 | Signal Subs | "I typed a sentence; my phone buzzed three days later with the evidence. Zero code." |
| P-103 | Unprecedentedness | "It's only the 3rd-biggest move in a decade — the two bigger ones are linked. Now I know." |
| P-104 | Insight Lifeline | "'4 new since Friday, 1 evolved' — I triaged in 30 seconds, not 30 minutes." |
| P-105 | Drill-In | "One click built me a case file I can resume tomorrow and hand to my editor." |
| P-106 | Bilingual | "My aunt in HK read the same finding, in Chinese, with the evidence. Not just for analysts." |
| P-107 | Detector Studio | "I dragged the threshold and instantly saw 3 more regime shifts." |

---

## 2.4 Exit-criteria check

- [x] ≥ 5 innovative, non-obvious use cases mapped to concrete features
      (7 candidates + 1 bonus; each names its required features + which atomic
      capability / existing detector it composes from).
- [x] Each candidate has a defined "Aha!" moment (§2.3).
- [x] Blue-ocean framing explicit for each (every candidate states why an
      incumbent can't copy it by repainting a dashboard).
- [x] Determinism-guarantee preserved on every candidate (each notes detection
      stays pure-Rust; the LLM only frames/compiles/translates).
- [x] Spreadsheet updated: §B populated with `P-100`–`P-108`, `Status = ideated`,
      preliminary Innovation/Effort/Impact, gap tags.

---

> **🛑 CHECKPOINT — Phase 2.** Pausing for approval. Specifically seeking:
> 1. **Which candidates advance to Phase 3 (UX storyboarding)?** My
>    recommendation: approve **P-100, P-101, P-102, P-103, P-104, P-106** as
>    the core narrative, **defer P-105 and P-107** to a later cycle (high
>    effort, lower narrative centrality). I'll storyboard whatever set you
>    choose.
> 2. Do you want the **Silence Index (P-100)** positioned as the flagship
>    (brand-defining) move, or keep the product positioned around the existing
>    "insights feed" with these as enhancements?
> 3. Any candidate you'd like **dropped**, **merged**, or **re-scoped** before
>    I design the UX?
>
> On approval, Phase 3 designs end-to-end UX flows (incl. empty states, edge
> cases, error recovery) for each approved candidate.
