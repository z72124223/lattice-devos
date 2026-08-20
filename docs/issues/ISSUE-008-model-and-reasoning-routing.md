---
issue_id: ISSUE-008
status: complete
branch: feature/issue-008-model-routing
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
delivery_remote: origin
delivery_repository: z72124223/lattice-devos
---

# ISSUE-008: Model and reasoning routing terminal evidence

## Scope

This issue delivers deterministic, provider-neutral model and reasoning
strength routing for the five governance profiles: Watch, Quick, Standard,
Deep, and Critical. Route selection consumes an injected capability snapshot
and emits requested, resolved, and actual-route evidence without provider
calls, paid services, or external model execution.

## Acceptance Evidence

- [x] All five profiles resolve to deterministic risk floors and reasoning
  effort requirements.
- [x] Unknown or stale capability evidence fails closed without selecting a
  provider.
- [x] Low-risk work does not select the highest-cost qualified capability;
  Critical work cannot fall below its quality floor.
- [x] Standby fallback is accepted only when explicitly compatible with the
  primary route.
- [x] Escalation is one bounded profile step at a time and uses explicit
  trigger reasons and a request limit.
- [x] Actual provider/runtime evidence must match the resolved route exactly;
  mismatches fail closed.
- [x] Injectable/simulated contract tests cover evidence and reason codes.
- [x] Focused policy tests, policy package tests, formatting, Clippy,
  characterization, repository verification, and locked workspace tests pass.

## Implementation And Verification

- Implementation: `crates/lattice-policy/src/model_routing.rs`
- Tests: `crates/lattice-policy/tests/model_routing.rs`
- Implementation checkpoint: `b97293ee67b97926c0dbd0482abc849f3a1653f1`
- Delivery evidence checkpoint: recorded in the commit containing this file.
- Branch delivery is authorized only through non-force push and exact
  `ls-remote` equality for `origin/feature/issue-008-model-routing`.

## Explicitly Deferred

PostgreSQL durable model-routing evidence is not implemented or claimed by
this issue. Integration with TASK-039 and TASK-040 is also not implemented or
claimed. Those remain later dependencies with independent acceptance evidence.

## Ownership Boundary

The implementation is pure policy code. It performs no filesystem, database,
network, provider, or model-runtime I/O. Capability availability is handled
through injected/simulated contract data only.

