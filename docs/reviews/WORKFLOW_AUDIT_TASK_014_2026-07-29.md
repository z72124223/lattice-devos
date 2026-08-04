# TASK-014 Workflow Audit

- Date: 2026-07-29
- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Mode: read-only audit before TASK-014 application-code modification

## Confirmed Slice

TASK-014 freezes Writer Lease 1.0 as a pure Rust semantic owner, a reusable
public transition planner and aggregate verifier, and a deterministic visibly
non-durable fake. It also replaces Policy's caller-owned lease
currentness/fencing fields with a fixed-producer authority receipt plus a head
obtained from an independent Writer Lease owner lookup.

This ticket performs no PostgreSQL, filesystem, Git, process, network,
credential, provider, payment, publication, deployment, product-repository, or
protected-release I/O. PostgreSQL remains the only future durable truth and
will execute the same pure planner inside its transaction in Step 6.

The worktree contains the shared uncommitted MVP-0 and TASK-009 through
TASK-013 result. No reset, clean, branch switch, commit, push, merge, worktree
mutation, publication, or deployment occurred during the audit.

## Audit Evidence

- Git branch/base, dirty-tree preservation, no remote/upstream, PLANS, and
  TASK-013 HANDOFF were re-observed.
- `PLANS.md` had exactly one current marker:
  `CURRENT TASK-014 PLANNING`.
- Baseline Rust:
  `cargo test --workspace --all-targets --all-features --locked -- --list`,
  exit 0, 145 tests.
- Baseline focused Contracts and Policy suites pass with 13 and 75 tests.
- Preserved V1 lock characterization:
  `node --test test/workspace-lock.test.js`, exit 0, 9 tests; the historical
  TASK-004 count of 10 is stale.
- Preserved exact ProjectLock/Workspace Git integration test passes.
- Rust `1.97.1`, Cargo `1.97.1`, and Node `24.16.0` were re-observed.
- The local PostgreSQL 17 service is running, but `psql` is not on PATH. No
  connection or database claim is made by this pure ticket.
- The configured local project-router entry point is absent, so routing used
  this repository's PLANS and HANDOFF directly.

## Capability Classification

| Capability | Status before update | Evidence | Enforcement |
|---|---|---|---|
| Repository/rules/plan/handoff | valid | Git, AGENTS, PLANS, HANDOFF, TASK-013 evidence | documented plus machine-observed |
| Requirements | valid | ADR-002/005/009 and approved V2 amendment | documented-only |
| Specification | partial | SPEC-002 v9 requires One Writer but AC-05 combines pure and PostgreSQL claims | documented-only |
| Writer Lease constitution | missing | proposal exists; no active module constitution | missing |
| Writer authority receipts | missing | no fixed producer/version/runtime/full receipt/head | missing |
| Policy writer consumption | unsafe/partial | caller supplies `active`, `current`, role, epoch, fence, and active count | machine-tested caller-owned model |
| V1 lock | characterization only | local-file lock and counter with 9 focused tests | machine-tested legacy behavior |
| Pure transition/store boundary | missing | no reusable Writer Lease owner API | missing |
| PostgreSQL lease durability | blocked for this ticket | AC-05 requires later transactional integration | deliberately deferred |
| Remote CI/branch protection | missing/unverified | no remote or service evidence | missing |
| Primary merge authorization | blocked | no committed candidate or explicit primary-merge authorization | absent |

## V1 Characterization Findings

Retained semantics:

- only the exact Implementer/holder subject may use a lease;
- one project has at most one active writer;
- lease ID and fencing token both bind validation and release;
- expiry denies continued writing but never proves holder death;
- corrupt, missing, partial, invalid-clock, or path-escape state fails closed;
- Workspace Git ignores only exact ProjectLock-owned metadata.

Rejected as active V2 design:

- filesystem record/counter truth, local caller clock, unchecked JavaScript
  number arithmetic, validation separated from durable mutation, record
  deletion on release, and absence of command receipts/recovery evidence.

Two adversarial reproductions establish the replacement need:

1. rolling the V1 counter from two back to one leaves lease token two
   authorized and causes the next acquisition to issue token two again;
2. a counter at JavaScript `MAX_SAFE_INTEGER` issues an unsafe larger token
   that `validateWriter` still authorizes before a later acquisition fails.

## Ownership And Material Decisions

1. Writer Lease owns lease state, fencing allocation, command idempotency,
   transition/recovery meaning, aggregate verification, and fake composition.
2. `postgres-store` will persist and serialize those transitions but may not
   duplicate them.
3. `lattice-contracts` 1.4 owns only neutral immutable Writer Lease identity,
   positive signed-BIGINT epoch/fence/revision values, runtime-admission
   representation, and authority receipt/full-head values.
4. Policy 2.5 consumes an owner receipt plus independently looked-up complete
   head and removes caller-owned active/current/current-epoch/current-fence/
   active-count fields. Policy has no normal Writer Lease dependency.
5. Runtime admission transitions remain Guardian/PostgreSQL owned. Writer
   Lease accepts a closed observation and evidence digest; it never advances
   daemon epoch or admission itself.
6. `DRAINING` permits exact release and recovery recording but not acquire or
   heartbeat. `CANARY` and `STOPPED` permit no user-project lease transition.
   `RECONCILIATION_REQUIRED` permits only evidence-bound recovery, not normal
   writer activity.
7. New Writer Lease epoch/fence/revision types use `1..=i64::MAX`. Existing
   non-lease Policy recovery fields remain compatibility surfaces for a later
   owner migration and cannot be substituted for Writer Lease values.
8. Expiry at `observed_at >= expires_at` permits only `ACTIVE -> SUSPECT`.
   `SUSPECT` cannot heartbeat back to active. Exact release or evidence-bound
   revoke moves to vacant; reacquire allocates a strictly newer fence.
9. Released and revoked are immutable terminal transition outcomes, not
   current authority states. The current owner lookup returns no authority
   head while the project is vacant.

No unresolved user choice remains in this bounded slice. The existing
fail-closed runtime matrix resolves the only ambiguity in favor of denying
heartbeats during draining.

## Execution Order

1. Update SPEC-002, ADR-009/012, constitutions, routing, and TASK-014.
2. Add Contracts 1.4 RED/GREEN for immutable Writer Lease values.
3. Add Writer Lease public planner/verifier/fake RED/GREEN one transition at a
   time.
4. Add Policy 2.5 receipt/current-head composition RED/GREEN.
5. Run focused/full verification, dependency/no-I/O scans, and governance
   validation.
6. Run independent code/security and architecture reviews, close every
   finding with a failing regression, and repeat integration evidence.
7. Update the workflow ledger, ticket, PLANS, and HANDOFF.

## Minimum Remaining Controls

- Machine-test exact command retry, changed-content reuse denial, legal state
  transitions, expiry/suspect/recovery, fence overflow/non-reuse, aggregate
  replay/corruption, full authority substitution, and runtime-admission
  matrices.
- Keep SPEC-002 AC-05 open for PostgreSQL concurrent acquisition, DB clock,
  restart, stale live connection, and same-transaction mutation fencing.
- Keep authenticated current-owner lookup, process-death observation, remote
  CI, branch protection, commit/merge, and primary merge authorization
  explicitly deferred or blocked.
