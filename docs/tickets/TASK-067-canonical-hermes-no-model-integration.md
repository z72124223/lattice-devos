---
ticket_id: TASK-067
spec_id: SPEC-002
spec_version: 31
module_id: latticed
constitution_version: 1.7
status: ready
parallel_safe: false
depends_on:
  - TASK-066
allowed_paths:
  - docs/tickets/TASK-067-canonical-hermes-no-model-integration.md
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/tests/composition.rs
  - crates/lattice-hermes-adapter/src/production.rs
  - crates/lattice-hermes-adapter/tests/reflection_api.rs
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-067 Canonical Hermes no-model integration

## Objective

Prove the canonical production Delivery Run can own the pinned local Hermes
runtime through a deterministic no-model provider fixture, validate one bounded
candidate, and tear down exactly, without credentials or external effects.

## Acceptance Criteria

1. Canonical production startup remains lazy; Status and Task tools launch no
   Hermes process.
2. One Delivery Run activates one pinned local Hermes owner and reaches the
   existing deterministic fake provider without a model or credential.
3. Candidate schema, graph binding, and provenance are validated before any
   persistence authority is granted.
4. Success and failure both prove one bounded teardown and no residual owned
   process or run root; ambiguous teardown fails closed.
5. No non-loopback listener, provider/model request, credential read, MCP
   schema change, push, merge, deployment, or release occurs.

## Non-Goals

No real provider/model, credential, public network, PostgreSQL restart, push,
merge, deployment, payment, account change, or release.
