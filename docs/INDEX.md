# Documentation Index

> **Start here.** This is the navigation map for every document in the repo.
> Find your question on the left; read the file on the right. If you're new,
> read [`README.md`](../README.md) first for the project overview, then come
> back here to go deeper.

## Quick-reference: "Which file do I read for…?"

| If you want to understand… | Read this | Type |
|---|---|---|
| What the project is and how to run it | [`README.md`](../README.md) | Entry point |
| The end-to-end design + the "why" behind each crate | [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) | Living |
| The scaling path (single node → 100k design target) + backend wiring status | [`docs/CAPACITY.md`](CAPACITY.md) | Living |
| Which HKGOV endpoints we hit and why (all 7 connectors, verified live) | [`docs/DATA_SOURCES.md`](DATA_SOURCES.md) | Living |
| What's shipped vs. what's planned | [`docs/ROADMAP.md`](ROADMAP.md) | Living |
| Does a specific feature work? (test status, expected behaviour) | [`FEATURES_TRACKER.md`](../FEATURES_TRACKER.md) | Living |
| What defects exist / were found / were fixed? | [`DEFECTS.md`](../DEFECTS.md) | Living |
| Real captured insights (proof the detectors work on live data) | [`EXAMPLES.md`](../EXAMPLES.md) | Living |
| Iconography rules (Remix Icon, no emoji) + i18n rules | [`AGENT.md`](../AGENT.md) | Living |
| How to contribute + feature matrix + source-verification rules | [`CONTRIBUTING.md`](../CONTRIBUTING.md) | Living |
| What shipped in each milestone (v1–v9) | [`CHANGELOG.md`](../CHANGELOG.md) | Living |
| The product/UX strategy (personas, features P-001–P-109, release plan) | [`docs/PM_STRATEGY/README.md`](PM_STRATEGY/README.md) → [`PRODUCT_STRATEGY_TRACKER.md`](PM_STRATEGY/PRODUCT_STRATEGY_TRACKER.md) | Design rationale |
| Historical QA audit reports (frozen, not current) | [`docs/archive/README.md`](archive/README.md) | Archive |

## By role

**New contributor / first session:**
1. [`README.md`](../README.md) — project overview, quick start, API reference.
2. This file — pick where to go next.
3. [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) — the design and the determinism guarantee.
4. [`CONTRIBUTING.md`](../CONTRIBUTING.md) — how to add a connector / detector + the invariants.

**Adding a data source:**
1. [`docs/DATA_SOURCES.md`](DATA_SOURCES.md) — the verified endpoint table + the format every connector documents.
2. [`CONTRIBUTING.md`](../CONTRIBUTING.md) §"Adding a connector" — the 5-step checklist.
3. `crates/connectors/src/registry.rs` — where to register it (wrap with rate limiter + circuit breaker).

**Debugging a defect:**
1. [`DEFECTS.md`](../DEFECTS.md) — the canonical defect log (D-001–D-024, all resolved/waived/deferred).
2. [`docs/archive/`](archive/) — the per-test traces that found each defect (frozen but detailed).

**Understanding the AI agent layer:**
1. [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) §"AI-agent layer" + §"The determinism guarantee".
2. `crates/agent/src/analysis.rs` — the detectors (pure Rust).
3. [`docs/PM_STRATEGY/`](PM_STRATEGY/) — the product rationale for the v7/v8 features built on the agent.

**Working on the dashboard:**
1. [`AGENT.md`](../AGENT.md) — iconography (Remix Icon) + i18n rules (mandatory before touching UI).
2. `dashboard/llms.txt` — the agent-facing API surface.

**Scaling / operations:**
1. [`docs/CAPACITY.md`](CAPACITY.md) — the honest scaling path (what's wired, what isn't).
2. [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) §"Configuration & operations".
3. `crates/common/src/config.rs` — every runtime knob.

## Document inventory

### Root-level living docs

| File | Purpose | Grep keywords |
|---|---|---|
| `README.md` | Project overview, quick start, full API reference, roadmap status | `API reference`, `quick start`, `docker`, `python` |
| `AGENT.md` | Agent working agreement: iconography (Remix Icon, no emoji) + i18n rules | `emoji`, `Remix Icon`, `i18n`, `data-i18n`, `zh-HK` |
| `CHANGELOG.md` | What shipped in each milestone (v1–v9 + unreleased) | `v6`, `v7`, `v8`, `v9`, `Silence Index`, `Cite-It` |
| `CONTRIBUTING.md` | How to contribute: data-source verification, feature flags, architecture invariants, adding a connector/detector | `feature flags`, `invariants`, `Connector trait` |
| `DEFECTS.md` | Canonical defect log (D-001–D-024), all resolved/waived/deferred | `D-006`, `D-012`, `D-018`, `D-024`, `fixed`, `waived`, `deferred` |
| `EXAMPLES.md` | Real captured insights (HIBOR +99.3%, March outlier cluster, cross-source gaps) | `series_jump`, `outlier`, `cross_source_gap`, `HIBOR` |
| `FEATURES_TRACKER.md` | Canonical feature/user-story status (F-001–F-107, ~280 tests, all passing) | `F-089`, `F-100`, `Phase 9`, `D-012` |
| `CODE_OF_CONDUCT.md` | Contributor Covenant | — |

### `docs/` — living architecture & ops docs

| File | Purpose | Grep keywords |
|---|---|---|
| `docs/INDEX.md` | **This file** — navigation hub | — |
| `docs/ARCHITECTURE.md` | Crate graph, data flow, the determinism guarantee, the four layers of v6 intelligence, proactive alerting | `crate graph`, `determinism`, `RecordStore`, `RecordStore trait` |
| `docs/CAPACITY.md` | Scaling path: single-node ceiling, the 5-stage table, what "100k" means | `100k`, `moka`, `Redis`, `Postgres`, `wired`, `k6` |
| `docs/DATA_SOURCES.md` | All 7 connectors' verified endpoints, envelopes, quirks, rate limits | `HKMA`, `data.gov.hk`, `Immigration`, `RVD`, `Land Registry`, `circuit breaker` |
| `docs/ROADMAP.md` | Milestone status (v1–v9 shipped) + the "Remaining" future-work list | `Remaining`, `OAuth`, `k8s`, `LB tier` |
| `docs/AGENTIC_GATE_FINDINGS.md` | Agentic-gate quality-gate run record + false-positive suppression rationale (why blocking findings were not applied) | `agentic-gate`, `false positive`, `SKILL_DEAD_BRANCH` |

### `docs/PM_STRATEGY/` — product design rationale

| File | Purpose |
|---|---|
| `docs/PM_STRATEGY/README.md` | Read-order index for the 5-phase PM/UX engagement + the headline |
| `docs/PM_STRATEGY/PRODUCT_STRATEGY_TRACKER.md` | **Canonical** — every feature (P-001–P-109), its user story, RICE score, KPI, status |
| `docs/PM_STRATEGY/PHASE_1_FOUNDATION.md` | Value proposition, 4 personas, 12-point friction catalog (G1–G12) |
| `docs/PM_STRATEGY/PHASE_2_IDEATION.md` | 8 feature candidates (P-100–P-107) + "Aha!" moments + blue-ocean rationale |
| `docs/PM_STRATEGY/PHASE_3_UX_STORYBOARD.md` | End-to-end UX flows for all 8 features |
| `docs/PM_STRATEGY/PHASE_4_PRD.md` | Formal user stories + acceptance criteria + RICE + ranked release plan |
| `docs/PM_STRATEGY/PHASE_5_VALIDATION.md` | Recursive validation loop; found 8 gaps, spawned P-108/P-109, re-scoped P-100 |

### `docs/archive/` — frozen historical artifacts

| File | What it was | Frozen at |
|---|---|---|
| `docs/archive/QA_PHASE1_FEATURES.md` | 56-feature spreadsheet + role matrix | 56 features, 168 scenarios |
| `docs/archive/QA_PHASE2_3_TESTS_DEFECTS.md` | 168 per-test traces + D-006..D-011 | 189 tests |
| `docs/archive/QA_PHASE5_REGRESSION.md` | Post-fix regression report | 189 Rust + 14 Python |
| `docs/archive/QA_PHASE6_FINAL_SUMMARY.md` | Closing QA report (confidence 92/100) | 189 Rust + 14 Python |
| `docs/archive/NEXT_FEATURE_INTEGRATION_MAPS.md` | Pre-implementation integration research | All shipped (v7/v8) |

See [`docs/archive/README.md`](archive/README.md) for why these are archived.

### Other doc surfaces

| File | Purpose |
|---|---|
| `dashboard/llms.txt` | Agent-facing API surface (served at `/llms.txt`) |
| `python/README.md` | `hkgov-py` Python client docs |
| `.github/PULL_REQUEST_TEMPLATE.md` | PR checklist |
| `.github/ISSUE_TEMPLATE/bug_report.md` | Bug report template |
| `.github/ISSUE_TEMPLATE/feature_request.md` | Feature request template |

## Key source files referenced by the docs

| If you want to understand… | Read this source |
|---|---|
| The normalized data model | `crates/common/src/model.rs` |
| Every runtime knob | `crates/common/src/config.rs` |
| The scaling contract (how to add a store backend) | `crates/store/src/lib.rs` (`RecordStore` trait) |
| How to add a data source | `crates/connectors/src/lib.rs` (`Connector` trait) + `crates/connectors/src/registry.rs` |
| The resilience layer | `crates/connectors/src/resilience.rs` |
| How insights are detected | `crates/agent/src/analysis.rs` (detectors) |
| How insights are framed | `crates/agent/src/llm.rs` |
| The agent loop | `crates/agent/src/loop_mod.rs` |
| The Silence Index methodology | `crates/agent/src/silence.rs` |
| The Cite-It reproducibility manifest | `crates/agent/src/cite.rs` |
| Signal subscriptions | `crates/agent/src/signal.rs` |
| The identity tier | `crates/agent/src/identity.rs` |
| File-based snapshot persistence | `crates/agent/src/persist.rs` |
| Per-IP rate limiting | `crates/api/src/ratelimit.rs` |
| Constant-time secret comparison | `crates/api/src/secrets.rs` |
| The route table | `crates/api/src/routes/mod.rs` |
