---
spec_id: SPEC-007
title: Live fault and recovery acceptance
version: 1
status: approved
approved_by: direct_user_goal_mode_authorization
approved_at_local: 2026-08-23
---

# SPEC-007 — Live fault and recovery acceptance

## Problem

LATTICE has deterministic and PostgreSQL-backed component tests, but the
current machine has not proved one continuous chain across process loss,
PostgreSQL loss, restart, reconciliation, and safe resume.  TASK-051 records
an attempted live gate as `FAIL`; it is evidence of a gap, not completion.

The existing disposable PostgreSQL harness is PowerShell-only.  The user has
forbidden that execution path, so a new acceptance runner must be a bounded
Rust executable/test and must preserve its evidence when cleanup cannot be
proved.

## Authority boundary

- PostgreSQL plus the owning domain verifier is the only durable task truth.
- SQLite, local dashboard files, Codex task/thread state, child-process output,
  and in-memory caches may be indexes or observations only.  They cannot
  resolve a conflicting task state or mint a terminal receipt.
- A fresh process must reconstruct its public task projection from PostgreSQL;
  it must not re-run an effect merely because local state is absent.
- If PostgreSQL, a receipt, a lease/fence, or an effect counter cannot be
  verified, the outcome is `RECONCILIATION_REQUIRED` or a bounded failure,
  never `COMPLETED`.

## Required behavior

1. Provide one Windows Rust-only opt-in live acceptance entry that accepts no
   caller-selected database, shell, credential, repository, or task input.
   It may use only a fresh marker-owned loopback PostgreSQL 17.10 cluster.
2. Before each phase, record redacted identities for the runner, PostgreSQL
   executable, run root, port, database system identifier, and source binary.
   A source/binary mismatch or unknown owner stops the run before mutation.
3. The first vertical slice proves: durable controlled-task admission; fresh
   process status replay; physical PostgreSQL stop; restart of the same owned
   cluster; fresh-process status replay with identical task semantics and
   durable digests; and a zero-duplicate-effect counter.
4. It must then add one fault at a time: PostgreSQL disconnect/unknown outcome,
   duplicate submit, duplicate callback/receipt, child/Codex timeout, and
   parent/child process interruption.  Every fault has an expected terminal or
   reconciliation state and an independently measured no-duplicate effect
   counter.
5. Tests may simulate a Codex child only through the existing fixed controlled
   canary adapter.  They must not invoke a general shell, use a user account,
   mutate a real Codex configuration, or send external work.
6. The runner creates all resources below one nonce-marked temporary root.  It
   stops or removes resources only after matching marker, executable identity,
   data root, listener, and process evidence.  Failed stop proof preserves the
   root and returns a blocker.
7. Secrets are generated in memory or a runner-owned temporary password file,
   never printed, committed, included in receipts, or copied to environment
   evidence.  The runner must not change installed PostgreSQL, Windows
   services, global configuration, accounts, credentials, network policy, or
   security settings.

## Acceptance criteria

1. An opt-in current-machine run produces a redacted receipt for the restart
   slice, including initial and restarted postmaster identities, database system
   identifier, task/result/head digests, fresh-client proof, and effect count.
2. Forced PostgreSQL loss and retry either reproduce the retained exact result
   without a second effect or stop in an explicit reconciliation state.
3. Duplicate submit/callback and controlled timeout/interruption probes have
   deterministic terminal/reconciliation results and zero duplicate effects.
4. The same acceptance run proves that a stale SQLite/index observation cannot
   override PostgreSQL task state.  If no SQLite implementation participates,
   the receipt records `INDEX_NOT_PRESENT` rather than fabricating a comparison.
5. Normal unit tests remain hermetic; the live runner is ignored/opt-in and
   cannot run against an arbitrary PostgreSQL target.
6. Rust formatting, focused unit tests, focused live-runner safety tests, and
   the actual opted-in slice pass.  Any unproven required scenario is reported
   as a blocker and prevents an overall verified claim.

## Non-goals

- New MCP task capabilities, autonomous scheduling, arbitrary Codex execution,
  deployment, release, GitHub changes, account/credential operations, SQLite
  as a second authority, or automatic repair of unknown durable data.
- Treating a previous test, a visible dashboard, or a process exit code as live
  fault-recovery acceptance.
