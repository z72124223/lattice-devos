# TASK-022 Workflow Audit

## Decision

`GOVERNANCE RE-REVIEW PASS; FIRST CHARACTERIZATION/RED STEP RELEASED`.

The first independent TASK-022 governance review returned
`CHANGES REQUIRED — IMPLEMENTATION BLOCKED` with P1=5 and P2=4. The current
governance files now contain the requested corrections. The fresh independent
re-review in `GOVERNANCE_REREVIEW_TASK_022_2026-08-03.md` returned PASS with
P0=P1=P2=P3=0 and released the governance blocker for the first bounded
characterization/RED step. That decision does not claim TASK-022 implementation
or acceptance.

## Current Correction Classification

| Artifact or stage | Current classification | Evidence and remaining action |
|---|---|---|
| SPEC-002 | current | Version 24 and AC-36 are present; no implementation evidence is implied |
| Project Registry constitution | current | Version 1.2 freezes the pure canonical/checkpoint/retained-state/currentness owner boundary |
| Postgres Store constitution | current after correction | Version 1.4 now mirrors ADR-020's vacant singleton, catalog/signature, current-checkpoint, and timeout contracts |
| TASK-022 ticket | current | Version 24 / Postgres Store 1.4 ticket, allowlist, TDD order, and acceptance matrix remain the bounded implementation contract |
| ADR-020 | accepted | The first review findings were incorporated and the fresh independent re-review passed with no open P0-P3 findings |
| First governance review | changes required | P1=5, P2=4; implementation blocker was not released |
| Independent governance re-review | passed | `GOVERNANCE_REREVIEW_TASK_022_2026-08-03.md`: P0=P1=P2=P3=0; governance blocker released |
| TDD implementation | current | Only the first Registry 1.1 characterization and focused Registry 1.2 RED step is released; no implementation evidence exists yet |

The corrected central decisions are now explicit:

- `0005` seeds exactly one Live vacant Registry singleton with command
  high-water/counts `0`, retained bytes `103`, and checkpoint digest
  `5bb1f9d9adf7228bef7ea45cc029e79d71761c4dfed3abe086634917db38c173`;
  the other four Registry tables are empty and first-seen commands are `1..N`.
- Project Registry owns the acyclic canonical graph, exact logical-retained-
  state accounting, and independently retained checkpoint. The adapter only
  persists, reloads, and compares replay against that separate singleton.
- Schema v4 is frozen at exactly 15 `control` tables, 28 retained functions,
  17 runtime-executable functions, and 11 historical functions retained
  without runtime EXECUTE.
- The nine Registry functions use exact scalar input counts `12`, five reads at
  `2` each, `73`, `22`, and `27`; maximum `73`. Composite/table arguments,
  builtin array row maps, JSON payloads, and alternate overloads are forbidden.
- PostgreSQL uses 5-second lock, 30-second statement, and 30-second idle-in-
  transaction timeouts. Rust adds a 45-second monotonic begin-to-pre-commit
  deadline. Timeout/deadline failures roll back as typed `Unavailable`, are not
  commit-unknown, and are not automatically retried.
- Historical Project Registry 1.1 golden vectors are observation, request,
  authority-receipt, and command-result digests. Checkpoint and record-set
  vectors are new in Project Registry 1.2; no historical record-set or separate
  terminal-receipt digest is claimed.

## Original Audit Snapshot (Preserved)

TASK-021 was complete and the next bounded slice directly served the approved
MVP-1 offline-control-core goal. The original audit allowed TASK-022 governance
work and an independent governance review, but blocked Rust/SQL until SPEC-002
v24/AC-36, ADR-020, both constitutions, and TASK-022 agreed.

At that original snapshot, the ticket and ADR proposal existed while the
working files still identified SPEC-002 version 23, Project Registry 1.1, and
Postgres Store 1.3. That historical observation and the inventory counts below
are preserved as snapshot evidence; they are not the current classification.

## Scope And Audit Method

- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Audited slice: TASK-022 durable global Project Registry planning,
  schema-v4 proposal, fixed-function PostgreSQL adapter boundary, and the
  workflow required before implementation.
- The `workflow-audit` inventory script was run from the repository root with
  hidden workflow files included and bulk dependency/build outputs excluded.
- The earlier pre-review inventory observed 134 dirty paths and 267 scanned
  files. A current rerun at `2026-08-03T15:05:30+08:00`, after the TASK-021
  final review artifacts landed, observed 136 dirty paths and 269 scanned
  files. These cumulative counts are snapshot-sensitive; the named artifacts,
  branch, HEAD, and safety boundary are the stable review target.
- No application code, SQL, Git state, external service, database, or protected
  action was changed by this audit.

## Repository And Git Evidence

- Branch/HEAD: `feature/v2-rust-postgres-bootstrap` at
  `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Feature versus primary: four committed commits ahead and zero behind.
- Remote: none. Upstream: none. Remote synchronization and branch protection
  therefore cannot be verified.
- The dirty tree is the intentional cumulative MVP-0-through-TASK-021 result
  plus TASK-022 governance work. It is not a clean per-ticket diff and must not
  be reset, cleaned, switched, or overwritten.
- The repository contains one root `AGENTS.md`; no nested repository
  `AGENTS.md` or `AGENTS.override.md` was found.
- No commit, push, merge, reset, clean, branch switch, deployment, production
  database operation, credential operation, or release action is authorized by
  this audit.

## Automatic Project And Memory Routing

The mandatory router entry point documented by the global workflow is absent:

`C:\Users\f7212\OneDrive\文件\codex 個人化\scripts\codex-memory-router.mjs`

`Test-Path` returned false, and a filename search across the OneDrive tree
found zero `codex-memory-router.mjs` matches. The routing entry point is thus
`broken` for this task. This does not block TASK-022 governance because the
current repository root, `PLANS.md`, `HANDOFF.md`, completed TASK-021 ledger,
and explicit TASK-022 ticket identify the project and next bounded slice
without ambiguity. No routing result was used to delete, archive, switch, or
mutate anything.

## Capability Inventory

The current inventory found:

| Capability | Count | Important evidence |
|---|---:|---|
| Repository instructions | 1 | `AGENTS.md` |
| Plans | 3 | `PLANS.md`, `docs/plans/BRANCH_WORKTREE_PLAN.md`, `docs/plans/V2_BOOTSTRAP_PRESERVATION.md` |
| Specifications | 2 | SPEC-001 retained; SPEC-002 active |
| Tickets | 22 | TASK-001 through TASK-022, including the new TASK-022 ticket |
| Module governance artifacts | 20 | module constitutions plus V2 amendment and module index |
| Architecture decisions | 20 | ADR-001 through proposed ADR-020 |
| Test files/evidence paths | 46 | Rust, Node, and specification fixtures |
| Build/test manifests | 16 | workspace/package Cargo files and `package.json` |
| CI definitions | 1 | `.github/workflows/ci.yml` |
| Git hooks | 0 | none found |
| Release/rollback documentation | 0 | none found |

The one CI file runs the preserved Node verification on pull requests and
pushes to `main`. It does not run Rust or the disposable PostgreSQL harness,
and file presence does not prove any remote run or required-check policy.

## Original Workflow Stage Classification (Preserved Snapshot)

This table records the pre-review classification at the original inventory
snapshot. The Current Correction Classification above supersedes its stale
version rows for the present governance gate.

| Stage | Classification | Evidence or required action |
|---|---|---|
| Repository/workflow audit | reused and current | Current Git/worktree, rules, plans, TASK-021 closure, TASK-022 proposal, source/test inventory, package commands, CI, and missing controls inspected; this report records the result |
| Requirements clarification / `grill-me` | reused and resolved | The approved V2 plan and user directive authorize the bounded durable Registry slice; ADR-020 resolves the global-aggregate, denial-without-snapshot, checkpoint, limit, migration, retry, and deferral decisions without adding a new product direction |
| Behavior specification | partial / governance in progress | TASK-022 names SPEC-002 v24 and AC-36, but the current SPEC file is still v23 and ends at AC-35; v24/AC-36 must be present and reviewed before implementation |
| Module constitutions | partial / governance in progress | TASK-022 requires Project Registry 1.2 and Postgres Store 1.4; the current files still identify 1.1 and 1.3, respectively |
| Architecture decision | partial / governance in progress | ADR-020 contains the proposed global Registry exception and schema-v4 design but remains proposed pending independent review |
| Ticket decomposition | present / pending governance agreement | One non-parallel TASK-022 depends on completed TASK-021 and has an explicit compatibility-unit allowlist, TDD behaviors, verification commands, non-goals, and protected boundaries |
| Branch/worktree/dependency plan | reused and current | Existing cumulative V2 worktree and one-way `postgres-store -> project-registry` direction; no new branch/worktree or overlap is authorized |
| TDD implementation | missing for TASK-022 | No TASK-022 RED/GREEN implementation evidence exists yet; first implementation action must be a focused failing pure Registry test after governance passes |
| Focused/full verification | pending | Ticket freezes pure Registry, adapter, Store package, PostgreSQL 17.10, format, Clippy, full Rust/Node, dependency, audit, and scan gates; none proves TASK-022 until run on its implementation |
| Independent code/security review | pending | Required after implementation and current verification; no TASK-022 code review exists |
| Independent architecture review | pending | Required because two module contracts, a domain dependency, global persistence exception, migration profile, ACL/function surface, and recovery model change |
| Local integration | pending | Combined Rust/Node/PostgreSQL behavior, conflict scan, synchronization facts, and compatibility must be recorded after blocker-free reviews |
| Handoff/workflow ledger | pending | TASK-022 completion requires updated `HANDOFF.md`, `docs/workflow/WORKFLOW_LEDGER.md`, and exact remaining-decision record |
| Merge | blocked | There is no committed TASK-022 candidate, remote/upstream, verified remote CI, required review policy, branch protection evidence, or primary-branch merge authorization |

## Original Snapshot Findings That Drive TASK-022

- Project Registry 1.1 already owns global cross-project identity, accepted and
  pending reservations, lifecycle, collision blocking, reconciliation, and
  immutable semantic receipts. PostgreSQL must persist and reverify those
  semantics; it must not decide them through SQL constraints.
- Registration can terminate as `Denied` without creating a project authority
  or `ProjectSnapshotId`. A false per-project `StoreScope` or fabricated
  snapshot would violate Contracts and Project Registry ownership. ADR-020's
  typed Registry-specific persistence evidence is therefore a necessary narrow
  amendment to ADR-016, not a generic Store-contract reinterpretation.
- Current command mutation is embedded in `FakeProjectRegistry`. The pure
  planner/checkpoint/replay surface must be extracted only after literal golden
  vectors freeze the actual Registry 1.1 observation, request, authority-
  receipt, and command-result digests. Checkpoint and record-set subjects are
  new Project Registry 1.2 vectors; Registry 1.1 has no historical record-set
  or separately exposed terminal-receipt digest.
- Accepted and pending identity collisions span all projects. One singleton
  global Registry checkpoint/ordinal is the proposed serialization point;
  per-project locks alone cannot close reservation front-running.
- A complete checkpoint must include every first-seen terminal command,
  including denial, blocking, and exact no-project-change observations.
  Otherwise denial-tail or no-change command truncation could remain invisible.
- Retained-state bounds are part of the domain contract: 4,096 current
  projects, 65,536 first-seen terminal commands, 67,108,864 retained snapshot
  bytes, and 131,072 UTF-8 bytes for one already-NFC canonical root. SQL may
  enforce fixed shapes but may not silently truncate or invent compaction.
- The schema-v4 proposal must append exact `0005` while leaving `0001` through
  `0004` byte-identical, preserving historical Store-v2 and Task Ledger receipt
  replay. Forward-profile Store/Task-Ledger functions and Registry functions
  must replace the runtime allowlist without granting direct protected-table
  access.
- Staged Registry rows cannot become authority merely because they exist.
  Finalization must accept only rows created by the current transaction, verify
  their exact pure-plan/checkpoint shape, and publish atomically; committed
  partial staging must fail closed and remain unrepaired.

## Actual Required Execution Order

1. Submit the corrected SPEC-002 v24/AC-36, ADR-020, Project Registry 1.2,
   Postgres Store 1.4, TASK-022, and refreshed audit to a fresh independent
   governance re-review. Resolve every finding and obtain an explicit blocker
   release before code.
2. Only after that release, add the precise Registry 1.1 observation/request/
   authority-receipt/command-result golden fixtures, new 1.2 checkpoint/record-
   set vectors, and the first focused failing pure Registry
   planner/checkpoint/replay test.
3. Implement one verified pure-domain behavior at a time, preserving Fake
   parity and zero I/O.
4. Add the exact schema-v4 manifest, catalog, function, ACL, and migration
   tests before implementing live persistence.
5. Implement the fixed-function `PostgresProjectRegistry` boundary with
   bounded transactions, retry/poison handling, and no raw SQL/client surface.
6. Run focused pure/adapter/Store checks, then one serial marker-owned
   PostgreSQL 17.10 initial/restart matrix.
7. Run full format, strict Clippy, Rust, Node, dependency, RustSec, governance,
   secret/scope/dynamic-SQL, conflict, and whitespace checks.
8. Perform independent code/security and architecture reviews; add a failing
   regression before any review repair.
9. Run local combined integration and compatibility verification on the exact
   reviewed snapshot.
10. Update the workflow ledger and handoff. Merge remains a separate gate and
    is not performed by TASK-022.

## Enforcement Truth

| Control | Strength | Current truth |
|---|---|---|
| Local `npm.cmd run verify` | machine-enforced when invoked | Runs governance/project checks plus 44 retained Node tests; it does not prove Rust/PostgreSQL behavior or automatic invocation |
| Cargo format/Clippy/tests/audit | machine-enforced when invoked | Repeatable local commands exist; TASK-022 has no execution evidence yet |
| Marker-owned PostgreSQL harness | machine-enforced when invoked against its exact disposable target | Existing TASK-019-through-TASK-021 harness is real local evidence; TASK-022 cases do not exist yet and production is excluded |
| Manifest/catalog/ACL tests | machine-enforced when implemented and invoked | Existing profiles fail closed locally; proposed schema v4 is not yet implemented |
| CI workflow | unverified remote control | Defines Node-only verification; no remote/upstream or run evidence, and required-check status is unknown |
| Branch protection, rulesets, required reviews, merge queue | missing or unverified | No remote exists and no service evidence was inspected |
| TDD order, module ownership, allowlist, reviews, protected-action boundaries | documented-only | Required by global/repository rules, plans, spec, ADR, constitutions, and ticket; no hook prevents bypass |
| Git hooks | missing | No repository hook framework or active hook was found |
| Release/rollback process | missing for a release claim | No release/rollback documentation was found; release is outside TASK-022 |
| Production database/credentials/deployment | deliberately absent | Local ticket authority does not extend to these actions |

## Codex Skip Risks And Minimum Missing Controls

- The cumulative dirty tree prevents a clean Git-only TASK-022 attribution.
  The ticket allowlist and frozen artifact hashes must define the review target.
- No hook automatically forces specification, constitution, ticket, TDD,
  independent review, integration, or handoff stages. They can be skipped
  unless the active agent follows the documented workflow and local checks.
- The current CI definition is Node-only. Even a future green remote run would
  not prove Rust or PostgreSQL acceptance without additional jobs and required
  checks.
- The missing router is a broken workflow entry point. The current explicit
  repository/handoff route is sufficient for TASK-022, but the global router
  should be restored or its documented command amended in separate
  control-center work.
- Minimum controls before local TASK-022 completion are: the completed
  independent re-review and explicit governance-blocker release; then test-first
  implementation, current focused/full/live evidence, blocker-free code and
  architecture reviews, combined integration, and a durable ledger/handoff.
- Minimum additional controls before any merge-readiness claim are: an exact
  committed candidate, remote/upstream synchronization, Rust and disposable
  PostgreSQL CI, verified required checks/branch protection/review policy, and
  explicit primary-branch merge authorization.
- Minimum controls before any future release claim additionally include a
  documented and tested release/rollback process plus protected activation
  authority. None is required or authorized in TASK-022.

## Safety And Authorization Boundary

TASK-022 governance and later reversible local implementation may proceed
without a routine human prompt after the governance gate passes. This audit
does not authorize credentials, accounts or payments, non-loopback exposure,
production database/schema mutation, real project filesystem/Git mutation,
security-control changes, permanent deletion, publication, deployment,
protected release activation, or primary-branch merge. The unrelated
companion/playmate website remains outside the project and evidence scope.
