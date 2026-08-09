---
ticket_id: TASK-038
spec_id: SPEC-003
spec_version: 2
module_id: latticed
constitution_version: 1.1
status: in-progress
parallel_safe: false
depends_on:
  - TASK-033
allowed_paths:
  - apps/lattice-runtime/src/mcp.rs
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/tests/mcp.rs
  - apps/lattice-runtime/tests/composition.rs
  - scripts/run-task037-full-chain-verification.ps1
  - scripts/start-chatgpt-mcp-tunnel.ps1
  - scripts/test-chatgpt-mcp-tunnel-entrypoint.ps1
  - docs/specs/SPEC-003-chatgpt-mcp-gateway.md
  - docs/tickets/TASK-038-chatgpt-mcp-gateway.md
  - docs/roadmap/TASK-038-MCP-COMPATIBILITY.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_038_2026-08-09.md
  - docs/reviews/CODE_REVIEW_TASK_038_2026-08-09.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_038_2026-08-09.md
  - docs/reviews/INTEGRATION_TASK_038_2026-08-09.md
  - PLANS.md
  - HANDOFF.md
likely_files:
  - apps/lattice-runtime/src/mcp.rs
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/tests/mcp.rs
  - scripts/start-chatgpt-mcp-tunnel.ps1
branch: feature/task-038-chatgpt-mcp
---

## Objective

Deliver a live, bounded ChatGPT MCP Phase 1 checkpoint: the official Secure MCP
Tunnel launches the private `latticed` stdio server, ChatGPT's stateless
`2026-07-28` refresh discovers exactly two closed zero-argument tools, and both
legacy and modern calls inject the same immutable server-owned binding into the
existing typed service boundary.

## Acceptance Criteria

- [x] SPEC-003 public schema, binding, tunnel-entrypoint, and rejection
      criteria pass with direct evidence.
- [x] Existing TASK-037 verifier remains compatible without a production PASS
      claim.
- [x] No second orchestrator, state store, writer, listener, credential store,
      or generic tool surface is introduced.
- [x] Legacy stateful and modern stateless MCP paths pass focused and
      real-binary compatibility tests.
- [x] A restricted runtime key starts the existing tunnel profile, readiness is
      200, and the existing ChatGPT app refresh discovers the two tools.

## Non-Goals

Successful production tool execution, per-human actor/session authorization,
production E2E, public exposure, push, merge, deployment, release, and
TASK-037 Hermes repair.

## Module And Constitution Constraints

`latticed` 1.1 remains unchanged: exactly two zero-argument tools,
composition-owned binding/configuration, one normal composition root, bounded
stdio, typed failures, and PostgreSQL truth.

## Dependencies And Overlap

Not parallel-safe because source, tests, the full-chain verifier, and current
plan share the active MCP contract with TASK-037. The dedicated worktree is
based on checkpoint `845328d` and preserves TASK-037 changes without editing its
branch.

## TDD Behaviors

1. RED/GREEN exact zero-argument discovery schema.
2. RED/GREEN empty-call dispatch with immutable injected binding.
3. RED/GREEN caller-supplied property rejection before dispatch.
4. RED/GREEN real-binary and TASK-037 verifier compatibility.
5. RED/GREEN exact, credential-free tunnel `init`/`doctor`/`run` entrypoint.
6. RED/GREEN stateless `server/discover`, per-request metadata, result/cache
   shape, downgrade rejection, and legacy-lifecycle preservation.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Focused MCP | `cargo test -p lattice-runtime --test mcp --locked` | pass |
| Runtime | `cargo test -p lattice-runtime --locked` | pass |
| Tunnel launcher | `powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/test-chatgpt-mcp-tunnel-entrypoint.ps1` | pass |
| Static/project | `npm.cmd run check` and `git diff --check` | pass |
| Quality | format, scoped strict runtime Clippy, and exact baseline lint reproduction | changed slice passes; unchanged baseline failures recorded |

## Human Gate

The user explicitly authorized the bounded live tunnel/workspace/runtime-key
flow on 2026-08-09. Production completion, public exposure, push, merge,
deployment, and release remain separately gated. The bounded Phase 1 live
discovery checkpoint is complete; the ticket remains in progress for the
explicitly excluded identity and production-execution slices.
