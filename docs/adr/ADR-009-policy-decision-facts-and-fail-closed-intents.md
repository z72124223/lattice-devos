# ADR-009: Policy Decision Facts And Fail-Closed Intents

- Status: accepted for TASK-011 and amended through Policy Engine 2.6 by
  TASK-015 under the user's directive to execute LATTICE through MVP-3
- Date: 2026-07-29
- Decision owner: user
- Related: SPEC-002 v11, ADR-002, ADR-004 through ADR-008, ADR-010 through
  ADR-013, TASK-011 through TASK-015

## Context

Task Domain now owns the validated immutable Task Spec V2.1, including risk,
capabilities, resource budgets, runtime, network, deployment, and approval
requirements. The early ADR-004 dependency diagram was written before this
separate Rust domain crate existed and shows only
`lattice-policy -> lattice-contracts`. The V1 Policy constitution, however,
already permits Task Domain public types.

Repeating those values as policy-owned strings would create two schema owners
and permit unsafe partial inputs. The V1 JavaScript oracle demonstrates the
consequence: missing Task Spec fields can compare equal to missing lease or
approval fields, and risk/requested capabilities do not restrict actions.

Policy must also consume facts produced later by Project Registry, Approval
Verifier, Writer Lease, capability preflight, PostgreSQL runtime admission,
Codebase Memory, and the guardian without becoming the owner of those facts.

## Decision

Create the pure Rust crate `lattice-policy` governed by Policy Engine 2.1.

The dependency direction is:

```text
lattice-policy
  -> lattice-task-domain
  -> lattice-contracts
```

`lattice-policy` directly consumes the immutable public `TaskSpec`,
`TaskState`, risk, capability, network, deployment, and approval-requirement
types. It may use shared identifiers/digests from `lattice-contracts`. It does
not depend directly on `lattice-cjson`, ports, adapters, stores, registries,
approval/lease implementations, orchestration, or I/O.

Policy evaluates closed, typed decision subjects and returns
`PolicyDecision { allowed, reason, evidence }`. Unknown, missing, stale,
mismatched, replayed, cross-project, over-budget, or wrong-surface inputs return
a deterministic denial rather than an error that callers might ignore.

## Authority Fact Boundary

Policy may consume typed facts for:

- registered project, immutable snapshot identity, project class, and canonical
  primary-branch reference plus Registry-owned physical ref-identity digest;
- runtime admission;
- provider capability identity/version/digest/freshness;
- execution, exact external-cost quote, merge, protected-change, and guardian
  approval;
- fresh merge readiness produced by `workspace-git`, bound to the exact merge
  subject, target head, analysis digest, and Scope Check evidence;
  it also carries the Workspace-Git-resolved physical target-ref identity;
- the exact requested writer target plus current lease/fencing identity;
- fixed-producer resource accounting produced by Task Ledger, bound to the
  exact Task Spec, full stream/resource head, observation revision, claim
  identity, one accounting currency, and checked current/requested counters;
- an exact normal-runtime or guardian-release recovery subject with a typed
  resolved outcome and immutable resolution-evidence digest;
- a memory candidate plus provenance/review bound to that same candidate;
- an immutable release subject plus upgrade manifest/delta/slot/saga/epoch and
  independent guardian-runtime identity.

Those types describe the facts Policy requires; their presence is not proof
that the fact is authentic. Future owner modules must produce and persist the
facts:

- Project Registry owns project status and canonical identity.
- Approval Verifier owns identity/cryptographic verification.
- PostgreSQL/guardian transactions own nonce consumption.
- Writer Lease owns lease, epoch, and fencing transitions.
- Workspace Git owns merge-readiness analysis; Scope Check owns changed-path
  classification evidence.
- Task Ledger owns mutable resource counters and their semantic projection;
  PostgreSQL owns only physical persistence. The effect owner must re-check and
  claim resource counters in the same transaction as the effect claim.
- Capability preflight owners bind provider identity and freshness.
- Codebase Memory owns promotion state.
- The guardian owns activation and rollback state.

Policy never reads a clock or store to make those facts current.

TASK-012/ADR-010 refines the Project Registry boundary without changing this
dependency direction. `ProjectClass`, closed project lifecycle,
`GitRefIdentity`, and the minimal task-agnostic
`ProjectAuthorityReceipt`/current-head representation move to
`lattice-contracts` 1.2. Project Registry owns issuance and lifecycle; Policy
2.3 wraps the receipt with its Task-Spec-specific `SubjectBinding` and compares
every security-relevant receipt field against a head obtained from an
independent current Registry-owner lookup. A receipt's own head projection
does not establish currentness. Policy does not import Registry, and Registry
does not create task identity. Authenticating and serializing that lookup
remains a future Orchestrator/PostgreSQL owner boundary.

TASK-013/ADR-011 applies the same ownership/currentness rule to resource
accounting. `lattice-contracts` 1.3 provides only the neutral full Task Ledger
stream head and resource observation receipt/head representation, with fixed
producer `lattice-task-ledger`, semantic version `2.0`, and exact runtime.
Task Ledger owns issuance, hashes, replay, and counter projection; Policy 2.4
owns only Task-Spec/decision-subject sufficiency and compares every receipt
security field with a head obtained from an independent current Ledger-owner
lookup. Caller-selected owner/producer strings and a `fresh` Boolean are
removed. A receipt's own head projection is structural only. Authenticated
durable currentness and the atomic resource/effect/outbox claim remain a future
Orchestrator/PostgreSQL transaction gate.

TASK-014/ADR-012 applies the same rule to Writer Lease authority.
`lattice-contracts` 1.4 provides only neutral positive signed-BIGINT
epoch/fence/revision values, complete lease identity, runtime-admission
representation, and fixed `lattice-writer-lease`/`1.0` authority receipt/head
values. Writer Lease owns transitions, fencing allocation, recovery, hashing,
and issuance; Policy 2.5 owns only exact Task-Spec/actor/lease/worktree/
daemon/epoch/fence sufficiency and compares the complete receipt projection
with a head obtained from an independent current Writer Lease lookup.
Caller-owned active/current/role/current-epoch/current-fence/active-count
fields are removed. A suspect, historical, missing-head, fake/live-substituted,
or otherwise mismatched receipt denies. Authenticated currentness and
same-transaction durable mutation fencing remain future
Orchestrator/PostgreSQL gates.

TASK-015/ADR-013 applies the rule to approval authority.
`lattice-contracts` 1.5 carries the complete neutral typed approval subject and
fixed `lattice-approval-verifier`/`1.0` authority receipt/head values.
Approval Verifier owns subject hashing, challenge/proof, nonce binding, time/
availability, retry, replay, and claim-precondition semantics. Policy 2.6 owns
only approval-floor/sufficiency and compares the complete receipt with an
independently queried available owner head. Caller subject/identity/freshness/
nonce/self-approval verdict Booleans are removed.

Caller security/architecture review Booleans are also removed. Approval
Verifier does not own review meaning; R3 and other independent-review-required
paths deny until Review Runtime supplies its own fixed-owner receipt/current
head.

The typed approval subject is domain-separated as execution, merge, memory
preference, protected-change intent, or protected release. Merge binds the
canonical Registry classification, target ref, reviewed commit, and diff
digest. Protected change binds both class and immutable operation digest.
Protected release binds activation/release/saga IDs, manifest, source tree and
commit, dependency lock, binaries, migrations, evidence, source/target slots,
target epoch, compatibility, delta, and the exact guardian ID, trust root,
daemon instance, and epoch that will perform activation. Rollback uses a
stage-specific subject that binds the failed activation and reverses its slots
under a strictly newer epoch. A bare caller-supplied digest, raw Git-ref alias,
or unbound boolean cannot substitute for these typed values.

`AgentAction` cannot return allow for merge, memory promotion, protected
change, upgrade lifecycle, or runtime-reconciliation actions. Those operations
are admitted only by their dedicated decision subjects. Normal daemon/effect
recovery and guardian release-saga recovery are distinct subject variants. A
normal Runtime Supervisor may record a resolved effect, proven holder death,
or replaced leadership and move admission only to `STOPPED`; it cannot restore
global `ACTIVE`. Guardian recovery binds the exact Guardian producer and an
owner-produced reconciliation of durable saga, database, and boot state before
it may restore `ACTIVE`.

## Fixed Decision Precedence

Security-relevant evaluation follows this order:

1. decision-subject and known-value validity;
2. project registration, identity, and snapshot binding;
3. runtime admission;
4. role/action and action/state matrix;
5. protected-subject routing;
6. Task Spec capability request;
7. provider capability identity/version/digest/freshness;
8. network/deployment/external-cost envelope;
9. risk floor and exact approval sufficiency;
10. writer lease/fencing;
11. resource budgets;
12. allow.

Recovery stop and guardian rollback remain bounded safety exceptions: budget
exhaustion cannot prevent them, but role, project, runtime, and guardian
authority still apply.

## Risk And Approval Floor

| Risk | Minimum execution authority |
|---|---|
| R0 | `not_required` |
| R1 | `policy` |
| R2 | `responsible_user` |
| R3 | `responsible_user` plus Security and Architecture checks |

A Task Spec may raise but never lower the floor. Primary-branch merge always
requires an exact-subject responsible-user fact. Core release activation
always requires a guardian-trust-root `protected_guardian` fact. Normal
OpenClaw sessions, models, external candidates, active daemons, and Policy
itself cannot satisfy protected guardian authority.

## Fail-Closed Intents

- `allowlisted` networking remains denied until an immutable allowlist digest
  enters an exact subject.
- `authorized` deployment is a request, not authority; public/production
  release stays on a protected surface.
- Unknown external cost denies. New non-zero external cost requires an exact
  amount/currency/provider/quote/pricing subject, a verified fresh quote fact,
  and responsible-user authority.
- Product merge conflicts return a denial requiring a new Implementer task.
- A conflict-free merge requires a fresh exact `workspace-git` readiness fact;
  Registry and merge subjects must carry a fully qualified `refs/heads/*`
  local-branch ref. Git pseudo-refs, revision DWIM, tags, remotes, and shorthand
  branch names deny before Registry classification. Primary classification
  compares Registry and Workspace-Git physical ref-identity digests rather
  than applying Policy-owned case folding; this closes Windows case aliases
  without collapsing valid distinct Unix refs.
- Worker admission and merge cannot introduce external cost. Recovery may
  ignore exhausted historical budget, but its request must add no agents,
  model calls, external cost, network, or deployment effect.
- `RunTests`, product writes, and writable Codex runs require the exact
  fixed-producer current Writer Lease receipt/head plus requested
  writer/lease/worktree/daemon/epoch/fence subject. `ReleaseWriter`
  requires the exact lease target it will release and the same requesting
  actor, unless a separate typed recovery authority performs the release.
- Resource usage from another project/task/revision, a stale observation, an
  unknown owner, a different expected Ledger stream/head/revision/effect claim,
  or a mixed accounting currency denies. Decimal budget math is checked without
  floating point against Task Domain's shared 256-byte, 127-integer-digit, and
  128-fractional-digit bounds. Task Spec 2.1 supplies the immutable accounting
  currency; Policy performs no conversion.
- Policy/constitution/supervisor/schema/security/credential/capability
  expansion, payment, public exposure, irreversible deletion, primary merge,
  and release activation never unlock through a normal task decision.
- The first A/B activation contract permits no schema migration.
- Rollback must reverse the failed activation source/target slots and advance
  to an epoch strictly greater than the failed activation epoch.
- A recovery target is not resolution evidence. Normal recovery to `STOPPED`
  requires a typed effect/holder/leadership resolution plus its immutable
  evidence digest. Restoring `ACTIVE` is Guardian-only and requires exact
  durable saga, database, and boot-state digests whose resolved release,
  manifest, slot, and epoch agree.
- Guardian shadow/activation/health/rollback require a Registry-classified
  LATTICE system project and an exact guardian fact. Health and rollback also
  require a reserved system stream with no user-project access.

## V1 Compatibility

V1 role/action/state denials, protected-action denials, worker limit, lease
staleness, and reason ordering may be retained only as read-only
characterization. V1 project-specific actions, fake-only product ceilings,
unbound approvals, missing-subject equality, and capability/risk omissions are
explicit vulnerability evidence and must not enter V2 active enums or allow
paths.

## Consequences

- Policy cannot authorize an absent or partially reconstructed Task Spec.
- Task Domain remains the sole schema owner and dependency direction stays
  acyclic.
- Exact authority still cannot be claimed until its producing modules and
  persistence paths exist and are live-tested.
- The pure crate can be exhaustively tested without database, filesystem,
  provider, credential, time, or network access.

## Verification

- Exhaustive role/action/state and protected-class matrices.
- Project/snapshot substitution and runtime-admission matrices.
- Risk/approval/capability/provider/lease/resource boundary tests.
- Memory and upgrade non-authority/protected-surface tests.
- V1 compatibility and known-vulnerability regressions.
- Cargo dependency and forbidden-I/O inspection.
- Full Rust and preserved Node verification plus independent reviews.
