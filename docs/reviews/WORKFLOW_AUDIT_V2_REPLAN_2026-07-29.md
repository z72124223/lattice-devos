# Workflow Audit — V2 Replan — 2026-07-29

## Confirmed Scope

- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos`
- Current branch: `feature/phase1-controlled-swarm`
- Current HEAD observed: `06c3954`
- Dirty state: eight pre-existing modified paths from TASK-004 work.
- Audit mode: read-only until V2 planning artifacts were intentionally added.
- Product direction: general local autonomous AI development platform; no
  unrelated website is in product scope.

## Evidence Collected

- Global and repository `AGENTS.md`.
- `PLANS.md`, charter, source boundary, SPEC-001, ADR-001 through ADR-003,
  seven module constitutions, TASK-001 through TASK-007, branch plan, workflow
  ledger, package/CI commands, Git status, and current diff.
- Original attached `pasted-text.txt` plus later direct user clarifications.
- Official OpenClaw, Codex app-server, Graphify, Hermes, Rust/Cargo, and
  PostgreSQL documentation.
- Current local command/service evidence.

## Workflow Capability Classification

| Capability | Status for V2 | Gate strength | Evidence / gap |
|---|---|---|---|
| Repository instructions | valid | documented-only | repository `AGENTS.md` now routes V2 and preserves mandatory gates |
| Active plan | stale -> replaced in replan | documented-only | old Node/Fake plan contradicted direct direction |
| Behavior specification | blocked | documented-only | SPEC-002 drafted; topology/amendments need approval |
| Module constitutions | blocked | documented-only | V1 active contracts conflict; proposal created |
| V2 tickets | missing by design | missing | cannot ticket a blocked spec |
| Branch/worktree plan | partial | documented-only | V2 preservation plan exists; exact DAG/base and execution await approval |
| Rust implementation | missing | missing | no Cargo workspace or Rust source in repository |
| PostgreSQL persistence | missing | missing | no migrations/schema/roles/connection evidence |
| Fake adapter tests | missing | missing | V1 Fake Runtime tickets are not V2 adapter coverage |
| Live component adapters | missing | missing | OpenClaw/Graphify/Hermes absent from PATH |
| Code review | blocked | documented-only | no V2 implementation diff |
| Architecture review | partial | documented-only | proposed topology reviewed; user decision pending |
| Integration verification | blocked | unverified | no V2 branch/result |
| CI/required checks | partial | unverified | Node CI file exists; remote required status unknown |
| Release/rollback | missing | missing | ADR-007 proposal only |

## Actual Current Execution Order

1. Preserve the dirty Node prototype.
2. Record the direction change.
3. Draft the V2 plan, charter, specification, ADRs, and amendment proposal.
4. Stop for explicit user approval of the Codex-owner topology and versioned
   constitution changes.
5. Only after approval: create constitutions and tickets, then execute the
   existing preservation/branch plan and implement one Rust vertical slice with
   TDD.

The old TASK-005 must not be executed as the current step.

## Local Capability Evidence

| Capability | Observed result |
|---|---|
| Rust compiler | `rustc 1.97.1` |
| Cargo | `cargo 1.97.1` |
| PostgreSQL service | `postgresql-x64-17` running |
| PostgreSQL readiness | `127.0.0.1:5432 - accepting connections` |
| PostgreSQL client | `psql 17.10`, full path found; not on PATH |
| Codex CLI | `codex-cli 0.144.6` |
| OpenClaw | not found on PATH |
| Graphify | not found on PATH |
| Hermes | not found on PATH |
| uv | not found on PATH |

Service readiness does not prove database login, role, schema, extension,
backup, or migration capability. No credential check was attempted.

## Enforcement Truth

- Current V2 rules, plan, specification, ADRs, and amendment proposal are
  documented-only.
- Local Rust/PostgreSQL gates are missing because implementation has not begun.
- Existing Node tests are machine-executable but do not enforce V2 behavior.
- The V2 audit's `npm.cmd run verify` baseline attempt timed out and is not a
  passing result. The dirty Node tree is not newly verified by this replan.
- Remote CI, required checks, branch protection, review requirements, and merge
  authorization are unverified or missing.
- No installation, database mutation, login, payment, publication, push,
  merge, or deployment occurred.

## Minimum Controls Before V2 Implementation

1. User approval of ADR-006 and the V2 module amendment proposal.
2. Accepted versioned constitutions and ready SPEC-002.
3. Dependency-aware V2 tickets with exact allowed paths.
4. A preservation plan for the dirty V1 branch and a non-overlapping V2
   worktree/branch, followed by an approved exact DAG/base.
5. Characterization fixtures for retained V1 behavior.
6. A disposable, least-privilege PostgreSQL test-database gate before schema
   implementation.

## Skip Risks

- Continuing TASK-005 would deepen a superseded Node/File-ledger architecture.
- Editing current constitutions without approval would make governance appear
  compliant after the fact.
- Using both OpenClaw and Rust as writable Codex supervisors would create
  split-brain thread ownership.
- Treating a running PostgreSQL service as ready would hide identity,
  credential, migration, and recovery gaps.
- Installing missing external tools during the planning gate would exceed the
  current authorization.
