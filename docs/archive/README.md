# Archive — historical point-in-time QA & research artifacts

> **These files are frozen snapshots, not current state.** They are kept for
> the audit trail and the per-test trace detail that the living trackers
> summarize. Do not update them; do not cite them as the current status.

## Why this archive exists

The project has run multiple independent QA cycles (each from scratch, each
assuming no prior audit was complete). Each cycle produced a set of phase
documents with per-test traces, defect reproductions, and regression tables.
Those documents are valuable as a record of *what was checked, when, and how* —
but they freeze at a point in time (test counts, feature counts, defect
statuses) that the living trackers have since moved past.

Keeping them in the active `docs/` folder meant that grepping for "current
feature status" or "how many tests" would return stale numbers from a frozen
audit alongside the authoritative current counts in
[`FEATURES_TRACKER.md`](../../FEATURES_TRACKER.md). Moving them here keeps the
active doc set clean while preserving the full audit trail.

## What lives here

| File | What it is | Frozen at |
|---|---|---|
| [`QA_PHASE1_FEATURES.md`](QA_PHASE1_FEATURES.md) | Canonical 56-feature spreadsheet + 4-role auth-state matrix, from the third QA cycle | 56 features, 168 test scenarios |
| [`QA_PHASE2_3_TESTS_DEFECTS.md`](QA_PHASE2_3_TESTS_DEFECTS.md) | 168 per-test traces (T001–T168) + D-006..D-011 defect details | 189 Rust tests, 56 features |
| [`QA_PHASE5_REGRESSION.md`](QA_PHASE5_REGRESSION.md) | Post-fix regression report + 7 end-to-end journey re-runs | 189 Rust + 14 Python tests |
| [`QA_PHASE6_FINAL_SUMMARY.md`](QA_PHASE6_FINAL_SUMMARY.md) | Closing QA report: coverage summary, defect tally, confidence score (92/100) | 189 Rust + 14 Python tests |
| [`NEXT_FEATURE_INTEGRATION_MAPS.md`](NEXT_FEATURE_INTEGRATION_MAPS.md) | Pre-implementation integration-point research for P-102/P-104/P-105/P-106/P-108 | All features it maps are now shipped (v7/v8) |

## Where the current state lives

| If you need… | Read this (not the archive) |
|---|---|
| Current feature status (does it work?) | [`FEATURES_TRACKER.md`](../../FEATURES_TRACKER.md) — 149 features, 200 tests |
| Current defect log (what's broken / fixed?) | [`DEFECTS.md`](../../DEFECTS.md) — D-001..D-012, all resolved/waived/deferred |
| What shipped in each milestone | [`CHANGELOG.md`](../../CHANGELOG.md) |
| What's planned next | [`docs/ROADMAP.md`](../ROADMAP.md) |
| Navigation index for all docs | [`docs/INDEX.md`](../INDEX.md) |
