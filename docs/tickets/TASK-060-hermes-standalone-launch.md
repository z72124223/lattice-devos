---
ticket_id: TASK-060
spec_id: SPEC-002
spec_version: 29
module_id: latticed
constitution_version: 1.5
status: in_progress
parallel_safe: false
depends_on:
  - TASK-056
allowed_paths:
  - docs/tickets/TASK-060-hermes-standalone-launch.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/modules/latticed/MODULE_CONSTITUTION.md
  - docs/adr/ADR-021-latticed-delivery-composition-and-bounded-mcp.md
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/src/bin/latticed.rs
  - apps/lattice-runtime/tests/composition.rs
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-060 Hermes standalone production launch

## Objective

Make exact canonical `latticed --hermes-launch` own the existing production
Hermes runner for one bounded process lifetime, without creating a second
runner, gateway, truth source, or credential path.

## Acceptance Criteria

1. Exact `--hermes-launch` routes to the existing production configuration and
   runner; unknown or extra arguments retain fixed rejection.
2. Missing preparation, invalid configuration, runner death, stdin failure,
   and teardown ambiguity return fixed redacted failures with empty stdout.
3. Controlled no-credential tests prove one launch, continuous liveness, and
   one explicit teardown on EOF or failure.
   `LATTICE_HERMES_READY` is flushed to stderr only after reader creation and
   live verification; it is ephemeral readiness, not truth or acceptance.
   Stdin bytes are discarded and only EOF/read failure controls lifecycle.
4. A bounded local live attempt uses only pinned local assets and stops at any
   genuine external credential or service boundary without reading its value.
5. Four-tool MCP discovery remains unchanged; focused and affected tests,
   format/diff checks, and independent code/architecture review pass.

## Non-Goals

- No new containment, preflight, runner, MCP tool, durable store, provider,
  download, credential installer, public listener, push, merge, or deployment.

## Module And Constitution Constraints

`latticed` remains the sole composition owner and owns the standalone process
lifetime. Hermes Adapter owns the opaque runner, containment, and reaping
implementation. Process configuration remains adapter-private and cannot enter
CLI arguments, MCP schemas, diagnostics, or durable truth.

## Dependencies And Overlap

Not parallel-safe because it touches the canonical CLI and composition root.
The main agent is the sole writer; all other agents are read-only.

## TDD Behaviors

1. Exact CLI route reaches production configuration.
2. Controlled runner proves launch/liveness/teardown ordering.
3. Child death and teardown ambiguity cannot become exit 0.
4. MCP contract and bounded local live-start evidence remain correct.

## Verification

| Check | Expected evidence |
|---|---|
| Focused CLI/lifecycle tests | fixed failures, one owner, explicit teardown |
| Affected runtime tests | unchanged four-tool MCP contract |
| `cargo fmt --all -- --check`; `git diff --check` | clean |
| Bounded local launch | fixed READY followed by liveness and explicit cleanup, or a precise redacted blocker; zero residual process |

## Human Gate

No additional gate for bounded local work. Reading/providing a real credential,
external model access, public exposure, push, merge, deploy, payment, or account
creation remains a separate explicit user decision.
