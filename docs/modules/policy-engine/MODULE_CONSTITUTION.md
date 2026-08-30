---
module_id: policy-engine
name: Policy Engine
version: 2.8
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-28
---

## Mission

Produce deterministic, fail-closed authorization decisions for a validated
Task Spec V2, actor/action/state, exact Registry, Task Ledger, and Writer Lease
receipt/current-head bindings, runtime admission, capabilities, exact Approval
Verifier receipt/current-head bindings, resources, memory promotion, and
upgrade stages without performing any side effect.

## Non-Goals

- Create or mutate a Task Spec, task state, event, projection, or receipt.
- Authenticate identities, verify signatures/MACs, consume approval nonces, or
  decide cryptographic trust.
- Register projects, acquire/release writer leases, advance daemon epochs, or
  change runtime admission.
- Persist or promote memory, create release manifests, or advance an activation
  saga.
- Execute Git, filesystem, database, process, provider, network, deployment,
  payment, credential, or product-repository operations.
- Read a clock, environment, random source, configuration file, or external
  service.

## Owned Data

- Policy contract version 2 and closed V2 role/action sets.
- Role/action/state and protected-subject routing matrices.
- Risk floors, capability requirements, resource envelopes, and stable
  decision precedence.
- Stable policy reason codes and bounded typed decision-evidence shapes.
- Sufficiency rules for facts and immutable receipts supplied by their
  authoritative owner modules.

Policy owns allow/deny meaning only. It owns no mutable project, task,
approval, lease, capability, memory, artifact, or release data.

## Public Contracts

- Evaluate a constructed immutable `TaskSpec`; an absent subject always denies.
- Require exact project ID/snapshot binding before any user-project action.
- Require one validated task-agnostic Registry authority receipt plus a head
  obtained from an independent current Registry-owner lookup; compare the full
  producer/version, runtime, project/snapshot, revision, lifecycle/class,
  primary-ref, observation-digest, and receipt-digest projection.
- Reject stale or substituted full-head fields. Re-projecting
  `receipt.head()` is not currentness evidence.
- Default unknown role, action, state, capability, authority, runtime
  admission, and decision subject to deny.
- Bind every action to the role, state, and Task Spec capability that permits
  it.
- Require current exact provider-capability facts for external lanes and reject
  fake/live substitution or capability drift.
- Enforce Task Spec network/deployment/cost envelopes without treating a
  requested policy as authority.
- Apply risk floors and exact-subject approval sufficiency using one
  fixed-producer Approval Verifier receipt plus a complete head obtained from
  an independent current owner lookup.
- Exactly compare the complete typed execution/cost, merge, memory-preference,
  protected-change, or protected-release subject carried by the owner receipt;
  reject an opaque caller digest or subject sidecar.
- Reject missing, historical, claimed, revoked, expired, self-approved,
  wrong-lane, fake/live-mismatched, or head-mismatched approval authority.
- Fail closed for R3 and any independent-review-required path until Review
  Runtime supplies a separate fixed-owner receipt/current head. Approval
  Verifier cannot substitute for Review Runtime.
- Require a fixed-producer Writer Lease authority receipt plus a complete head
  obtained from an independent current Writer Lease owner lookup for
  product-code writer actions.
- Bind the exact Implementer/actor/lease/worktree/daemon/epoch/fence subject
  and reject missing, suspect, historical, substituted, or head-mismatched
  writer authority. A receipt's own `head()` is never currentness evidence.
- Enforce checked resource budgets and at most four worker agents with at most
  one Implementer per project using a fixed-producer Task Ledger resource
  receipt bound to the exact Task Spec, full current stream/resource head,
  observation revision, effect claim, accounting currency, and counters.
- Require the full resource receipt projection to equal a head obtained from
  an independent current Task Ledger owner lookup. A receipt's own `head()`
  projection and a caller-owned freshness Boolean are never sufficient.
- Evaluate memory promotion and upgrade stages without allowing candidate
  output to grant authority.
- Route merge, memory promotion, protected change, and upgrade lifecycle only
  through their dedicated subjects; the generic action gate cannot authorize
  them.
- Route runtime reconciliation through a dedicated subject that separates
  normal daemon/effect recovery from guardian release-saga recovery.
- Require every recovery subject to carry an exact immutable resolution
  evidence digest and a typed resolved outcome. A normal Runtime Supervisor
  may reconcile only to `STOPPED`; only the exact Guardian lane may restore
  `ACTIVE`, after durable database, boot, and activation-saga state agree.
- Admit guardian canary/recovery only for a Registry-classified LATTICE system
  project and reserved system stream.
- Return `PolicyDecision { allowed, reason, evidence }` with no partial success.
- For managed local execution, `evaluate_execution_gate_with_evidence` must use
  the existing `ExecutionGate` evaluation and return opaque Policy-owned
  evidence that captures the exact Task Spec digest, project binding, Registry
  receipt/current head, task state, and runtime admission evaluated. Downstream
  consumers may inspect but cannot construct or broaden that evidence.
- Provider-dispatch-capable managed evaluation must use
  `evaluate_managed_execution_gate_with_evidence` and capture the exact
  Task-Ledger task reference, successor stream, Task Spec, approval subject,
  and budget. Legacy evidence without that binding is diagnostic-only and
  fails closed as execution authority.
- An allowed managed-execution decision is bounded evaluation evidence, not a
  durable or current execution credential. The final provider-dispatch owner
  must still atomically revalidate the exact persisted Approval evidence,
  validity interval, task/spec/budget binding, and current Project Registry
  authority before admitting a new external effect.

## Invariants

1. Only `Implementer` may receive product-code write or writable Codex
   permission.
2. Integrator may mutate governed Git integration state but never edit product
   files or auto-resolve a product conflict.
3. Reviewers, planners, researchers, Graphify, Hermes, memory, generated data,
   and the guardian cannot acquire product-code write authority.
4. Task, request, Registry authority receipt/head, capability, approval, and
   Writer Lease receipt/head
   must bind the same project, snapshot, task, revision, and Task Spec hash
   wherever those fields apply.
   Merge additionally accepts only fully qualified `refs/heads/*` local-branch
   identities and binds the Registry/Workspace-Git owner-produced physical ref
   identity, canonical primary identity, target, reviewed
   commit, diff, target head, analysis, and scope evidence; writer use/release
   binds actor, worktree, daemon, epoch, and fence; resource facts bind owner,
   an independently requested Ledger stream/head/observation revision/effect
   claim, and currency; memory review binds the exact
   candidate; protected approval binds its class-specific immutable subject
   and guardian runtime.
5. Risk requirements and Task Spec requirements combine by the stricter rule;
   Task input can never lower a risk or protected-action floor.
6. Normal policy approval may authorize only bounded routine work. Primary
   merge and protected release never accept normal OpenClaw/model/candidate
   authority.
7. `DRAINING`, `CANARY`, `STOPPED`, and
   `RECONCILIATION_REQUIRED` deny ordinary user-project mutation according to
   ADR-005/007; exact typed recovery and guardian rollback remain bounded
   exceptions. A rollback reverses the failed activation slots and advances a
   strictly newer epoch. Recovery never equates a requested target with a
   resolved outcome: normal authority can move only to `STOPPED`, while
   Guardian restoration to `ACTIVE` requires exact owner-produced
   DB/boot/saga resolution evidence.
   Rollback binds the exact typed protected-release receipt for the failed
   activation; an opaque, unchecked activation digest cannot substitute.
   Registry status uses one closed lifecycle, never caller-owned
   registered/active/drift booleans. Primary classification compares the
   shared owner-produced physical ref identity
   digests, so case-insensitive storage aliases cannot bypass the protected
   primary-merge floor and case-sensitive repositories retain distinct refs.
   An explicit pseudo-ref denylist rejects revision aliases without rejecting
   valid uppercase branch names. Resource facts use one fixed producer/version,
   exact runtime, and a full independently obtained current Ledger head rather
   than caller-selected owner/producer/freshness fields. Writer facts likewise
   use one fixed producer/version, exact runtime, and a full independently
   obtained current Writer Lease head rather than caller-owned
   active/current/epoch/fence/count fields.
   Approval facts likewise use fixed producer/version, complete typed subject,
   exact runtime/trust lane, and an independently queried full current head
   rather than caller-owned verification/freshness/nonce/self-approval/review
   Booleans.
8. Unknown, missing, stale, replayed, cross-project, mismatched, over-budget,
   overflowed, or wrong-surface evidence denies.
   Recovery cannot carry new network, deployment, agent, model-call, or
   external-cost effects.
9. Memory and external evidence can inform a decision but cannot create a
   capability, approval, lease, scope, project identity, or release authority.
10. Policy performs no I/O and has no hidden clock, mutable singleton, or
    nondeterministic collection ordering.
11. V1 characterization is namespaced and cannot add V1-only actions or unsafe
    behavior to the active V2 contract.
12. Substituting any managed-execution gate input changes its opaque evidence;
    an allowed decision detached from those captured inputs is not reusable
    execution authority.
13. Policy evidence cannot make a historical Approval receipt or Registry head
    current. Exact replay may reproduce prior evidence, but every new provider
    claim independently proves owner currentness at that claim boundary.
14. Substituting task reference, successor stream, Task Spec, approval subject,
    or budget after managed Policy evaluation cannot reuse its opaque evidence.

## Allowed Dependencies

- Rust standard library.
- `lattice-task-domain` 2.1 public immutable Task Spec, state, risk,
  capability, network, deployment, approval-requirement, check types, and
  canonical decimal byte bound.
- `lattice-contracts` 1.5 shared immutable identifiers, Project Registry, Task
  Ledger, and Writer Lease receipt/head values, physical Git-ref identities,
  checked resource usage, signed-BIGINT writer values, complete typed approval
  subjects and approval receipt/head values, and SHA-256 references.

The approved edge remains
`lattice-policy -> lattice-task-domain + lattice-contracts` per ADR-009.
For TASK-013/TASK-014 verification only, Policy integration tests may use
`lattice-task-ledger` as a Cargo `dev-dependency` to obtain a real
`FakeTaskLedger::current_resource_head` result. This dependency is excluded
from the library's normal/production graph and may not import Ledger semantics
into Policy.
Policy integration tests may similarly use `lattice-writer-lease` only as a
`dev-dependency` to obtain an actual fake-owner receipt/current-head pair.
TASK-015 Policy integration tests may use `lattice-approval-verifier` only as
a `dev-dependency` to obtain an actual fake-owner approval receipt/current-head
pair.

## Forbidden Dependencies

- Normal/production dependencies on direct `lattice-cjson`, `lattice-ports`,
  Task Ledger, Project Registry, Writer Lease, Approval Verifier, Codebase
  Memory, guardian, Orchestrator, or concrete adapters/stores. The one
  TASK-013 Ledger `dev-dependency` is test-only composition evidence, never a
  production edge. The TASK-014 Writer Lease and TASK-015 Approval Verifier
  dependencies are likewise test-only.
- Filesystem, Git, database, process, network, clock, randomness, environment,
  credential, model, payment, publication, or deployment libraries.

## Failure, Compatibility, And Migration

Evaluation never fails open. Invalid boundary values and stale/substituted
Registry receipts become typed denials with stable reason codes. Permission
expansion, reason precedence changes, a new role/action/risk floor, relaxed
evidence sufficiency, or a new dependency is a security-sensitive contract
change.

Pure Policy cannot infer that a self-consistent historical receipt/head pair
has become stale without an independently obtained current owner head. In
TASK-012 through TASK-015 this source requirement is documented-only and the
fake Registry/Ledger/Writer Lease/Approval Verifier owners offer current
lookups for composition tests; a later Orchestrator/PostgreSQL boundary must
authenticate and serialize those lookups before live authority is claimed. A
resource, writer, or approval receipt additionally cannot authorize a live
effect until PostgreSQL rechecks counters, daemon epoch/instance, runtime
admission, lease/fence, approval nonce/status/database time, and outbox intent
atomically.

TASK-015 removes caller-owned independent-review Booleans. Until Review Runtime
has its own semantic owner receipt/current-head contract, R3 and every
independent-review-required path deliberately deny. This is a temporary
fail-closed capability gap, not permission to skip review.

The V1 Node policy remains a read-only oracle. Its missing-subject equality,
unbound merge approval, risk/capability omission, fake-only ceiling, and
project-specific action are vulnerability evidence and are not compatible V2
allow behavior.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Default deny | unknown/missing/partial subject matrix | Engineering | yes |
| Role/action/state | exhaustive closed-set matrix | Security review | yes |
| Project isolation | project/snapshot substitution matrix | Security review | yes |
| Registry receipt currentness | independent current-head lookup plus full-field producer/runtime/revision/digest/lifecycle/class/ref substitution matrix | Security review | yes |
| Risk and approvals | risk × requested requirement × complete owner receipt/current-head/typed-subject matrix | Security review | yes |
| Independent review | caller Booleans absent; R3 denies until Review Runtime owner evidence exists | Security review | yes |
| Capabilities | requested/current/version/source/digest/freshness matrix | Security review | yes |
| One Writer | all non-Implementers plus lease/fencing substitution tests | Architecture review | yes |
| Writer authority currentness | fixed producer/runtime plus full receipt/independent-head/status/identity substitution matrix | Security review | yes |
| Runtime and resources | admission, fixed Ledger producer/runtime, independent full current head, substitution matrix, and below/equal/above/overflow budgets | Engineering | yes |
| Memory/upgrade safety | non-authority, protected surface, no-schema-migration tests | Architecture review | yes |
| V1 boundary | retained denials plus known-vulnerability regressions | Engineering | yes |
| No I/O/dependency drift | normal-edge Cargo tree, test-only composition dependency check, and forbidden-reference scan | Architecture review | yes |
| Full verification | Rust workspace and preserved Node suite | Engineering | yes |

## Change Policy

Mission, owned decisions, public subjects, role/action/capability matrices,
risk or approval floors, protected classes, reason precedence, dependencies,
or any permission expansion require a versioned constitution amendment,
specification/ADR trace, security and architecture review, and responsible-user
authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-001, ADR-002 | Initial Node Phase-1 fail-closed policy | Current user task |
| 2.0 | 2026-07-29 | SPEC-002 v5, ADR-009, approved V2 amendment, TASK-011 | Pure Rust project/capability/risk/authority/resource/memory/upgrade policy | User MVP-3 execution directive |
| 2.1 | 2026-07-29 | SPEC-002 v6, ADR-009, TASK-011 independent review RED | Exact owner-bound merge/resource/recovery facts, typed durable recovery resolution, guardian-bound release approval, stage-specific rollback, canonical branch identity, and single-currency accounting | User MVP-3 execution directive |
| 2.2 | 2026-07-29 | SPEC-002 v7, ADR-010, TASK-012 | Replace caller-owned project booleans and Policy-local identity values with an exact shared Registry receipt plus current head | User MVP-3 execution directive |
| 2.3 | 2026-07-29 | SPEC-002 v8, ADR-010 review amendment, TASK-012 | Compare every receipt security field with an independently obtained current Registry head and document the future authenticated-currentness boundary | User MVP-3 execution directive |
| 2.4 | 2026-07-29 | SPEC-002 v9, ADR-009/011, TASK-013 | Replace caller-owned resource owner/producer/freshness fields with a fixed-producer Task Ledger receipt plus independent full current owner head | User MVP-3 execution directive |
| 2.5 | 2026-07-29 | SPEC-002 v10, ADR-009/012, TASK-014 | Replace caller-owned lease active/current/role/epoch/fence/count fields with a fixed-producer Writer Lease receipt plus independent full current owner head | User MVP-3 execution directive |
| 2.6 | 2026-07-29 | SPEC-002 v11, ADR-009/013, TASK-015 | Replace caller-owned approval and review verdict Booleans with complete Approval Verifier receipt/current-head authority; R3 fails closed pending Review Runtime | User MVP-3 execution directive |
| 2.7 | 2026-08-27 | SPEC-011, ADR-028 durable-core review | Clarify that managed Policy decision evidence is not current execution authority and require owner-current Approval and Registry revalidation at new provider dispatch | Delegated product owner |
| 2.8 | 2026-08-28 | SPEC-011, ADR-028 execution-authority security repair | Seal the exact Task-Ledger execution binding into managed Policy evidence and fail closed for legacy unbound evidence | Delegated product owner |
