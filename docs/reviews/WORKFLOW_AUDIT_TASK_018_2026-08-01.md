# TASK-018 Workflow Audit

- Date: 2026-08-01
- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Mode: read-only audit before TASK-018 application-code modification

## Confirmed Slice And Continuity

TASK-017 is complete and remains aligned with MVP-1. Its 70 focused tests,
358 full-workspace Rust tests, and 41 Node tests passed; independent final
code/security and architecture review reported zero P0 through P3 findings.
Gateway IPC 1.1 remains pure/fake and AC-07 live OpenClaw evidence remains
open for MVP-2.

TASK-018 freezes the next bounded MVP-1 dependency: a typed zero-I/O physical
store port and deterministic in-memory `PostgreSQL` fake. This slice must prove
transaction request binding, project isolation, optimistic head checks,
all-or-none mutation, exact command retry, command substitution denial, and
unknown-commit reconciliation without connecting to or mutating PostgreSQL.
It does not define domain transition legality, execute SQL, activate runtime
admission, claim durable evidence, or replace One Truth with in-memory state.

The configured project-router entry point remains absent; `PLANS.md`,
`HANDOFF.md`, SPEC-002, ADR-005, and repository-local governance provide the
direct project match. No companion/playmate website is part of this repository
or task.

## Repository And Enforcement Evidence

- Feature HEAD remains four commits ahead and zero behind local `main`.
- No remote/upstream is configured. CI definition exists, but remote execution,
  branch protection, and required reviews are unverified.
- The shared V2 worktree is intentionally dirty and uncommitted. No reset,
  clean, branch switch, commit, push, merge, install, deployment, database
  connection, or external action occurred during this audit.
- The repository has SPEC, ADR, ticket, constitution, project-check, Rust
  format/Clippy/test, Node characterization, independent review, integration,
  workflow-ledger, and handoff conventions.
- Merge readiness remains blocked by dirty/uncommitted state, absent remote
  enforcement, absent synchronization evidence, and absent primary-merge
  authorization.

## Capability Classification Before TASK-018

| Capability | Status | Evidence | Enforcement |
|---|---|---|---|
| Repository/rules/plan/handoff | valid | TASK-017 closure plus current Git state | documented plus machine-observed |
| PostgreSQL truth decision | valid | ADR-005 transaction and authority rules | documented-only |
| Postgres Store constitution | missing | SPEC lists 1.0; no active constitution exists | missing |
| Current TASK-018 ticket | missing | `PLANS.md` marker exists; no ticket exists | missing |
| Store port | stale | nominal `append(AppendCommand)` only | machine-compiled |
| Transaction request/result | missing | no atomic request, expected heads, or receipt | missing |
| Exact command retry | missing | no store-owned request digest/replay contract | missing |
| Unknown commit outcome | missing | generic ambiguous port error only | missing |
| Project isolation | missing | nominal append has no physical project scope | missing |
| Fake current authority | missing | request cannot bind independently retained instance/epoch | missing |
| Durable/live evidence separation | partial | generic fake lane evidence exists | constructor checks only |
| Real PostgreSQL driver/schema/runtime | deferred | TASK-019 onward | missing by design |
| Remote CI/branch protection | missing/unverified | no remote | missing |

## Blocking Findings And Resolution

Four findings block Rust implementation before governance:

1. `AppendCommand -> ControlStoreEvidence` cannot express a bounded atomic
   mutation set, expected current heads, an independently retained fake
   authority head, exact idempotency, or an unknown commit outcome.
2. The proposed Postgres Store section describes the eventual physical driver,
   while the current plan requires a narrower zero-I/O conformance fake first.
   Without a constitution amendment, fake evidence could be mistaken for
   PostgreSQL durability.
3. The current project checker accepts a `CURRENT TASK-018` marker even though
   no TASK-018 ticket exists. It also does not require that the current
   ticket's active module has a constitution. Future SPEC modules may remain
   intentionally inactive, so the safe rule is current-ticket linkage rather
   than requiring every proposed future module immediately.
4. Physical persistence must not become a generic SQL/DML escape hatch or
   duplicate domain truth. Closed owner identities, opaque physical record
   addresses, no SQL text, and no domain-transition Boolean are required.

SPEC-002 v15, ADR-016, Postgres Store 1.0, Contracts 1.8, Ports 1.3, and
TASK-018 must resolve these blockers before RED tests. The project checker must
then machine-enforce exactly one current marker, a matching unique ticket, and
an existing constitution for that current ticket's module.

## Required Execution Order

1. Freeze SPEC-002 v15, ADR-016, the three constitutions, TASK-018, and exactly
   one current plan marker; validate the current-task linkage gate.
2. Add failing shared-contract and typed `ControlStore` port tests.
3. Add failing fake-store tests for bounds, isolation, expected heads,
   all-or-none behavior, retry/substitution, authority drift, and unknown
   commit reconciliation.
4. Implement only the minimum pure Rust contracts and deterministic in-memory
   fake needed to turn each RED case GREEN.
5. Run focused/full tests, strict Clippy/format, dependency/no-I/O/SQL/
   credential/provider/product/governance scans, and diff checks.
6. Complete independent code/security and architecture reviews; every accepted
   finding receives a failing regression before repair.
7. Write integration evidence, ledger, ticket closure, `PLANS.md`, and
   `HANDOFF.md`, then advance to TASK-019 only if all local gates pass.

No unresolved responsible-user decision remains for this bounded pure/fake
slice. Real PostgreSQL connections, roles, migrations, credentials, destructive
schema changes, live runtime authority, protected release, deployment, and
primary-branch merge remain outside TASK-018.
