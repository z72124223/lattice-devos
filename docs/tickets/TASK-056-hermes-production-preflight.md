---
ticket_id: TASK-056
title: Hermes production preflight
spec_id: SPEC-002
spec_version: 28
module_id: latticed
constitution_version: 1.4
status: completed
parallel_safe: false
depends_on:
  - TASK-054
  - TASK-055
allowed_paths:
  - docs/tickets/TASK-056-hermes-production-preflight.md
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/src/bin/latticed.rs
  - apps/lattice-runtime/src/bin/lattice-full-chain.rs
  - apps/lattice-runtime/tests/composition.rs
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-056 Hermes production preflight

## Objective

Make canonical `latticed` report the strongest statically verifiable state of
its sealed Hermes production configuration, without launching Hermes, MCP,
OpenClaw, Codex,
Graphify, PostgreSQL, or a network operation.

## Acceptance criteria

1. Canonical `latticed --hermes-preflight` reports fixed, redacted
   classifications and never emits configuration values, paths, credentials,
   or raw errors. The legacy `lattice-full-chain` observer rejects this flag.
2. Missing required Hermes configuration is individually named by setting key;
   all other invalid or unavailable dependencies remain fixed classifications.
   Static parsing can report only `CONFIGURATION_PRESENT_UNVERIFIED`; it never
   grants launch authority or claims runtime, asset, containment, or broker
   readiness.
3. The normal no-argument `latticed` and legacy full-chain behavior remain
   fail-closed and unchanged.
4. Focused binary tests, affected runtime tests, formatting, and the available
   project check pass.

## Non-goals

- No configuration mutation, credential installation, runtime download,
  external connection, process launch, MCP surface change, or automatic repair.

## Completion evidence

- `cargo test -p lattice-runtime --all-targets --locked`,
  `cargo test --workspace --locked`, `npm.cmd run verify`, and
  `cargo fmt --all -- --check` passed on 2026-08-13.
- The direct local preflight reported all currently missing setting names and
  no values, paths, credentials, or raw errors. One test preserves unavailable-
  manifest redaction evidence. A separate test writes the exact pinned manifest
  bytes under `CARGO_TARGET_TMPDIR`, verifies their fixed SHA-256, reaches the
  invalid-secret branch with a short secret sentinel, matches the complete
  fixed stderr, proves the secret and owned path are absent, and removes its
  test-owned fixture through scoped cleanup.
- Independent review P1-P2 were repaired. The P3 follow-up now has separate
  unavailable-path and post-manifest invalid-secret branch evidence; independent
  re-review remains pending.
- Strict `lattice-runtime --no-deps` Clippy remains blocked by six pre-existing
  diagnostics on unchanged lines; full workspace Clippy also has eleven
  pre-existing diagnostics in unchanged `lattice-hermes-adapter`. No TASK-056
  line was reported and this ticket adds no lint suppression.
- Implementation and local verification are complete. Independent re-review
  found no P0, P1, P2, or P3 findings after the separate unavailable-manifest
  and invalid-secret branches were verified.
