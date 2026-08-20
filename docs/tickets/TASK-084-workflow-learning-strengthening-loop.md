---
ticket_id: TASK-084
title: Fail-closed workflow learning and strengthening loop
spec_id: SPEC-007
spec_version: 1
module_id: workflow-learning
constitution_version: 1.0
status: partial
parallel_safe: true
depends_on:
  - TASK-079
branch: feature/task-084-workflow-learning-loop
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: local_only
delivery_archive: keep_open
allowed_paths:
  - crates/lattice-workflow-learning/**
  - docs/adr/ADR-025-fail-closed-workflow-learning.md
  - docs/specs/SPEC-007-workflow-learning-strengthening-loop.md
  - docs/modules/workflow-learning/MODULE_CONSTITUTION.md
  - docs/plans/TASK-084-workflow-learning-loop.md
  - docs/tickets/TASK-084-workflow-learning-strengthening-loop.md
  - docs/reviews/**TASK_084**
---

# TASK-084 - Fail-closed workflow learning and strengthening loop

## Objective

Deliver the non-conflicting pure characterization slice of SPEC-007: historical
incident observation, closed diagnosis/proposal, pre-stop counterexample replay,
machine-gated promotion, monitored rollback, and protected-authority/cost denial.

## Acceptance Criteria

- [x] The TASK-080 and large-project zero-work fixtures reject stop and request
  reconcile/dispatch.
- [x] A versioned zero-work certificate rejects stale, incomplete, quiet,
  unknown/idle-derived, ready-candidate, missing-reason, and missing-tool cases.
- [x] No evidence means no promotion; malicious/untrusted and duplicated
  reports cannot contaminate learning.
- [x] Single-case promotion rejects; multiple hypotheses coexist; fresh
  counterevidence lowers confidence; expiry triggers refresh; a reversible
  bounded probe can overturn the prior judgment.
- [x] Policy version/fence, fresh checkpoint replay, previous-version
  retention, and automatic rollback pass.
- [x] Cost caps and protected permissions cannot be rewritten.
- [x] Real dependency, resource, duplicate-work, and safety gates remain denied.
- [x] A continuous expiring suspicion queue selects minimal high-value probes,
  rejects fresh duplicates, and prevents governance busywork from displacing
  product work.
- [x] TASK-083 nested worktree blocks zero-work/healthy/completion, recommends
  only verified Git-native move, records the successful actual outcome, and
  requires cross-case evidence for path-policy promotion.

## Non-Goals

Task Ledger/PostgreSQL persistence, Writer Lease acquisition, runtime
composition, scheduler/worker control, dashboard/exporter changes, TASK-051,
push, merge, deployment, release, and archive.

## Module And Constitution Constraints

`workflow-learning` 1.0 is pure and owns only closed learning semantics.
TASK-079/Task Ledger/Postgres/Writer Lease retain future durable ownership.
Candidate policy is untrusted and cannot grant authority.

## Dependencies And Overlap

Production durable integration depends on TASK-079. The current pure slice is
parallel-safe because it does not edit TASK-079's active `Cargo.toml`,
`Cargo.lock`, `PLANS.md`, `HANDOFF.md`, branch guide, contracts, ports, Ledger,
Postgres, Orchestrator, dashboard, or foreman-state paths. The crate is
temporarily a standalone Cargo workspace until the dependency is committed.

## TDD Behaviors

1. TASK-080/zero-work replay and all-stale UNKNOWN/idle challenge.
2. Fresh exhaustive zero-work certificate and failure matrix.
3. Trusted evidence, malicious-report rejection, and event deduplication.
4. Multi-hypothesis epistemic record, confidence degradation, expiry/refresh,
   and falsifying reversible probe.
5. Closed proposal, multi-case required machine gates, policy version/fencing.
6. Fresh checkpoint recovery, monitor regression, and rollback.
7. Cost ceiling, protected-authority denial, and preserved blocker conflicts.
8. Suspicion queue ranking, expiry/dedup/refutation, and product-value guard.
9. TASK-083 nested topology, destructive-repair denial, actual move receipt,
   and cross-case path policy.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Focused | `cargo test --manifest-path crates/lattice-workflow-learning/Cargo.toml` | all TASK-084 tests pass |
| Format/lint | standalone Cargo fmt and strict Clippy | exit 0 |
| Constitution | module validator against workflow-learning path | valid 1.0 contract |
| Repository | `npm.cmd run check` and `git diff --check` | result recorded; shared-current mismatch not hidden |

## Human Gate

Bounded local specification/contracts/tests and one local commit are authorized.
Durable integration awaits TASK-079. Non-force push is not authorized because
`delivery_push` is `local_only`; PR, merge, deployment, release, archive,
credential/public-network actions, force, and TASK-051 remain prohibited.
