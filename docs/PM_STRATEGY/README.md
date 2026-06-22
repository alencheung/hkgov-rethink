# Product Strategy — hkgov-rethink

> The complete PM + UX strategy engagement for `hkgov-rethink`, the Rust
> platform that surfaces what the HKGOV press room leaves unsaid. Five phases,
> executed in order against the engagement prompt, with checkpoints at each gate.

## Read order

| # | Document | Phase | Purpose |
|:---:|:---|:---|:---|
| 1 | [`PRODUCT_STRATEGY_TRACKER.md`](./PRODUCT_STRATEGY_TRACKER.md) | **canonical** | The one source of truth — every feature (`P-###`), its user story, UX flow, RICE score, KPI, status, release. Read this first; the phase docs are the reasoning behind it. |
| 2 | [`PHASE_1_FOUNDATION.md`](./PHASE_1_FOUNDATION.md) | Foundation | Value proposition, 4 personas (Maya/Chen/Marcus/Priya), current-state journey, 12-point friction catalog (G1–G12), base-feature inventory. |
| 3 | [`PHASE_2_IDEATION.md`](./PHASE_2_IDEATION.md) | Ideation | 7 candidates + 1 bonus (P-100→P-107), each with an "Aha!" moment + blue-ocean rationale. The strategic narrative they form. |
| 4 | [`PHASE_3_UX_STORYBOARD.md`](./PHASE_3_UX_STORYBOARD.md) | UX | End-to-end flows for all 8 features, with empty states, edge-case & error-recovery patterns, cross-feature interactions, and a friction-resolution scorecard. |
| 5 | [`PHASE_4_PRD.md`](./PHASE_4_PRD.md) | Requirements | Formal user stories + acceptance criteria (US-100→US-107), full-backlog RICE, ranked 3-release plan (~27 PM), KPI tree, risk register. |
| 6 | [`PHASE_5_VALIDATION.md`](./PHASE_5_VALIDATION.md) | Validation | The recursive loop. Honestly audited Phase 2–4 → found 8 gaps (incl. a hidden identity prerequisite) → spawned P-108/P-109, re-scoped P-100, re-ordered the plan. Post-iteration health summary. |

## The headline

The engagement **repositions the product** from "a dashboard of findings" into
*"the system of record for government data-transparency,"* built on three
defensible properties no incumbent combines:

1. **Cross-source, not single-source** — the findings live in the gaps *between* sources.
2. **Deterministic-first AI** — detection is pure Rust; the LLM only frames. Same data in → same findings out, **no API key required**.
3. **Evidence, not assertions** — every finding links to verifiable source rows.

## The flagship

**The Silence Index (P-100)** — productizes the project's tagline into an ownable,
citable number: *"how much did HKGOV not explain this period?"* Built purely
from existing deterministic detectors (`cross_source_gap` + unattributed
`series_jump`), trended per quarter, drilled to the exact missing dates, and
exportable as a CI-reproducible transparency report.

## The release plan (post-validation, ~34 PM)

```
R1 — "Make it stick"   (11 PM)  identity + persistence + context
R2 — "Make it citable" ( 7 PM)  the flagship beat (Cite-It + HKMA Silence Index)
R3 — "Make it reach"   (16 PM)  growth + depth (Bilingual, Signals, Mobile, …)
Parallel: data.gov.hk expansion (flagship fuel)
```

## Engagement outcome

| Cohesion criterion | Result |
|:---|:---:|
| C1 Onboarding → action frictionless | ✅ |
| C2 Every feature → persona JTBD/"Aha!" | ✅ |
| C3 No use case blocked by missing UX | ✅ |
| C4 Narrative cohesive | ✅ |
| **Overall cohesion score** | **88 / 100** |

**Next action:** engineering execution against R1, starting with P-108 (identity)
and P-104 (persistence) in parallel.
