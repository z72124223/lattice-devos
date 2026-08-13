---
ticket_id: TASK-065
spec_id: SPEC-002
spec_version: 31
module_id: hermes-adapter
constitution_version: 1.1
status: completed
parallel_safe: false
depends_on:
  - TASK-064
allowed_paths:
  - docs/tickets/TASK-065-remove-unused-hermes-helper-gate.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/modules/hermes-adapter/MODULE_CONSTITUTION.md
  - docs/modules/latticed/MODULE_CONSTITUTION.md
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/tests/composition.rs
  - crates/lattice-hermes-adapter/src/broker.rs
  - crates/lattice-hermes-adapter/tests/reflection_api.rs
  - scripts/run-task037-full-chain-verification.ps1
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-065 Remove unused Hermes helper gate

## Objective

Remove the non-executed `lattice-hermes-broker` helper path and digest from
production Hermes admission. Keep the actual direct Codex proxy, process
containment, and the legacy one-shot helper independently intact.

## Acceptance Criteria

1. Production configuration requires twelve executed-input settings; stale
   helper variables have no effect and are never echoed.
2. The production preflight receipt uses domain `v2` and contains no helper
   path or digest. Old identity cannot substitute.
3. The production command still executes the exact verified Codex launcher in
   the existing Job-owned, strict-config path.
4. The legacy helper binary and protocol remain buildable and fail closed.
5. MCP tools/schemas, PostgreSQL truth, provider requests, lazy activation,
   status/task zero effects, and teardown semantics remain unchanged.

## Non-Goals

No live Hermes or model request, credential read, provider change, database
migration, new MCP surface, push, merge, deployment, or release.

## Completion Evidence

- The canonical production preflight now requires twelve executed-input
  settings. A test injects both retired helper variables with sentinel values
  and proves byte-identical redacted output.
- The adapter production config, verified config, factory binding, and receipt
  contain no helper path or digest. The exact `v2` receipt golden differs from
  the former 17-field `v1` identity, while the command plan still launches the
  pinned Codex executable directly.
- The tracked TASK-037 production verifier no longer builds, requires, hashes,
  transmits, or records the helper. Its PowerShell AST parses with zero errors.
- `lattice-hermes-broker` remains independently buildable; no-argument use
  exits 64 with empty stdout and the fixed fail-closed stderr record.
- `cargo test -p lattice-hermes-adapter --all-targets --locked` passed 80 unit
  tests plus 4 preparation tests (9 live-only tests ignored). `cargo test -p
  lattice-runtime --all-targets --locked` passed 92 library, 20 composition,
  1 coordination, 5 dispatch, 31 MCP, and 1 task-control tests. `npm.cmd run
  verify` passed 48/48, and format/diff/project checks passed.
- One full-workspace run reached an unchanged `process` fake-Codex child and
  exceeded its outer bound; the run-owned process tree was stopped. Affected
  adapter and runtime suites subsequently passed in full. No credential,
  Hermes/model request, push, merge, deploy, or release was performed.
