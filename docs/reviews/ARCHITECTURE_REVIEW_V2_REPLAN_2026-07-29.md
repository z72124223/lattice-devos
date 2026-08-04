# Architecture Review — V2 Replan — 2026-07-29

## Result

`BLOCKED_PENDING_USER_APPROVAL`

The proposed architecture is coherent enough to present for approval, but it
cannot be activated or implemented under the current V1 module constitutions.
An independent review initially found three architecture blockers; the proposal
was revised to resolve them at contract level before presenting this decision.
All enforcement remains documented-only until implemented and tested.

## Triggers

- Control core language changes from Node.js to Rust.
- Durable event, approval, lease, memory, and release ownership moves to
  PostgreSQL.
- Eleven new module responsibilities are proposed.
- Seven existing public contracts require versioned amendments.
- Four external process/protocol boundaries are introduced or activated:
  OpenClaw, Codex, Graphify, and Hermes.
- A local IPC surface, database migrations, outbox, projections, memory
  retrieval, A/B release activation, and rollback are introduced.
- Security, reliability, compatibility, and recovery responsibilities change
  materially.

## Before And After

| Concern | V1 prototype | V2 proposal |
|---|---|---|
| Control core | Node.js ESM | Rust workspace |
| Durable workflow truth | per-task files | PostgreSQL event streams |
| Writer lease truth | filesystem record/counter | PostgreSQL transaction/counter/lease |
| Runtime | deterministic fake only | fake first, then capability-gated adapters |
| OpenClaw | inert scaffold | thin local IPC gateway |
| Codex | deferred | exclusive code Implementer |
| Graphify | deferred read-only role | required read-only snapshot adapter |
| Hermes | deferred | required read-only research/candidate adapter |
| Codebase Memory | excluded | provenance/review/retrieval subsystem |
| Self-upgrade | absent | isolated candidate plus guardian A/B activation/rollback |
| Project boundary | named exclusion plus Task project | generic registered-project isolation |

## Responsibility And Ownership Review

### Confirmed strengths

- One normal gateway remains explicit.
- PostgreSQL has one durable control-plane ownership boundary.
- Product code, control events, and active release slot have distinct single
  writers.
- Domain/policy remain pure and adapters remain outside authoritative state.
- Graphify output and Hermes output remain derived/untrusted.
- Memory is prevented from granting authority.
- The guardian cannot implement code, accept ordinary tasks, or approve policy.
- The Node prototype has a migration/characterization role instead of a
  dual-write role.

### Required ownership clarifications

- ADR-006 must choose one writable Codex process/thread owner. The proposal
  selects the Rust core.
- `task-ledger` owns event semantics/canonical bytes/replay while
  `postgres-store` owns physical persistence mechanics.
- `writer-lease` owns lease/fencing/daemon-epoch semantics;
  `workspace-git` owns worktree/Git/filesystem evidence; `postgres-store`
  implements their transaction ports. Policy owns none of these.
- `project-registry` owns canonical repository identity and lifecycle.
- `codebase-memory` owns semantic/review rules while `postgres-store` owns
  physical rows/indexes.
- `artifact-store` owns bytes/atomic write/retention cleanup while PostgreSQL
  stores authoritative references.
- `approval-verifier` owns exact-subject identity/nonce/trust-root proof;
  OpenClaw and the candidate own no release approval authority.
- `review-runtime` owns independent read-only review execution and cannot reuse
  the Implementer's mutation capability.
- `self-upgrade-guardian` owns activation mechanics and is the sole writer to a
  narrow release/daemon-epoch procedure set during handoff; PostgreSQL owns
  durable release events; the Orchestrator owns candidate workflow/promotion
  request.

These splits are explicit in the amendment proposal and must be preserved in
the final constitutions.

## Dependency Review

The proposed direction is acyclic. In this diagram, `A -> B` means A depends on
B:

```text
ports -> contracts
policy -> contracts
orchestrator -> contracts + policy + ports
concrete store/workspace/domain/provider adapters -> ports + contracts
latticed -> orchestrator + concrete adapters
OpenClaw -> public IPC -> latticed
guardian -> narrow release/epoch/status ports + approval-verifier
```

Blocking dependency patterns:

- OpenClaw, Codex, Graphify, or Hermes writing PostgreSQL directly.
- Provider adapters calling one another.
- Memory or Graphify calling Policy as an authority override.
- Orchestrator executing provider-specific protocol logic directly.
- Guardian importing general Orchestrator/task intake.
- Rust core and OpenClaw harness sharing writable Codex thread ownership.

## Failure And Rollback Review

- Intent/outbox before effect addresses crash windows but still needs
  transaction-outcome and reconciliation tests.
- PostgreSQL `LISTEN/NOTIFY` is correctly treated as a wake signal, not queue
  truth.
- Lease expiry is correctly treated as suspect rather than permission to break
  an unknown holder.
- A/B immutable slots and daemon epochs support binary rollback.
- A durable activation saga records intent/outcome around drain, pointer,
  process start, epoch activation, health, finalization, and rollback.
- Except for guardian-only release/epoch procedures, every daemon-authorized
  durable mutation/effect checks active instance, epoch, and runtime admission,
  so an old live connection or pre-finalization candidate is rejected.
- A guardian-only atomic claim consumes the release nonce, appends the claim,
  and enters `DRAINING`; complete drain evidence precedes slot switching.
- Epoch activation enters `CANARY`, where only a reserved system health stream
  is writable. Finalization alone enters `ACTIVE`.
- The first A/B MVP forbids schema migration. Later expansion migration needs a
  separately reviewed compatibility/recovery protocol; destructive data
  rollback remains human-owned.
- Guardian verifies exact release approval through a separate OS-authenticated
  trust boundary. Until that identity/ACL/trust-root path is live-tested,
  promotion is disabled and the control is documented-only.
- OS-admin/same-user hostile process containment remains unproven and is not
  overclaimed.

## Independent Review Findings And Contract Resolutions

### Resolved blocker 1 — partial A/B activation

- ADR-007 now defines explicit durable saga and rollback states.
- The guardian has an intentionally narrow PostgreSQL procedure role that can
  append release receipts and activate daemon epochs even while `latticed` is
  stopped.
- The boot pointer contains activation/manifest/slot/epoch plus checksum and is
  reconciled from PostgreSQL; ambiguous evidence enters
  `RECONCILIATION_REQUIRED`.
- `claim_activation` atomically binds receipt/subject, consumes nonce, appends
  the claim, and sets `DRAINING`. Drain completion requires zero leases,
  effects, unknown outcomes, and writable Codex children.
- The candidate starts in `CANARY`; database admission denies user-project work
  until finalization.

### Resolved blocker 2 — stale daemon still writes

- ADR-005 defines `daemon_instances` and `daemon_leadership`.
- Every daemon-authorized durable mutation—not only events/outbox/leases—checks
  active instance, epoch, and admission in the same transaction. This includes
  registry, artifact metadata, memory, review/capability, and approval state.
- Rollback allocates a higher epoch; an old open database connection cannot
  bypass the stored-procedure checks.

### Resolved blocker 3 — release approval trust chain

- ADR-007 and `approval-verifier` bind authenticated actor/session, exact
  release/manifest/source/binary/migration/delta/slot/epoch subject, nonce, and
  expiry.
- The candidate and normal daemon cannot access the protected trust root.
- OpenClaw can initiate/display but cannot by itself satisfy core-release
  promotion.
- Approval Verifier owns cryptographic/identity validation; the guardian-only
  `claim_activation` transaction is the sole nonce-consumption owner.

### Follow-up ownership corrections

- Graphify consumes an injected `ArtifactStagingPort`; the composition root,
  not the Graphify adapter, connects it to the concrete Artifact Store.
- Review Runtime emits findings/recommendations/evidence only. Task Packet plus
  Policy/Orchestrator and the responsible human where required decide the
  acceptance gate.

These are design resolutions, not execution evidence. Their machine-enforced
status remains missing until implementation, adversarial tests, and a user
acceptance drill exist.

## Compatibility And Migration Review

- SPEC-001 and Task Packet V1 remain historical contracts.
- Node and Rust dual-write is forbidden.
- Rust characterization fixtures are required before replacing retained
  behavior.
- No SQLite migration is claimed because no SQLite implementation exists.
- External component compatibility stays unverified until exact-version
  preflights.
- The dirty V1 worktree requires a preservation gate before any V2 branch or
  worktree is created.

## Confirmed Violations If Implementation Started Now

1. Current V1 constitutions forbid or omit the proposed real adapters,
   PostgreSQL ownership, and self-upgrade modules.
2. SPEC-002 is blocked and not approved.
3. No V2 tickets or exact allowed paths exist.
4. No safe branch/worktree baseline exists because current WIP is dirty.
5. PostgreSQL identity/role/migration capability is unverified.
6. Missing external tools have not been authorized for installation.

Implementation now would violate repository workflow and module governance.

## Risks Requiring Tests

- Canonical JSON differences between JavaScript and Rust.
- PostgreSQL receipt races, serializable retry/idempotency, unknown commit
  outcomes, and at-least-once external effects.
- Lease/fencing `BIGINT` overflow, daemon epoch, and suspect-holder recovery.
- Adapter event duplication/order/schema drift and cancellation ambiguity.
- Memory poisoning, contradiction, project leakage, and no-answer false
  positives.
- Graph snapshot/source mismatch and inferred-edge overtrust.
- Hermes process escape or hidden write behavior.
- Candidate/active schema incompatibility and guardian crash during activation.
- Protected approval replay/identity substitution and trust-root separation.
- Artifact partial writes/digest mismatch/retention cleanup.
- Traditional Chinese and code-symbol retrieval quality.

## Required Decisions And Amendments

- User approval of ADR-006's Rust-owned writable Codex topology.
- User approval of ADR-004, ADR-005, ADR-007, and
  `docs/modules/V2_AMENDMENT_PROPOSAL.md`.
- After approval, create versioned constitutions before tickets or code.

## Integration Blocker

Architecture result: **blocked**.

No code integration, merge-readiness claim, database migration, external
installation, or live adapter work is permitted until the decisions above are
approved and the missing governance stages are completed.
