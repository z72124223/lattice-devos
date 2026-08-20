# TASK-084 Architecture Review

- Review date: 2026-08-21
- Scope: bounded standalone `workflow-learning` characterization
- Decision: **APPROVE_BOUNDED_SLICE; PRODUCTION_INTEGRATION_BLOCKED**

## Boundary Findings

1. Mission fit passes. The crate owns closed observation, epistemic lifecycle,
   suspicion ranking, stop challenge, candidate promotion, monitor, and rollback
   semantics only.
2. Ownership passes. It creates no Task Ledger, PostgreSQL, Writer Lease,
   dashboard, scheduler, Git, process, filesystem, network, or model writer.
3. Dependency direction passes. The standalone crate uses only `std` and does
   not import orchestration or storage modules.
4. Public contracts are closed enums and validated constructors. Policy input
   records version/base/fence, reason, expected impact, scope, replay cases,
   cost, and allowlisted changes.
5. Failure isolation passes. Unknown/stale/untrusted inputs, active/UNKNOWN/idle
   windows, topology faults, missing gates, stale fencing, and protected
   authority fail closed without side effects.
6. Compatibility is additive. The root Cargo workspace, lockfile, PLANS,
   handoff, branch guide, and all TASK-079 paths remain untouched.

## Required Follow-up

Durable replay and atomic activation remain blocked on TASK-079's committed
Task Ledger/Writer Lease contract. A later integration review must define the
event schema, writer fence, snapshot identity, Postgres migration, and
fresh-process conformance. The in-memory checkpoint is not durable truth.

## Residual Architecture Risk

The bounded crate proves semantics, not composition. Production promotion,
live persistence, scheduler wiring, and zero-work certificate consumption are
not approved by this review.
