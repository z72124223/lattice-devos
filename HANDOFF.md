# HANDOFF

## Status

`NEEDS_REVIEW`

TASK-008 local bootstrap is complete and verified. Integration remains blocked
because the result is intentionally uncommitted, no remote CI exists, and no
commit/merge authorization was given.

## Objective and scope

- Objective: establish the first buildable Rust project for LATTICE DevOS, a
  general-purpose local AI development platform.
- In scope: approved governance activation, dedicated V2 worktree, inert core
  component manifest, read-only CLI, unexecuted PostgreSQL namespace draft,
  TDD evidence, local verification, independent reviews, and durable handoff.
- Out of scope: installation, login, credentials, database connection or
  mutation, live OpenClaw/Codex/Graphify/Hermes calls, payment, publication,
  push, merge, deployment, and public network exposure.

## Completed work

- Recorded the user's `好 開始執行` reply as approval for ADR-004 through
  ADR-007, the Rust-owned writable Codex topology, the V2 module direction, and
  the named local bootstrap slice.
- Created sibling worktree
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
  on `feature/v2-rust-postgres-bootstrap` from base `06c3954`.
- Preserved the dirty V1 source worktree. Its five named Node code/test WIP
  paths remain modified only there.
- Created SPEC-002 version 2, two active bootstrap constitutions, and completed
  TASK-008.
- Created a dependency-free Cargo workspace with `lattice-core` and
  `lattice-cli`; both packages have `publish = false`.
- Added an inert manifest for Rust Core, OpenClaw, PostgreSQL, Codex, Graphify,
  Hermes, Codebase Memory, and Guardian.
- Added read-only `lattice status`; missing, unknown, and extra arguments fail.
- Added `db/migrations/0001_bootstrap.sql`, which declares only `control`,
  `memory`, and `readmodel`; it was not executed.
- Updated the legacy project checker to ignore Cargo `target/` output.

## Files changed

| Path | Why | Verification |
|---|---|---|
| `Cargo.toml`, `Cargo.lock` | Establish the Rust workspace without third-party packages or publication | Cargo metadata, lint, tests |
| `crates/lattice-core/**` | Own the inert component manifest and SQL artifact test | focused and workspace tests |
| `apps/lattice-cli/**` | Provide read-only local status rendering | focused tests and executable probes |
| `db/migrations/0001_bootstrap.sql` | Record the unexecuted PostgreSQL namespace draft | static contract test |
| `.gitignore`, `scripts/check-project.mjs` | Exclude Cargo build output from Git/project scanning | RED/GREEN invalid-JS probe |
| `README.md` | Show current status and exact local command | manual read plus CLI smoke |
| `PLANS.md`, `AGENTS.md`, `docs/specs/**`, `docs/adr/**`, `docs/modules/**`, `docs/tickets/**`, `docs/plans/**` | Activate approved V2 governance and preserve scope | project check, parity, constitution validator |
| `docs/reviews/**`, `docs/workflow/WORKFLOW_LEDGER.md` | Preserve audit, review, integration, and verification evidence | re-read; project check |

## Workflow ledger

| Stage | Status | Evidence / artifact |
|---|---|---|
| Repository inspection | valid | independent audit; source/target Git evidence |
| Requirements | valid | direct approval record and TASK-008 boundary |
| Specification | valid | SPEC-002 version 2 |
| Module governance | valid | two active constitutions; validator exit 0 |
| Ticket/worktree | valid | TASK-008 and preservation record |
| TDD implementation | valid | four observed RED/GREEN cycles |
| Focused/full verification | valid | Cargo and Node results below |
| Code review | valid | independent P3 fixed; re-review `No findings` |
| Architecture review | valid | governance blocker fixed; re-review no blocker |
| Integration/CI/merge | blocked | uncommitted result; no remote/CI/authorization |

Full stage details are in `docs/workflow/WORKFLOW_LEDGER.md`.

## Verification

- `cargo test -p lattice-core --test platform_manifest`: exit 0, 2 tests.
- `cargo test -p lattice-core --test bootstrap_sql`: exit 0, 1 test.
- `cargo test -p lattice-cli --test status_output`: exit 0, 2 tests.
- `cargo fmt --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  exit 0.
- `cargo test --workspace`: exit 0, 5 tests.
- `cargo run -p lattice-cli -- status`: exit 0; expected inert manifest.
- `lattice status unexpected`: binary exit 2; expected usage rejection.
- `npm.cmd run verify`: exit 0; 38 preserved Node tests.
- module parity: exit 0; 20 specification modules match 20 proposal headings.
- selected V2 constitution validator: exit 0; 2 valid.
- `git diff --check`: exit 0.
- CI: no remote CI result, required check, branch protection, or review policy.
- Runtime: no PostgreSQL connection and no live external adapter was attempted.

## Review and integration

- Code review: independent. Initial P3 found ignored extra CLI arguments; fixed
  with a RED/GREEN regression and executable exit-2 probe. Final result:
  `No findings`.
- Architecture review: independent. Initial module/spec traceability blocker
  was fixed by SPEC-002 v2 and module records. Final result: no blocker; no ADR
  or constitution amendment required.
- Branch/worktree synchronization: source and V2 branches share base
  `06c3954`; V2 changes are an uncommitted local worktree diff.
- Merge status: blocked/not authorized. No commit, push, merge, publication, or
  deployment occurred.

## Risks and open decisions

- `DurableMemory` is a bootstrap label only; PostgreSQL must remain the sole
  durable truth when Codebase Memory is implemented.
- The SQL test is static. A future real migration needs disposable database,
  role, ownership, `search_path`, rollback, and least-privilege evidence.
- OpenClaw, Codex, Graphify, and Hermes remain uninstalled/unpreflighted.
- Any operational CLI command must use approved local IPC; direct core linkage
  is limited to the inert manifest.
- The feature result is not integration-ready until an intentional commit
  grouping, synchronization check, remote policy decision, and user merge
  authorization exist.

## Next action

1. Continue PLANS Step 5 with the first real contracts/ports and fake adapters,
   starting from a new bounded ticket.
2. Keep PostgreSQL and every external component fake/offline until its separate
   execution gate.
3. Ask the user before any commit disposition, live database operation,
   install, credential use, push, merge, publication, or deployment.

## Restart context

- Current worktree:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Current branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954`
- Relevant plan: `PLANS.md`, Step 5.
- Current ticket: `docs/tickets/TASK-008-v2-rust-bootstrap.md` is complete.
- First command: `cargo run -p lattice-cli -- status`.
