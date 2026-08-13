---
ticket_id: TASK-073
spec_id: SPEC-002
spec_version: 31
module_id: hermes-adapter
constitution_version: 1.1
status: ready
parallel_safe: false
depends_on:
  - TASK-072
allowed_paths:
  - docs/tickets/TASK-073-retire-definitive-hermes-recovery.md
  - crates/lattice-hermes-adapter/src/lib.rs
  - crates/lattice-hermes-adapter/src/production.rs
  - crates/lattice-hermes-adapter/tests/reflection_api.rs
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-073 Retire definitive Hermes recovery receipts

## Objective

Remove the active same-process recovery receipt whenever either an initial
event stream or a later reconciliation observes an authoritative `Failed` or
`Cancelled` terminal. An already-terminated run must never remain advertised
as active or recoverable.

## Acceptance Criteria

1. An initial SSE `run.failed` or `run.cancelled` preserves its exact typed
   failure but clears `active_run` and attaches no recovery receipt. Other
   non-terminal, malformed, cross-bound, timeout, transport, HTTP, or ambiguous
   observations remain fail closed under their existing receipt policy.
2. After one known-run observation timeout, reconciliation to `Failed` or
   `Cancelled` preserves the exact terminal failure, clears `active_run`, and
   attaches no recovery receipt. Reusing the retired receipt is rejected
   before endpoint I/O, and the original run submission count remains one.
3. A pinned official Hermes no-model acceptance withholds then releases a
   failing fake Codex terminal. Same-port reconciliation returns the exact
   fixed failure, clears the receipt, observes exactly one `turn/start`, tears
   down the proxy once, and removes the owned root.
4. Existing uncertain reconciliation failures continue retaining their
   recovery receipt; successful reconciliation remains unchanged.
5. No public API, error/receipt schema, dependency, persistence, database,
   MCP, FullChain, credential, model, external-network, runtime-identity, or
   ownership contract changes.

## Non-Goals

No retry loop, resubmission, durable or cross-process recovery, canonical MCP
or PostgreSQL end-to-end claim, real provider/model request, external
credential read, public listener, push, merge, deployment, payment, account
change, or release.
