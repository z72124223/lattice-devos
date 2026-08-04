# TASK-021 Workflow Audit

## Decision

`READY FOR GOVERNANCE REVIEW`. The repository, cumulative dirty-worktree
boundary, TASK-020 closure, current plan/spec/ADRs, Task Ledger pure API,
Postgres Store transaction/migration/verifier internals, live harness, and Git
state were inspected before implementation. No code or SQL implementation may
start until SPEC v23, ADR-019, both constitution amendments, and TASK-021 pass
an independent governance/architecture review.

## Repository And Git Evidence

- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch/HEAD: `feature/v2-rust-postgres-bootstrap` at
  `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`
- Committed primary/V2 ahead/behind: `0/4`; remote/upstream: none.
- The shared dirty MVP-0-through-TASK-020 result is intentional and must not be
  reset, cleaned, switched, overwritten, or treated as a per-ticket Git diff.

## Reused And Updated Workflow Stages

| Stage | Classification | Evidence or action |
|---|---|---|
| Repository inspection | valid/current | Git, AGENTS, PLANS, HANDOFF, TASK-020 reviews, manifests, domain/store source, SQL, tests, and harness read |
| Requirements clarification | valid/current | approved V2 amendment plus user directive covers durable Ledger; exact API, outbox, schema profile, migration, error, and deferral choices frozen in ADR-019; no material user question remains |
| Specification | updated/current | SPEC-002 v23 AC-35; AC-03/04 remain open until direct evidence |
| Module governance | updated/pending review | Task Ledger 2.1 and Postgres Store 1.3; ADR-019 records the approved conflict amendment and tradeoffs |
| Ticket decomposition | current | one non-parallel TASK-021 with exact allowlist and TASK-020 dependency |
| Branch/worktree plan | reused | existing feature branch/shared worktree; no branch mutation authorized |
| TDD | pending | first implementation action must be a focused failing Task Ledger test |
| Focused/full verification | pending | exact commands and marker-owned PostgreSQL evidence frozen in TASK-021 |
| Code/security review | pending | independent review required after GREEN/refactor |
| Architecture review | pending | required for two module versions, domain dependency, migration, functions, and historical receipt profile |
| Integration | pending | local combined verification required; remote CI remains missing |
| Handoff/ledger | pending | required before ticket completion |
| Merge | blocked | no committed candidate, remote policy/CI, or primary merge authorization |

## Current-State Findings That Drive The Design

- Task Ledger 2.0 has complete pure fake/replay semantics but exposes no Live
  genesis, non-mutating append plan, verified retained command lookup, outbox
  admission, or independently comparable complete checkpoint. An adapter
  cannot safely duplicate its private builders.
- `VerifiedStream` currently discards parsed command records after verifying
  them, so typed terminal receipt reconstruction after restart is unavailable.
- Denied commands do not change the event head. Head-only persistence cannot
  detect a truncated denial tail; the new checkpoint must bind all commands.
- Ledger sequence/resource fields are `u64`; silently storing them as signed
  `BIGINT` would change the existing domain contract. Schema v3 therefore uses
  constrained `numeric(20,0)` with canonical decimal-text transfer.
- `PostgresControlStore` owns its `Client` and complete transaction. A new
  adapter wrapping that public adapter cannot atomically add Ledger rows.
  The approved one-way `postgres-store -> task-ledger` dependency and a
  crate-private shared physical engine avoid adapter-to-adapter composition.
- Store receipt hashes bind persistence evidence. Advancing the global manifest
  without a frozen Store profile would invalidate old exact replay. Schema v3
  must verify the full current profile while reconstructing Store v2 receipts
  from the immutable first-three-entry profile.
- A successfully appended `EFFECT_INTENT` with audit outcome `RECORDED`
  supplies a closed source for exactly one immutable outbox admission. Existing
  non-`RECORDED` outcomes remain append-compatible but create no admission;
  denied and non-effect commands also create none. TASK-021 need not invent a
  provider payload or effect-delivery protocol.
- TASK-020's direct table denial and fixed SECURITY DEFINER function model must
  extend to all Ledger/outbox relations; generic SQL/JSON mutation remains
  forbidden.

## Approved Constitution-Conflict Resolution

Postgres Store 1.2 deliberately lists domain persistence as a non-goal for
TASK-020. TASK-021 is the separately planned repository ticket and therefore
requires the explicit 1.3 amendment rather than silently violating 1.2. The
responsible user already approved `docs/modules/V2_AMENDMENT_PROPOSAL.md`, which
specifically directs PostgreSQL to implement Task Ledger stream-head,
command-receipt, append/outbox atomicity, and projection persistence without
owning event meaning, and later directed execution through MVP-3. ADR-019 and
the versioned constitutions apply that approved change; no new authority or
product direction is inferred.

## Enforcement Truth

- Local Rust/Node tests, exact manifest/catalog/ACL assertions, and the
  marker-owned PostgreSQL harness can be machine-enforced.
- Task Ledger planner parity, checkpoint meaning, module ownership, ticket
  allowlist, TDD order, and protected boundaries are documented and locally
  checked but not enforced by a remote merge system.
- The PostgreSQL role/function/table boundary is machine-verified only for the
  disposable target. It is not production provisioning evidence.
- Remote Rust/PostgreSQL CI, required remote review, branch protection, merge
  queue, committed candidate, and upstream synchronization are missing or
  unverified. This does not block reversible local implementation; it blocks a
  merge-readiness claim.
