## Summary

<!-- One or two sentences: what does this PR do and why? -->

## Change type

- [ ] Bug fix (non-breaking)
- [ ] New feature (detector / connector / endpoint)
- [ ] Refactor / internals
- [ ] Docs
- [ ] Breaking change

## Checklist

- [ ] `cargo fmt --all -- --check` is clean
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean
- [ ] `cargo test --workspace` passes
- [ ] If I touched a feature-gated crate, I ran clippy with that feature
      (`llm` / `alerts` / `redis` / `pg` / `otel`) — see CONTRIBUTING.md
- [ ] If I added/changed a detector, it has a unit test with synthetic records
- [ ] If I added/changed an endpoint, I updated the README API table
- [ ] The determinism invariant holds: **the LLM never performs detection** —
      every finding originates from a pure-Rust detector in `analysis.rs`, and
      the heuristic baseline reproduces it without an API key

## Notes for reviewer

<!-- Anything non-obvious. If this adds a detector, point at the test fixture.
If it changes behavior, note the before/after. -->
