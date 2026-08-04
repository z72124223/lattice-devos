# TASK-014 Independent Code And Security Review

## Target

- Worktree: `lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Specification: SPEC-002 version 10
- Ticket: TASK-014
- Active contracts: Writer Lease 1.0, Contracts 1.4, Policy 2.5
- Reviewers: independent read-only code, security, raw-parser, and architecture
  subagents

## Review RED Findings

Independent review rejected earlier implementations until every accepted
finding had a failing regression:

- restore could accept a rolled-back aggregate and could overwrite current
  state without binding an independently retained checkpoint;
- a heartbeat could move expiry backward, weakening the current lease;
- fake restore did not reject a history containing `RuntimeKind::Live`;
- the first public snapshot boundary wrapped typed values rather than parsing
  a versioned untrusted raw representation;
- `ProcessDeath` evidence did not bind the daemon instance;
- `MarkSuspect` admitted reconciliation-required runtime;
- Policy denied the exact `ReleaseWriter` exception for a suspect lease;
- the Policy owner-composition test did not call `evaluate`;
- a terminal denied-only command receipt could be deleted without changing
  lease state, allowing a previously consumed command ID to be reused after
  restore;
- `WriterLeaseCheckpoint` originally had no validated public constructor for a
  future PostgreSQL adapter.

## Resolutions

- Added a strict, exact-key raw canonical parser for aggregate, request,
  receipt, claim, observation, transition, authority identity, and recovery
  records. Unknown versions, kinds, outcomes, denial combinations, malformed
  numerics/identifiers, duplicates, omissions, and digest substitutions fail
  closed.
- `verify_snapshot` replays every command through the public semantic core and
  compares the complete typed aggregate.
- Applied and denied receipts now share one predecessor digest chain. The
  aggregate separately commits the command high-water mark and tail digest, so
  denial-only tail loss is detected.
- Added validated public `WriterLeaseCheckpoint::new`, complete checkpoint
  export, and `verify_snapshot_against_checkpoint`. Rollback-sensitive restore
  requires an independently retained project/high-water/tail/snapshot
  checkpoint; it is never derived from the snapshot under test.
- Fake restore scans current state, transitions, and all command history for
  fake/live substitution before accepting the verified aggregate.
- Heartbeat now requires strictly later heartbeat and expiry values.
- `ProcessDeath` binds daemon instance, PID, and process-start digest.
- `MarkSuspect` only admits active or draining runtime. Reconciliation remains
  recovery-only.
- Policy permits exact `ReleaseWriter` for an active or suspect current lease
  while ordinary writes still require active authority.
- Policy's cross-crate composition test obtains the current head from the fake
  owner and calls `evaluate`.

## Final Results

Code review: `PASS`, zero remaining P0, P1, P2, or P3 findings.

Security review: `PASS`, zero remaining P0, P1, P2, or P3 findings.

Final focused evidence:

- Writer Lease: 2 unit plus 22 integration tests pass;
- Policy: 81 tests pass;
- full Rust workspace: 180 tests pass;
- strict workspace Clippy with `-D warnings`: pass;
- Rust format and `git diff --check`: pass;
- normal dependency trees and forbidden-I/O scan: pass;
- preserved Node verification: 38 tests pass; final closure project check
  reports 167 files and 14 constitutions.

## Documented Residuals

- Context-free replay proves internal consistency, not historical freshness. A
  coherent old prefix is rejected only when compared with an independently
  retained checkpoint.
- PostgreSQL must atomically persist the aggregate, command receipt chain,
  high-water/tail, and trusted checkpoint while serializing lease transitions.
- Concurrent acquisition, database time, restart, authenticated holder-death,
  stale live connection fencing, and same-transaction mutation authorization
  remain AC-05/Step 6 work.

These are explicit future durable-owner gates, not remaining defects in the
bounded pure/fake AC-28 contract.
