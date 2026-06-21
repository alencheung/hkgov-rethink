# Contributing to hkgov-rethink

Thanks for considering a contribution. This guide covers the parts that matter:
data-source integrity, code quality, and the feature matrix.

## Data sources — verify before you add

Every connector must target a **real, verified** HKGOV endpoint. The platform
exists to surface authoritative data; a broken or guessed endpoint undermines
that. Before adding a dataset:

1. **Probe it live.** Confirm the exact URL, params, and response shape with
   `curl`. Capture a sample response — if it changes shape, your parser needs
   to handle that.
2. **Document it.** Add the endpoint to `docs/DATA_SOURCES.md` with the verified
   URL, auth requirements, and envelope shape.
3. **Normalize, don't pass through.** Connectors emit
   `hkgov_common::NormalizedRecord`. Never leak upstream-specific types into the
   store or API.

The LandsD gov-only map API (`api.portal.hkmapservice.gov.hk`) stays excluded —
use the open data.gov.hk / CSDI endpoints instead.

## Code quality gates

All PRs must pass:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

If you touch a feature-gated crate, also run clippy with that feature:

```bash
cargo clippy -p hkgov-store --features redis -- -D warnings
cargo clippy -p hkgov-store --features pg -- -D warnings
cargo clippy -p hkgov-common --features otel -- -D warnings
cargo clippy -p hkgov-agent --features llm -- -D warnings
cargo clippy -p hkgov-agent --features alerts -- -D warnings
```

CI runs the default build + tests. Feature-gated builds are the contributor's
responsibility to verify locally (they need external services).

## Feature flags

Optional backends and integrations are behind Cargo features so the default
build stays zero-dependency on external services:

| Feature | Crate | What it enables |
|---|---|---|
| `redis` | `hkgov-store` | Redis-backed `RecordStore` (multi-node cache) |
| `pg` | `hkgov-store` | Postgres-backed `RecordStore` (persistent tier) |
| `otel` | `hkgov-common` | OpenTelemetry trace export |
| `llm` | `hkgov-agent` / `hkgov-api` | HTTP LLM client (insight framing + `/ask` agent loop + function-calling) |
| `alerts` | `hkgov-agent` / `hkgov-api` | Proactive webhook sink for alert dispatch |
| `live` | `hkgov-connectors` | Tests that hit real HKGOV endpoints (never in CI) |

## Architecture invariants

When changing code, respect these — they're what make the platform scale:

- **The API never calls a connector directly.** It reads from the store. Ingest
  writes to the store. One direction.
- **The store is behind a trait.** New backends implement `RecordStore`; no
  other crate changes.
- **The agent reads, never blocks serving.** Agent analysis runs on its own
  scheduler; insights land in the `InsightStore`.
- **Detection stays deterministic.** The LLM only selects which tool to call and
  frames results — every `Finding` originates from a pure-Rust detector in
  `analysis.rs`. The heuristic baseline must always produce the same structured
  findings an LLM would.
- **Config drives everything.** No hardcoded limits, URLs, or cadences.

See `docs/ARCHITECTURE.md` for the full picture.

## Adding a connector

1. Create `crates/connectors/src/<source>.rs` implementing the `Connector` trait.
2. Add your datasets to the `DatasetSpec` list.
3. Register it in `registry.rs::Registry::build` (wrap it with the rate limiter
   + circuit breaker).
4. Add the verified endpoint to `docs/DATA_SOURCES.md`.
5. Add a unit test for the response parser (use a captured sample payload — see
   `hkma.rs` tests for the pattern).

## Adding an agent detector

1. Add a `pub fn -> Vec<Finding>` in `crates/agent/src/analysis.rs`. Set a unique
   `kind`, severity, confidence, and `EvidenceRef`s pointing back into the store.
2. Add a dispatch arm in `scheduler.rs::run_one_target` and in
   `tools.rs::RunDetectorTool` (so it's callable by name via the tool belt).
3. Add a `[[agent.scan]]` example in `config.toml` documenting the new detector.
4. Add a unit test with synthetic records (see the existing detector tests).
4. The detector must be **provider-agnostic** — no LLM calls inside it. The LLM
   client only *frames* findings into prose.

## Commit messages

Follow the existing style: a short imperative summary line, blank line, then a
body explaining the *why*. Reference the data source or ROADMAP item when
relevant.
