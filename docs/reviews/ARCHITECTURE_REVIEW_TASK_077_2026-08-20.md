# TASK-077 architecture review — 2026-08-20

## Triggers

- Creates the `engineering-status-dashboard` module.
- Introduces local snapshot schema `lattice.engineering-status/1.0`.
- Adds read-only Git/GitHub/OS process dependencies and a cross-cutting
  post-handoff refresh contract.
- Materially handles privacy, partial failure, and status reliability.

## Before and after

Before TASK-077, task state remained authoritative in existing tickets,
receipts, Git/test/CI evidence, and LATTICE ledgers, but no single
plain-language local projection existed. The Rust control plane and the
`lattice-cli` module intentionally exclude Git/network/process ownership.

After TASK-077, a separate Node standard-library adapter reads registered Git
worktrees, exact ticket frontmatter, live remote heads, and optional GitHub PR/CI
metadata. It owns only disposable `status.json` and self-contained `index.html`
files under local application data. It does not write control-plane state,
repository state, GitHub state, task tickets, or credentials.

## Ownership and contracts

- ADR-001 remains intact: the task ledger/control-plane evidence stays truth;
  the dashboard is explicitly non-authoritative derived evidence.
- ADR-002 remains intact: no approval subject, writer lease, merge authority,
  scope decision, or execution capability is added.
- ADR-006 remains intact: the dashboard is a read-only observer and never owns
  a writable Codex process or worktree lease.
- Snapshot status precedence is fail closed. Explicit task outcomes outrank
  clean Git/passing CI, while missing, conflicting, malformed, stale, or
  unavailable sources remain `UNKNOWN`, partial, or an exact non-success state.
- The v1.0 schema is initial and disposable. Additive changes are compatible;
  removals or reinterpretations require a schema/specification amendment.

## Dependency direction

```text
launcher / npm script
  -> engineering-status-dashboard
       -> Node.js standard library
       -> Git read-only commands
       -> optional gh read-only query
       -> local OS file opener
       -> repository tickets as read-only input
       -> local disposable HTML/JSON output
```

There is no dependency on `lattice-cli`, Rust control-plane crates, PostgreSQL,
MCP, Hermes, LATTICE writers, or third-party JavaScript packages. No reverse
dependency or cycle is introduced. ADR-004's safety-critical Rust boundaries
are therefore not diluted; this module is a presentation adapter outside the
trusted control plane.

## Failure, compatibility, and rollback

- Repository identification or complete-output write failure returns nonzero.
- Per-worktree, live-remote, and optional GitHub failures stay visible and do
  not become success.
- Bounded external processes use argv arrays and timeouts. Live remote heads
  prevent stale tracking refs from being labeled synchronized.
- Output files are staged and installed as a pair; rejected snapshots leave the
  previous output intact. Generated data requires no migration.
- Rollback is removal/revert of this feature and deletion of the disposable
  local output directory. No database, event, credential, deployment, or public
  state must be rolled back.

## Risks and decisions

- Absolute-path redaction is deliberately heuristic. Verified Windows drive,
  forward-slash drive, UNC, `/home`, `/root`, and `/data` forms are covered;
  uncommon mount roots remain a disclosed local-only residual risk.
- Remote and GitHub checks depend on installed tools/network; `UNKNOWN` and
  partial/offline states are the designed degradation path.
- No ADR is required: SPEC-004 and the new module constitution define a bounded
  presentation adapter without revising a control-plane architecture decision.

## Decision

- Confirmed architecture violations: none.
- Constitution conflict or amendment required: none.
- Migration required: none.
- Unapproved architecture decision: none.
- Integration blocker: `PASS`, blocker-free.
