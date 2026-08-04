# TASK-016 Workflow Audit

- Date: 2026-07-30
- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Mode: read-only audit before TASK-016 application-code modification

## Confirmed Slice

TASK-016 freezes Artifact Store 1.0 as the project-scoped semantic owner for
content-addressed objects, immutable reference/provenance manifests,
availability, bounded retention, exact-command behavior, replay, and safe
sweep planning. A visibly non-durable in-memory fake will prove these
semantics before a PostgreSQL metadata adapter or filesystem blob adapter can
perform live I/O.

The bounded implementation will not install, invoke, or integrate OpenClaw,
Codex, Graphify, Hermes, PostgreSQL, or a model. It will not touch a product
repository, create a product worktree, delete a real file, use a credential,
publish, deploy, or activate a release.

## Preceding Task Continuity

TASK-015 remains complete and directly serves the platform-wide MVP-1 goal:

- Approval Verifier owns approval subject/proof/nonce/currentness semantics.
- Policy consumes fixed-owner approval receipt/current-head evidence and
  fails closed for missing Review Runtime authority.
- Artifact Store does not change or duplicate those authorities.
- No active file introduces an unrelated website or project-specific product
  dependency.

The next dependency remains Artifact Store because Graphify, Hermes, Review
Runtime, Codebase Memory, release evidence, and PostgreSQL metadata must not
invent incompatible artifact references.

## Baseline Evidence

- `PLANS.md` has exactly one current marker:
  `CURRENT TASK-016 GOVERNANCE`.
- `HANDOFF.md` identifies Artifact Store 1.0 as the next bounded slice.
- TASK-015 final verification passes:
  `npm.cmd run verify` reports 177 files, 15 constitutions, and 38 passing
  Node tests; `git diff --check` exits zero.
- TASK-015 handoff records 218 passing Rust workspace tests plus focused
  Contracts, Approval Verifier, and Policy checks.
- Feature HEAD is four commits ahead and zero behind local `main`; no remote
  or upstream is configured.
- The shared V2 worktree remains intentionally dirty and uncommitted. No
  reset, clean, branch switch, commit, push, merge, deployment, or external
  action occurred.
- The configured project-router entry point
  `C:\Users\f7212\OneDrive\文件\codex 個人化\scripts\codex-memory-router.mjs`
  is absent and returned `MODULE_NOT_FOUND`; repository plans and handoff
  provide the direct project match.

## Capability Classification Before TASK-016

| Capability | Status | Evidence | Enforcement |
|---|---|---|---|
| Repository/rules/plan/handoff | valid | TASK-015 closure and current Git state | documented plus machine-observed |
| Artifact module constitution | proposal only | V2 amendment section | documented-only |
| Project-scoped artifact identity | missing | only generic SHA-256 value exists | missing |
| Immutable provenance/reference contract | partial | adapter prose only | documented-only |
| Producer trust boundary | partial | lane-specific adapter evidence exists | no artifact-owner receipt |
| Byte/metadata size bounds | missing | no Artifact Store type or crate | missing |
| Exact command idempotency | missing | no Artifact Store aggregate | missing |
| Replay/currentness/rollback check | missing | no Artifact Store aggregate | missing |
| Retention/sweep authority | unsafe if implemented directly | prose requires references but no exact head | missing |
| Filesystem atomic publication | deferred | no owned filesystem adapter | missing |
| PostgreSQL durable references | deferred | no PostgreSQL vertical slice | missing |
| Remote Rust CI/branch protection | missing/unverified | no remote configured | missing |
| Primary merge authorization | blocked | no committed candidate or authorization | absent |

## Ownership Decisions

1. Contracts 1.6 carries immutable project-scoped artifact object/reference,
   provenance, availability, receipt, and complete current-head
   representations only.
2. Artifact Store 1.0 owns manifest hash selection, byte digest verification,
   object/manifest/reference/task/project/staging quotas, object generation,
   typed reference authority, exact retry, availability, replay, checkpoint,
   durable-delete-claim/unknown-outcome reconciliation, and sweep eligibility.
3. Object identity is `(project_id, sha256)`; cross-project physical
   deduplication and existence disclosure are forbidden in 1.0. A positive
   generation prevents a stale sweep from deleting reintroduced bytes.
4. Every retained use is a separate immutable reference bound to project,
   snapshot, task, task revision/spec, attempt, request, producer/adapter
   identity and binary, runtime, schema/media/bundle metadata, correlation/run/
   sequence/produced-at/payload, inputs/config/evidence, Registry/effect/daemon/admission/
   capability-owner authority, and hash-bound limit snapshot.
5. A Graphify, Hermes, Codex, model, or product repository may appear only as
   provenance. Only fixed `lattice-artifact-store` receipts represent Artifact
   Store state, and those receipts grant no content-trust, policy, code-write,
   memory-promotion, approval, or release authority.
6. Initial publication/reference, retain/release, and read claims accept only
   a typed fixed-owner authority receipt plus an independently obtained
   complete current owner head bound to exact action, owner
   record/revision/status, project/task/object/generation/reference/read, and
   runtime. The fake accepts visibly fake pairs only; live owner contracts and
   same-transaction authentication remain deferred.
7. Delete claim requires internally recomputed zero references/quota,
   retention/grace expiry, exact generation/current head, database time,
   root/daemon/epoch/admission binding, and a typed sweep-authority
   receipt/current-head pair. A unique durable token blocks retain/read;
   unknown outcome enters reconciliation rather than guessing safety.
8. Project/store bytes count each non-deleted generation once; task bytes
   count once per object with an active task reference. Read/reference/staging/
   command/history counts are exact, and separate object/task/project/store
   aggregates update atomically. Delete-claimed/reconciliation/orphan state
   retains worst-case quota until verified terminal reconciliation.
9. PostgreSQL will serialize authoritative metadata and references without
   redefining lifecycle/hash semantics. A future filesystem adapter owns
   staging, flush, atomic rename, verified read, link/root containment, and
   exact unlink mechanics without becoming metadata truth.

No unresolved responsible-user decision remains for this bounded pure/fake
slice.

## Required Execution Order

1. Activate SPEC-002 v12, ADR-014, Artifact Store 1.0, Contracts 1.6, and
   TASK-016 before code.
2. Add shared immutable artifact values and full substitution tests.
3. Add the pure Artifact Store aggregate and deterministic in-memory fake
   through RED/GREEN tests for publication, typed reference authority,
   aggregate quotas, idempotency, replay, checkpoint, delete claim/token,
   unknown outcome, reconciliation, and sweep planning.
4. Run focused/full tests, dependency/no-I/O/secret/project-governance checks.
5. Complete independent code/security and architecture reviews; reproduce
   each accepted finding with a failing regression before repair.
6. Write integration evidence, workflow ledger, ticket closure, PLANS, and
   HANDOFF.

## Minimum Remaining Controls

- Full object/reference/producer/receipt/head substitution matrices.
- Empty and maximum-size objects, manifest/object/reference/task/project/
  staging limits, over-limit streaming, digest mismatch, and concurrent
  duplicate semantics.
- Exact retry before stale-head/time evaluation; changed-command denial with
  zero partial mutation under object-scoped storage key, complete sanitized
  request retention, and separate hash domains.
- Cross-project/object/generation/reference/owner-action substitution denial.
- Raw replay, denial-tail, reference-set digest, and trusted-checkpoint
  rollback regressions.
- Durable claim-token, retain/read block, known no-effect, unknown outcome,
  and verified reconciliation semantics; no public real filesystem deletion
  in this slice.
- Later live gates remain PostgreSQL atomicity/durability/restart, filesystem
  crash/flush/rename/link/cleanup evidence, provider staging, and authenticated
  reference-owner composition.
