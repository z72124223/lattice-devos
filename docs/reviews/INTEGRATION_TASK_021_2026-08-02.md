# TASK-021 Integration Report

## Decision

- Local combined integration（本機整合）: `PASS`。
- Repository merge readiness（儲存庫合併就緒）: `BLOCKED`。
- Merge performed（已執行合併）: `NO`。

The final frozen TASK-021 result passes the available local combined checks and
independent code/security and architecture reviews with P0=P1=P2=P3=0. Merge
readiness remains blocked because the cumulative result is uncommitted, no
remote/upstream or Rust/PostgreSQL CI exists, branch protection is unverified,
and primary-branch merge authorization was not requested or granted.

## Identity And Git State

- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Committed primary/feature ahead-behind: `0/4` (feature is four ahead).
- Remote/upstream: absent.
- TASK-021 has no attributable commit; MVP-0 through TASK-021 remain one
  inspectable cumulative dirty worktree.
- `git diff --check`: exit 0; unmerged paths and conflict markers: zero.
- No reset, clean, branch switch, commit, push, merge, release, publication,
  deployment, production database, credential, or protected action occurred.

## Scope Repair And Compatibility

The final review P2 is closed across the complete compatibility unit:

- `task_ledger_read_head_v1(bytea,text,text)` receives expected project and
  snapshot identity and rejects a mismatched retained stream, duplicate
  physical rows, or a same-owner/aggregate wrong-scope physical row with
  `LCR01`, including when no Ledger stream row exists.
- Rust passes the complete expected scope on every head read.
- Catalog identity, function ACL, source/config verifier, and migration
  contract all use the new fixed signature.
- The direct live regression inserts a vacant Store-only wrong-scope orphan,
  requires public `load_stream` to return `RetainedRowCorrupt`, and proves the
  row counts remain `[1,0,0,0,0,0]` without repair or domain-row creation.

TASK-020 compatibility is preserved:

| Migration | Bytes | SHA-256 |
|---|---:|---|
| `0001_bootstrap.sql` | 312 | `7bff021fc17f738551309c906578c8015b2dd0307d27d239c21df1697c4d09c8` |
| `0002_control_store_foundation.sql` | 14,259 | `e996dc64af3112a647e75ebf07df2a77b1e9b3a018ed443880150365184883f0` |
| `0003_live_control_store.sql` | 29,518 | `00ae3eedd76704f26b1df58955d9d594c98f0ba525be93b15d8c9ebb1f2115c1` |
| `0004_task_ledger_repository.sql` | 111,742 | `cd658ed2f4624cd0a829c818c1cf96d8ac3829264046976bdde3b2fc7feea6e5` |

The final four-entry manifest is
`09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407`.
The frozen Store-v2 receipt profile remains
`4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129`.

## Dependency And Combined Verification

- Locked Cargo metadata reports 14 workspace members.
- Dependency direction remains one-way
  `lattice-postgres-store -> lattice-task-ledger`; Task Ledger has no adapter
  dependency and Postgres Store has no reverse workspace consumer.
- Approved direct Postgres Store dependencies are cjson, Contracts, Ports,
  Task Ledger, exact `postgres = 0.19.14`, `serde_json`, and exact
  `sha2 = 0.11.0`; duplicate dependency output is empty.

| Check | Status | Evidence |
|---|---:|---|
| PostgreSQL 17.10 disposable harness | PASS | actual latest frozen initial + restart run; self-test and harness PASS on a non-5432 loopback endpoint |
| Rust workspace | PASS | 432 tests across 52 test binaries |
| Postgres Store package | PASS | 57 tests; environment-gated marker test is not substituted for the live harness |
| Node governance/preserved suite | PASS | `check=ok`; 44/44 tests |
| Format and strict Clippy | PASS | workspace/all-targets, zero warnings |
| RustSec audit | PASS | 109 `Cargo.lock` dependencies checked against 1,178 advisories; zero known vulnerability |
| Migration/catalog contract | PASS | 15/15 static tests plus actual PostgreSQL 17 fresh apply and catalog verification |
| Git/PowerShell hygiene | PASS | no diff error, unmerged path, conflict marker, or PowerShell parse error |
| Code/security review | PASS | initial findings repaired; final P0-P3 all zero |
| Architecture review | PASS | ownership, scope, transaction, dependency, migration, receipt-profile, and deferral boundaries pass |

## Runtime And Cleanup Evidence

- No disposable harness PostgreSQL, Cargo, `postgres_live`, or test process
  remained after the final run. The installed PostgreSQL 17 Windows service
  was not replaced or stopped by this task.
- Two stopped diagnostic roots remain under ignored `target/` paths:
  `target/task019-postgres/be9400ccb8504058bc87cf06f2eae309` and
  `target/task021-postgres-syntax/probe-20260802-xmin`. Both have no
  `postmaster.pid`, are not reparse points, and have no listener/process.
- Exact recursive cleanup was attempted earlier only after path/stopped-state
  verification, but local policy blocked the command before execution. It was
  not bypassed. These roots are a disclosed hygiene note, not live state or a
  functional integration blocker.

## Enforcement Truth And Merge

| Gate | Classification | Evidence or gap |
|---|---|---|
| Local tests, lint, audit, migrations | machine-enforced for this run | actual exit-0 commands on the frozen files |
| Disposable PostgreSQL behavior | machine-enforced locally | actual PostgreSQL 17.10 initial/restart run; production excluded |
| Module/dependency/scope boundaries | locally scanned and independently reviewed | no remote policy enforces them |
| Ticket allowlist | documented plus local evidence | no clean per-ticket commit exists |
| Remote Rust/PostgreSQL CI | missing | tracked workflow does not run these gates |
| Remote synchronization/branch protection | missing or unverified | no remote/upstream exists |
| Committed merge candidate | missing | cumulative worktree remains dirty |
| Primary merge authorization | absent | continued local implementation is authorized; primary merge is separate |

Local TASK-021 integration is complete. Repository merge readiness is
`BLOCKED`, merge was not performed, and continued reversible local MVP-1 work
may proceed to TASK-022 governance.
