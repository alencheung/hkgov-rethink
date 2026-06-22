# Phase 1 — Foundation Analysis & Persona Modeling

> **Status:** Phase 1 complete — **🛑 CHECKPOINT: awaiting approval before Phase 2.**
> Canonical spreadsheet: [`PRODUCT_STRATEGY_TRACKER.md`](./PRODUCT_STRATEGY_TRACKER.md)
> Grounded in: `README.md`, `EXAMPLES.md`, `FEATURES_TRACKER.md`, `docs/ROADMAP.md`,
> `CHANGELOG.md`, `config.toml`, `dashboard/index.html` (read in full before writing this).

---

## 1.1 Core value proposition

> **hkgov-rethink surfaces what the HKGOV press room leaves unsaid.**

It cross-references Hong Kong Government open-data sources — HKMA monetary
statistics, data.gov.hk, press releases, LandsD/CSDI geospatial — and flags the
moments where **the official narrative and the published data disagree**, then
hands back every finding with **evidence pointers a human can verify against the
source rows**.

The differentiator is not "AI over data." It is three properties that no
single-source dashboard or black-box LLM tool delivers together:

1. **Cross-source, not single-source.** The high-value findings (HIBOR doubled
   in one settlement window with no attributing press release; press dates with
   no matching data row) come from *comparing* sources. A per-source dashboard
   structurally cannot see them.
2. **Deterministic-first AI.** Detection is pure Rust; the LLM only *frames*.
   Same data in → same findings out, **no API key required**. This is the
   credibility foundation — findings are reproducible in CI and citable.
3. **Evidence, not assertions.** Every `Insight` carries `EvidenceRef`s back
   into the store, so a journalist or researcher can verify the claim against
   the source data before publishing.

**Atomic capabilities** (the building blocks every feature composes from):

| # | Capability | Source of truth |
|---|---|---|
| A1 | Multi-source ingestion w/ resilience (token-bucket rate limit, 3-state circuit breaker, retry+backoff) | `crates/connectors/src/resilience.rs`, `registry.rs` |
| A2 | One normalized data dialect across all sources (`NormalizedRecord`) | `crates/common/src/model.rs` |
| A3 | Cache-first serving (in-process moka → Redis → Postgres; hot reads never touch network) | `crates/store/src/lib.rs` (`RecordStore` trait) |
| A4 | Deterministic anomaly detection — 8 detectors incl. cross-source | `crates/agent/src/analysis.rs` |
| A5 | Evidence-pointer provenance (every finding links to source rows) | `crates/agent/src/insight.rs` |
| A6 | Agentic investigation loop (LLM chooses which deterministic tool to call, bounded) | `crates/agent/src/loop_mod.rs`, `tools.rs` |
| A7 | Natural-language Q&A over data (`POST /v1/ask`) | `crates/agent/src/qa.rs` |
| A8 | Proactive push alerting (webhook + email, severity-filtered, deduped) | `crates/agent/src/alerts.rs` |
| A9 | Ranked daily brief (scored, experimental-discounted) | `crates/api/src/brief.rs` |
| A10 | Human-in-the-loop feedback on insight quality | `POST /v1/insights/{id}/feedback` |

---

## 1.2 User archetypes (4 personas)

### Persona 1 — "Maya," the Investigative Journalist *(The Scoop-Hunter)*
- **Role:** Data-desk reporter at a HK business/science publication.
- **Job to be done:** *"Find the story where the official line and the numbers
  diverge — before a competitor files it."*
- **Goals:** Break market-integrity / fiscal-transparency stories; publish fast;
  survive legal review (so every claim must be checkable to a source row).
- **Pain points:** Manually diffing press releases against HKMA datasets is
  slow; she misses *regime* shifts (sustained clusters) because she scans charts
  by eye; her editor kills anything that isn't sourced.
- **Tech proficiency:** Medium. Uses Google Sheets, basic Python via the data
  desk, lives in a browser. Will not write Rust.
- **Loves:** `cross_source_gap` ("press release with no data row" = a literal
  story lead); evidence pointers = instant sourcing.
- **Quote:** *"If you can show me the date HKMA went silent on a 99% HIBOR move,
  that's my front page."*

### Persona 2 — "Dr. Chen," the Economic Researcher / Academic *(The Rigorist)*
- **Role:** University economist / think-tank analyst writing papers & policy
  submissions.
- **Job to be done:** *"Produce reproducible, citable analysis of HK monetary /
  fiscal anomalies — and be able to defend every number."*
- **Goals:** Publish credible research; cite a deterministic, versioned finding;
  hate black-box AI with a passion.
- **Pain points:** LLM tools hallucinate and can't be cited; reproducing a
  finding across runs is impossible with stochastic models; no baseline/normal
  range to contextualize "is this actually unusual?"
- **Tech proficiency:** High. Python notebooks, R, version control. The Python
  client (`pip install hkgov-py`) is their front door.
- **Loves:** The determinism guarantee ("same data in, same findings out, no key
  required"); the `experimental` badge honesty; would pay for a citation/export
  artifact.
- **Quote:** *"If I can't reproduce it in CI, it doesn't go in my paper."*

### Persona 3 — "Marcus," the Treasury / Market-Risk Analyst *(The Desk Operator)*
- **Role:** Buy-side or corporate-treasury desk, monitoring HIBOR/liquidity for
  funding & hedging decisions.
- **Job to be done:** *"Get a verified, evidence-backed liquidity-event signal
  pushed to me the moment it happens — so I can act with a defensible audit
  trail."*
- **Goals:** Act first; justify the trade to risk/compliance; never miss a
  regime shift.
- **Pain points:** Bloomberg/Reuters tells him the move but not *whether it's
  unprecedented* or *whether HKMA explained it*; alerts arrive after the window;
  no provenance to satisfy compliance.
- **Tech proficiency:** High. Webhooks, Slack integrations, maybe Python. Time-
  poor; wants push, not pull.
- **Loves:** Severity-filtered webhook/email alerting; the `series_jump` +
  `outlier` combo (boundary + regime).
- **Quote:** *"Push it to my phone the second overnight HIBOR breaks 2σ, with
  the source rows — or I've already missed it."*

### Persona 4 — "Priya," the Civic-Tech Watchdog *(The Accountability-Seeker)*
- **Role:** NGO / civic-tech volunteer monitoring government fiscal
  transparency and livability data.
- **Job to be done:** *"Show the public, in plain language, where the government
  is opaque — and back it up."*
- **Goals:** Translate cross-source gaps into citizen-readable accountability
  stories; advocate for transparency; lower the technical barrier for the
  public.
- **Pain points:** Can't ingest 4 data sources herself; needs a readable
  dashboard, not a curl command; needs Chinese-language reach for the HK public;
  needs shareable artifacts (a link, a card, a tweet) not a JSON payload.
- **Tech proficiency:** Low–medium. Browser user; comfortable sharing links.
- **Loves:** The dashboard; the brief hero; would love a "share this finding"
  link and a Chinese surface.
- **Quote:** *"I don't need the JSON — I need a link I can put in a press
  release that anyone can click and see the evidence."*

> **Secondary persona (deployer):** the *Platform/Ops Engineer* who self-hosts
> (Docker/k8s, config tuning). They are served by the existing config + feature
> flags + capacity docs and are not the focus of the product/UX strategy, which
> targets the four *consumers* of insights above.

---

## 1.3 Current-state map & baseline friction

**Current user journey (dashboard consumer, today):**
1. Boots the API / opens `/dashboard` → configures base URL + optional API key
   (auto-filled when served by the API).
2. **Waits ~30s** for cache warm + the agent's first pass (20s delay, then scan).
3. Reads **Today's brief** (top-5 ranked insights, experimental-discounted).
4. Browses the **all-insights feed**, filters by severity (critical/warning/info).
5. Expands the **evidence** drawer on a card to verify a claim.
6. **Asks the agent** a question in the right-hand chat rail (multi-turn).
7. Optionally expands **Browse datasets** / **System health** (collapsed).
8. **Votes 👍/👎** on an insight (feeds the feedback counter).
9. Page auto-polls brief + insights every 30s.

**Baseline friction points & missing capabilities** (the gap list Phase 2 will
attack — each tagged to the persona it most hurts):

| # | Friction / gap | Persona | Severity |
|---|---|---|---|
| G1 | **Cold-start opacity.** ~30s warm + 20s agent delay; only skeleton loaders, no "warming / scanning / ready" state machine, no ETA. | All | Med |
| G2 | **Insights are ephemeral.** In-process only (roadmap admits this); restart loses history → no citation trail, no "what changed since last visit," no read/unread. | Researcher, Journalist | High |
| G3 | **No personalization / subscriptions.** Every user sees the same brief; no "watch HIBOR," no saved queries, no per-topic digest. | Analyst, Journalist | High |
| G4 | **Alerting needs infra.** Webhook sink needs a receiver; email needs an email-API config. No "push to my inbox/Telegram/Slack without standing up infrastructure" path for an individual. | Analyst, Watchdog | High |
| G5 | **No "how unprecedented" context.** A finding says "moved 99%" but not its percentile rank vs. history, nor a normal-range band. Is this a 1-in-100 event? | Analyst, Researcher | High |
| G6 | **No export / share / cite.** Can't export an insight as a citable artifact (link, PDF, citation string, OG image). A JSON payload is not a shareable. | Journalist, Watchdog, Researcher | High |
| G7 | **/ask is single-user & ephemeral.** No saved investigations; no "drill into this insight" chaining from a card. | Journalist, Researcher | Med |
| G8 | **Detectors aren't self-serve.** Tuning a threshold = edit `config.toml` + restart. No in-product scan-target authoring, no "make this sensitivity the default for me." | Researcher, Analyst | Med |
| G9 | **English-only.** Press releases are bilingual zh/en; HK public audience is bilingual. No Chinese surface. | Watchdog | High (for reach) |
| G10 | **No temporal "what's new" signal.** A card shows relative time but not "new since your last visit." Returning users can't triage deltas. | All | Med |
| G11 | **No collaborative layer.** Teams can't share/comment/assign/"snooze" insights. A newsroom or research team works alone. | Journalist, Researcher | Med |
| G12 | **Severity is coarse.** Only critical/warning/info; no composite "newsworthiness" or "unprecedentedness" rank surfaced to the user (it exists internally as the brief score but not on cards). | Journalist, Analyst | Low |

**What already works well (preserve, don't break):** the determinism guarantee,
evidence pointers, cache-first latency, the brief→feed→evidence reading rhythm,
the collapsed secondary panels keeping the insight-first hierarchy, the
experimental honesty badge, and the heuristic-mode zero-config path. These are
the product's spine.

---

## 1.4 Exit-criteria check

- [x] Core value proposition explicitly stated (§1.1).
- [x] Atomic capabilities enumerated and sourced to code (§1.1 table).
- [x] 4 detailed personas with goals, pains, tech proficiency, JTBD, quote (§1.2).
- [x] Current-state journey mapped + baseline friction catalogued (§1.3).
- [x] All base features documented in the canonical spreadsheet
      ([`PRODUCT_STRATEGY_TRACKER.md`](./PRODUCT_STRATEGY_TRACKER.md), rows
      `P-001`–`P-018`, `Status = shipped`).

---

> **🛑 CHECKPOINT — Phase 1.** Pausing here for approval before generating the
> Phase 2 ideation pipeline. Specifically seeking sign-off on:
> 1. The four personas (do these match the audiences you want to serve?).
> 2. The friction catalog (G1–G12) — are these the right problems to attack?
> 3. Confirmation that the base-feature inventory (P-001–P-018) is complete.
>
> On approval, Phase 2 will generate 5–7 non-obvious, blue-ocean use cases
> mapped to concrete features, each with a defined "Aha!" moment, and add them
> to the spreadsheet with preliminary Innovation/Effort/Impact scores.
