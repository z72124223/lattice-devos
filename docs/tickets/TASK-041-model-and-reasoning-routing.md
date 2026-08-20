---
ticket_id: TASK-041
spec_id: ISSUE-008
spec_version: 1
module_id: lattice-policy
constitution_version: 1.0
status: complete
parallel_safe: false
depends_on: []
allowed_paths:
  - crates/lattice-policy/**
  - docs/tickets/TASK-041-model-and-reasoning-routing.md
branch: feature/issue-008-model-routing
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
delivery_remote: origin
delivery_repository: z72124223/lattice-devos
---

# TASK-041: Model and reasoning routing

## Objective

Provide deterministic, provider-neutral model and reasoning-strength routing
for the five Issue 8 governance profiles: Watch, Quick, Standard, Deep, and
Critical. Route selection consumes an injected capability snapshot and emits
requested, resolved, and actual-route evidence without making provider calls.

## Acceptance Criteria

- [x] All five profiles resolve to deterministic risk floors and reasoning
  effort requirements.
- [x] Unknown or stale capability evidence fails closed without selecting a
  provider.
- [x] Low-risk work does not select the highest-cost qualified capability;
  Critical work cannot fall below its quality floor.
- [x] Standby fallback is accepted only when the capability is explicitly
  compatible with the primary route.
- [x] Escalation is one bounded profile step at a time and is controlled by
  explicit trigger reasons and a request limit.
- [x] Actual provider/runtime evidence must match the resolved route exactly;
  mismatches fail closed.
- [x] Evidence and reason-code contracts are covered by injectable/simulated
  tests; no paid or external model call is part of this ticket.
- [x] Focused policy tests, policy package tests, formatting, Clippy,
  characterization, repository verification, and locked workspace tests pass.

## Completion Evidence

- Implementation: `crates/lattice-policy/src/model_routing.rs`
- Tests: `crates/lattice-policy/tests/model_routing.rs`
- Base implementation commit: `b97293ee67b97926c0dbd0482abc849f3a1653f1`
- Current delivery commit: recorded by the feature branch after this ticket
  evidence repair.
- Remote branch: `origin/feature/issue-008-model-routing`, non-force push
  required and verified by `ls-remote`.

## Explicitly Deferred Dependencies

PostgreSQL durable model-routing evidence is not implemented or claimed by
this ticket. Integration with TASK-039 and TASK-040 is also not implemented or
claimed; those remain later dependencies with their own acceptance evidence.

## Verification

The Issue 8 characterization and the injected policy contract tests are the
authoritative acceptance boundary for this ticket. The implementation is pure
policy code and performs no filesystem, database, network, provider, or model
runtime I/O.

