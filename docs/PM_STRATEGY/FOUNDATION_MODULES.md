# Foundation Modules — HK Budget 2026-27 "AI+" Alignment

> Four foundation modules that package `hkgov-rethink`'s existing capabilities
> into reusable foundations aligned to the HK 2026-27 Budget Chapter 7 ("AI+")
> direction. Each is independently deliverable; each is the foundation a
> HKGOV-facing project delivery can stand on. Built on
> `feat/foundation-modules-m1-m3-m6-m5`.

## The budget direction (the "why")

[HK Budget 2026-27, Chapter 7](https://www.budget.gov.hk/2026/eng/budget07.html)
("人工智能+ / AI+") commits HKGOV to **AI industrialization + deep integration
with every industry → universal use, universal beneficial use**, anchored by:

| Budget pillar | What they're building |
|---|---|
| **AI+ Strategy Committee** (§35) | FS-chaired body driving data-driven decision-making across gov |
| **AI Cybersecurity Lab** | HKMA + 8 banks + HKCERT + Cyberport — AI for financial-crime / fraud detection |
| **AI Subsidy Scheme** | HK$3B (~30 R&D apps approved) + new **HK$100M** "new quality productive forces" + **HK$50M** responsible-AI-in-society |
| **Digital Policy Office** | Single public data gateway; 100+ public datasets; push departments to open high-value data |
| **FinTech (HKMA)** | GenAI Sandbox + Stablecoin licensing regime |
| **NT Metropolis** | Data-driven planning for the new tech/commercial hub |

The throughline: **data-driven, accountable, AI-everywhere government**, with
responsible/reproducible AI as an explicit mandate.

## What shipped (M1–M5)

| Module | Budget pillar | Status | LOC | Tests |
|---|---|:---:|---:|---:|
| **M1** — Open Data Gateway Foundation | Digital Policy Office (single gateway) | ✅ | +650 | 7 |
| **M3** — Responsible AI Audit Layer | HK$50M responsible AI; AML Lab accountability | ✅ | +779 | 5 |
| **M6** — Transparency Foundation | Data-driven decision-making (flagship) | ✅ | +956 | 4 |
| **M5** — Property / NT Metropolis Intelligence | NT Metropolis + productive forces | ✅ | +966 | 11 |

**Total: ~3,350 LOC, 27 new tests, 0 regressions.** Full workspace builds clean;
294 lib tests pass (199 agent + 73 connectors + 19 store + 3 ingest).

---

## M1 — Open Data Gateway Foundation

**Pillar:** Digital Policy Office — single public data gateway, 100+ datasets.

**What it does:** Adds per-dataset provenance tracking so every served record is
traceable to a verifiable upstream source, and any upstream revision is
detectable by recomputing the content hash. Adds a runtime dataset-registration
API so the platform can serve as the cross-departmental data layer.

**Design:** Sidecar `LineageStore` keyed by `DatasetId` (not fields on
`NormalizedRecord` — avoids touching every record/serialization). The content
hash reuses cite.rs's NaN/Inf-safe canonicalization, so lineage + cite hashes
agree.

**Contracts:**
- `GET /v1/datasets/{src}/{ds}/lineage` — upstream URL, wire format, content
  SHA-256, fetch timestamp.
- `GET /v1/lineage?source=` — the provenance index (every dataset).
- `POST /v1/datasets` — register an external dataset (auth-gated).

**Key files:** `crates/store/src/lineage.rs`, `crates/api/src/routes/gateway.rs`.
HKMA connector overrides `upstream_url()`/`upstream_format()` so every HKMA
dataset carries a verifiable URL.

---

## M3 — Responsible AI Audit Layer

**Pillar:** HK$50M responsible-AI-in-society; AI Cybersecurity Lab
accountability.

**What it does:** Every insight the agent produces carries a `ProvenanceRecord`
attesting how it was made: detector, threshold, evidence SHA-256, detector
version, runtime version, producer (heuristic vs specific LLM model), and —
critically — a `deterministic` flag. Makes the determinism guarantee **typed
and checkable** (previously informal — a code-placement convention).

**Design:** `Finding.deterministic` is a sealed field set only inside
`analysis.rs`; the LLM framing path reads it but cannot flip it. The
`ProvenanceStore` is a sidecar keyed by `insight.id`. The hash reuses cite.rs's
`reproducibility_hash` (NaN/Inf-safe), so provenance + cite hashes agree.

**Contracts:**
- `GET /v1/insights/{id}/provenance` — the full audit trail.
- `GET /v1/audit?since=&producer=&deterministic=` — paginated audit log (the
  regulator/researcher surface).
- `GET /v1/audit/attestation/{id}` — a signed attestation bundle (insight +
  provenance + cite manifest + plain-text claim).

**Key files:** `crates/agent/src/provenance.rs`,
`crates/api/src/routes/audit.rs`. The `provenance_hash_matches_cite_evidence_hash`
test enforces the cross-module hash agreement.

**The headline:** `?deterministic=true` returns findings reproducible in CI;
`?producer=llm:*` returns the LLM-framed ones. A regulator can verify
reproducibility without trusting the system.

---

## M6 — Transparency Foundation

**Pillar:** Data-driven decision-making — the flagship ("system of record for
government data transparency").

**What it does:** Generalizes the Silence Index from HKMA-only + 2 hardcoded
detector kinds to **any source + a pluggable signal registry**, plus a
multi-source composite and a citable quarterly report generator.

**Design:** A `TransparencySignal` trait replaces the `match` arm in silence.rs.
The default registry reproduces the v1 Silence Index byte-for-byte (golden test
pinned). `build_index_from_registry` is the generalized core; the composite is
the events-weighted average across sources. The report generator produces a
Markdown + JSON quarterly report with the score, breakdown, top contributing
insights (each with M3 provenance + cite permalink), and methodology version.

**Contracts:**
- `GET /v1/transparency-index?sources=hkma,rvd,landregistry&period=2026-Q2` —
  multi-source composite (events-weighted).
- `GET /v1/transparency-index/report?source=&period=&format=markdown|json` —
  quarterly report.
- `GET /v1/silence-index` stays as the HKMA-scoped alias (backward compat).

**Key files:** `crates/agent/src/transparency.rs`,
`crates/agent/src/transparency_report.rs`.

**Backward compat:** All existing silence tests pass unchanged; the HKMA-only
default registry reproduces the v1 score.

---

## M5 — Property / NT Metropolis Intelligence Foundation

**Pillar:** NT Metropolis data-driven planning; "new quality productive forces".

**What it does:** Packages the 6 property connectors (RVD, LandRegistry, HKP,
Midland, Chung Sen, AA Property) into a cross-portal market-intelligence module
with a canonical projection layer, a cross-portal divergence detector, and an
NT-Metropolis planner-facing composite.

**The load-bearing finding:** the 6 connectors expose 9 datasets with 5
distinct field vocabularies (price: `price_hkd`/`sale_price_hkd`/`price_10k`/
`price_hint`; area: `build_area_sqft`/`saleable_area_sqft`/`area_sqft`). No
cross-portal comparison was possible before. `property_canon.rs` reconciles them
onto one `CanonicalListing`.

**Design:** `project()` dispatches by source, mapping each portal's vocabulary
onto `CanonicalListing`. Honesty rule: unparseable fields (AA Property's
free-text price) return `None`, not guesses. `detect_portal_divergence` is
pure-Rust (determinism guarantee preserved): compares per-portal medians by
(region, month) bucket, fires when medians diverge > threshold (default 10%).

**Contracts:**
- `GET /v1/property/composite?region=kln&month=2026-06` — cross-portal median
  per-net-sqft price + per-portal breakdown.
- `GET /v1/property/portals` — portal health + dataset coverage.
- `GET /v1/property/divergence?threshold=&region=` — cross-portal divergence
  findings ("are the portals telling the same story?").

**Key files:** `crates/connectors/src/property_canon.rs`,
`crates/api/src/routes/property.rs`.

---

## Build order & rationale

| Phase | Modules | Why this order |
|---|---|---|
| **A** | M1 (gateway/lineage) + M3 (audit/provenance) | Foundation: M3's hash reuses M1's lineage hashing; both are sidecar stores every later module benefits from. |
| **B** | M6 (transparency generalization) | Builds on M3 (report insights carry provenance) and M1 (multi-source composition). |
| **C** | M5 (property foundation) | Most independent; needs M1's lineage for composite provenance + M3 for divergence audit. |

## Design principles (shared)

1. **Sidecar stores, not core-struct bloat.** Lineage/provenance/transparency
   live in sidecar `Arc<RwLock<HashMap>>` stores keyed by existing ids. Core
   structs (`NormalizedRecord`, `Insight`, `DatasetMeta`) stay stable. The only
   core addition is `Finding.deterministic: bool` (M3) — a single bool, set only
   inside `analysis.rs`.
2. **Determinism-first, always.** Every new detector/analysis is pure-Rust.
   `detect_portal_divergence`, `build_index_from_registry`, `content_hash` —
   same inputs → same outputs, no API key required.
3. **Non-breaking trait extensions.** `RecordStore::lineage()` and
   `Connector::upstream_url()`/`upstream_format()` have default impls; existing
   implementations keep compiling.
4. **Hash consistency across modules.** Lineage, provenance, and cite all reuse
   the same NaN/Inf-safe canonicalization, so the three hashes agree on the same
   data (cross-module invariant tests enforce this).

## Scope guardrails (what was NOT done, and why)

- **No Postgres/Redis wiring.** ROADMAP "Remaining" owns this; the new sidecars
  inherit the volatile + file-snapshot shape and gain persistence when the Pg
  tier lands.
- **No dashboard work.** Routes return JSON; the dashboard can adopt these.
- **No PDF rendering.** M6's report ships Markdown + JSON; `format=pdf-data`
  returns the payload a renderer consumes.
- **No new external deps.** Everything reuses existing crates.

## Verification

- **294 lib tests pass** (199 agent + 73 connectors + 19 store + 3 ingest).
- **Backward compat:** all pre-existing silence/cite/store tests pass unchanged.
- **Cross-module invariants:** provenance hash == cite evidence hash; default
  transparency registry reproduces v1 Silence Index byte-for-byte.
- **CI parity:** `cargo build --workspace` clean.

## Sources

- [HK 2026-27 Budget — Ch.7 AI+ (English)](https://www.budget.gov.hk/2026/eng/budget07.html)
- [HK 2026-27 Budget — Ch.7 AI+ (Chinese)](https://www.budget.gov.hk/2026/chi/budget07.html)
- [AI Subsidy Scheme — Digital Policy Office](https://www.digitalpolicy.gov.hk/en/our_work/digital_infrastructure/industry_development/ai_subsidy_scheme/)
- [Cyberport supports the 2026-27 Budget (press release)](https://www.cyberport.hk/wp-content/uploads/Press-Release-Cyberport-Supports-the-2026-2027-Budget-for-Accelerating-IT-and-AI-Development-1.pdf)
- [Hong Kong to Drive AI Integration Under 2026-27 Budget — China Daily HK](https://www.chinadailyhk.com/hk/article/629442)
