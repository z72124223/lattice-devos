# TASK-033 Terminal Delivery Workflow Audit

> Regenerated on 2026-08-21. The historical filename is retained because it is
> the exact path authorized by the existing TASK-033 ticket. No file existed at
> this path in the clean base or in visible Git history.

## Status

`NEEDS_REVIEW`. Non-live checks pass, but the current stable validator rejects
the clean candidate because it predates the mandatory engineering protocol.
Live execution, commit, push, finisher, and dashboard refresh did not start.

## Scope And Identity

- Repository: `github.com/z72124223/lattice-devos`.
- Repair branch: `feature/task-033-terminal-delivery`.
- Source implementation checkpoint:
  `52389375cd7dde552ceec9319120d3659dd7bb2f`.
- Clean terminal-delivery base:
  `fd9561c2f488c30365135ab94b392f212fe68afc`.
- Protected source worktree: `feature/v2-rust-postgres-bootstrap`; its ten
  uncommitted paths were observed read-only and are excluded from this branch.
- TASK-033 is the one existing ticket. No TASK-090 ticket or duplicate TASK-033
  identity was created.

## Capability Audit

| Capability | Status | Evidence | Gate strength |
|---|---|---|---|
| Repository instructions | present | `AGENTS.md` read from `fd9561c` | documented-only |
| Specification and ADR | present | SPEC-002 v27 and ADR-022 | documented plus inspected |
| Ticket and dependency | present | one TASK-033 ticket; unique completed TASK-021 ticket in captured base | validator plus Git inspection |
| Module constitutions | present | Graphify, Codebase Memory, PostgreSQL Memory, Contracts, Ports, Orchestrator, Latticed, Store | documented plus dependency inspection |
| Focused verification | present | both ticket-listed Rust package groups exit 0 | machine-executed locally |
| Full non-live verification | present | format, strict Clippy, locked full Rust tests, and `npm.cmd run verify` exit 0 | machine-executed locally |
| Live acceptance | unverified | coordinated Graphify/PostgreSQL restart/replay gate intentionally not started | blocked on resource coordination |
| Code/architecture review | partial | current read-only review artifacts exist; reviewer independence is not proven | documented self-review |
| Remote/CI/merge | unverified | no repair-branch remote exists yet; primary merge not authorized | unverified |
| Current stable validator | broken for this candidate | TASK-085 `e34bc9b` validator exits 1 because the candidate lacks the engineering protocol and required AGENTS routing | machine-executed fail-closed |

## Actual Execution Order

1. Read repository rules, ticket, SPEC, ADR, constitutions, plans, and handoff.
2. Verify target branch is unoccupied locally and remotely.
3. Use `git branch -m` and `git worktree move`; preserve the dirty source tree.
4. Change only TASK-033 terminal metadata and provenance while status remains
   `in_progress`.
5. Run the ticket-listed non-live checks and inspect the exact implementation,
   base, dependency graph, diff allowlist, and secret scan.
6. Regenerate these four exact review paths with live acceptance still marked
   pending.
7. Coordinate the run-owned live root, non-5432 port, marker, and process
   ownership with the foreman before any live effect.
8. Only after all live and cleanup gates pass, finalize review/handoff, mark the
   ticket complete, commit, and invoke the bounded finisher.

## Non-Live Verification Evidence

| Command | Exit | Result |
|---|---:|---|
| `cargo test -p lattice-contracts -p lattice-ports -p lattice-codebase-memory -p lattice-orchestrator --locked` | 0 | focused contracts/ports/pure memory/orchestration pass |
| `cargo test -p lattice-graphify-adapter -p lattice-postgres-store -p lattice-runtime --locked` | 0 | focused adapter/store/runtime pass; live Graphify cases remain ignored |
| `cargo fmt --all -- --check` | 0 | no formatting drift |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | 0 | strict workspace lint pass |
| `cargo test --workspace --all-targets --all-features --locked` | 0 | full available non-live Rust suite pass; live cases remain ignored |
| `npm.cmd run verify` | 0 | project check and 44 Node tests pass |
| diff/allowlist/secret scan | 0 | current changed path is ticket-allowed and no secret signature matched |

## Skip And Blocker Risks

- File existence alone is not accepted as review evidence; each review records
  the current commands and limits.
- Passing ignored-test suites is not Graphify or PostgreSQL live acceptance.
- The system PostgreSQL listener on 5432 is out of scope.
- TASK-079 may concurrently run a disposable PostgreSQL gate. Live execution
  remains blocked until the foreman confirms the proposed TASK-033 root, port,
  marker, and owned processes do not overlap.

## Exact Contract Blocker

The clean and remotely synchronized current stable governance tool at
`origin/feature/task-085-parallel-task-governance`
`e34bc9bfcf18c71e771f704d50128e1fbeba53ea` was run against this worktree:

```text
docs/contracts/ENGINEERING_PROTOCOL_V1.md: missing engineering protocol.
AGENTS.md: must point to docs/contracts/ENGINEERING_PROTOCOL_V1.md.
AGENTS.md: must require engineering protocol checks before editing and completion.
AGENTS.md: must route completion through delivery:finish and archive the current Codex task only after its success marker.
docs/tickets/TASK-033-graphify-postgres-codebase-memory.md: parallel ticket must be terminal.
```

The terminal-status error is expected while revalidation is incomplete. The
protocol and AGENTS errors cannot be repaired within TASK-033's allowed paths:
neither `AGENTS.md` nor `docs/contracts/**` is authorized. Adding them to the
ticket would be self-authorized scope expansion. No old validator or manual
push may be used to bypass this blocker.
