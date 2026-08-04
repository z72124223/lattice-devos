# TASK-020 Integration Report

## Decision

- Local combined integration（本機合併結果）: `PASS`。
- Repository merge readiness（儲存庫合併就緒）: `BLOCKED`。
- Merge performed（已執行合併）: `NO`。

TASK-020 的凍結工作樹通過目前所有可用的本機驗證、獨立 code/security
review（程式碼／安全審查）與 architecture review（架構審查），沒有剩餘 P0、
P1、P2 或 P3 finding（問題），也沒有本機整合衝突。`BLOCKED` 只表示目前沒有
可合併的 commit（提交）、remote/upstream（遠端／上游）、Rust/PostgreSQL 遠端
CI（持續整合）、branch protection（分支保護）或 primary-branch merge
authorization（主分支合併授權）；不阻擋繼續可逆的本機 MVP-1 工作。

## Identity

- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Feature branch: `feature/v2-rust-postgres-bootstrap`
- Feature HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Feature commit: none for TASK-020; MVP-0 through TASK-020 remain one
  inspectable, uncommitted local result.
- Source-preservation branch: `feature/phase1-controlled-swarm` at the same
  committed HEAD.
- Primary target if later authorized: `main` at
  `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Verification worktree: the dedicated V2 worktree above; no temporary merge
  branch/worktree was created because the candidate is uncommitted and the
  preservation plan forbids unsafe branch/worktree mutation.

## Synchronization And Conflict State

- `main...HEAD` committed ahead/behind is `0/4`: the feature HEAD is four
  commits ahead of `main` and is not behind it.
- `feature/phase1-controlled-swarm...HEAD` is `0/0` at the committed level.
  The two worktrees intentionally hold different uncommitted contents.
- Remote/upstream: absent. A fetch, remote divergence check, CI service lookup,
  ruleset check, and merge-queue check are therefore unavailable.
- `git diff --name-only --diff-filter=U`: empty.
- Full conflict-marker scan over the TASK-020 allowlist: empty.
- `git diff --check`: exit 0 for tracked changes. Because most V2 files are
  untracked, a separate allowlist-wide trailing-whitespace scan was also empty.
- Future textual/behavioral conflict against a real committed target remains
  `unverified`; the current dirty result cannot be represented by
  `merge-tree` or a clean integration commit without first changing Git state.

## Scope And Allowed Paths

TASK-020 declares 34 exact `allowed_paths` and `parallel_safe: false`. At the
integration snapshot, 33 allowed paths exist as changed/report paths after this
report is added; the unused optional
`crates/lattice-postgres-store/tests/postgres_control_store.rs` does not exist.
All observed TASK-020 product, test, governance, review, and harness paths are
inside the ticket allowlist.

The complete worktree also contains 189 changed paths outside the TASK-020
allowlist. They are the documented cumulative MVP-0-through-TASK-019/V1 dirty
baseline, not a clean TASK-020 delta. A timestamp-based observation found zero
out-of-allowlist paths newer than the TASK-020 workflow audit, but timestamps
are not immutable provenance. Therefore:

- ticket scope and allowlist: `documented-only` plus local scan;
- exact per-ticket Git attribution: `unverified`;
- automatic path enforcement at commit/CI: `missing`.

No reset, clean, checkout, stash, commit, merge, push, deploy, production
database mutation, credential change, remote connection, provider/product
operation, or companion/playmate website work occurred.

## Cargo Dependency Boundaries

`cargo metadata --locked --offline --format-version 1 --no-deps` and
`cargo tree` show these direct edges:

| Crate | Direct dependencies | Result |
|---|---|---|
| `lattice-contracts` | none | pass |
| `lattice-ports` | `lattice-contracts` only | pass |
| `lattice-postgres-store` | `lattice-cjson`, `lattice-contracts`, `lattice-ports`, exact `postgres = 0.19.14` with default features off, exact `sha2 = 0.11.0` | pass |

`cargo tree -i lattice-postgres-store --workspace --locked` reports no reverse
workspace consumer. `cargo tree --duplicates --locked` reports nothing. The
approved synchronous `postgres` crate has `tokio-postgres` transitively, but
there is no forbidden direct `tokio-postgres`, domain, policy, gateway,
provider, product, OpenClaw, Graphify, Hermes, Git, or website dependency edge.

## TASK-019 And Next-Slice Compatibility

- TASK-019 is `completed`; TASK-020 depends on it and preserves its exact
  migrations:
  - `0001_bootstrap.sql`: 312 bytes,
    SHA-256 `7bff021fc17f738551309c906578c8015b2dd0307d27d239c21df1697c4d09c8`.
  - `0002_control_store_foundation.sql`: 14,259 bytes,
    SHA-256 `e996dc64af3112a647e75ebf07df2a77b1e9b3a018ed443880150365184883f0`.
- The only expansion is `0003_live_control_store.sql`: 29,518 bytes,
  SHA-256 `00ae3eedd76704f26b1df58955d9d594c98f0ba525be93b15d8c9ebb1f2115c1`.
- The PostgreSQL 17.10 harness proves fresh v2, exact empty-v1-prefix upgrade,
  rejected non-empty/partial/edited/reordered/unknown v1 states, rollback,
  concurrency, restart, permission, overflow, retry, and unknown-commit
  reconciliation behavior. It does not claim production compatibility.
- Full Rust tests preserve the prior fake and module behaviors while adding the
  physical live Store. TASK-020 closes only AC-34; it does not close
  AC-03/04/05/19 or MVP-1.
- SPEC-002 assigns domain repositories to TASK-021 through TASK-024, but no
  `TASK-021` ticket exists yet. The next implementation slice is therefore
  only directionally compatible, not ready: its spec/ticket/constitution
  impact and allowlist must be created before code changes.

## Combined-Result Verification

The implementation owner froze the result and supplied the full execution
evidence. This integration review independently reran the non-conflicting
format, dependency, Git, governance, Node, hash, and scope checks; per task
instruction it did not rerun the complete PostgreSQL harness.

| Check | Command or service | Exit/status | Evidence |
|---|---|---:|---|
| Rust format | `cargo fmt --all -- --check` | 0 | independently rerun |
| Full Rust workspace | `cargo test --workspace --all-targets --all-features --locked` | 0 | 409/409 on frozen result; independently confirmed by code review |
| Strict Clippy | workspace/all-targets, `-D warnings` | 0 | zero warnings on frozen result |
| Project governance | `npm.cmd run check` | 0 | post-report snapshot: 257 files, 18 constitutions, 20 tickets, one current task |
| Preserved Node tests | `npm.cmd test` | 0 | independently rerun, 44/44 |
| Disposable PostgreSQL | `scripts/run-task019-postgres.ps1` | PASS | PostgreSQL 17.10 initial + restart, including TASK-020 live/fault/upgrade matrices and cleanup |
| Dependency graph | `cargo metadata`, `cargo tree`, reverse tree, duplicates | 0 | approved direct edges, no reverse consumer, no duplicate package versions |
| Dependency audit | `cargo audit` 0.22.2 | 0 | 109 dependencies checked against 1,178 advisories; zero known vulnerability |
| Migration preservation | SHA-256 and length scan | 0 | exact hashes and lengths above; manifest matches |
| Git hygiene | `git diff --check`, unmerged and marker scans | 0 | no tracked whitespace error, unmerged path, or allowlist conflict marker |
| Code/security review | `CODE_REVIEW_TASK_020_2026-08-02.md` | PASS | P0=0, P1=0, P2=0, P3=0 |
| Architecture review | `ARCHITECTURE_REVIEW_TASK_020_2026-08-02.md` | PASS | earlier constitution wording conflict repaired and reverified; no remaining blocker |

## Enforcement Truth

| Gate | Classification | Evidence / gap |
|---|---|---|
| Local format/tests/lint/audit | `machine-enforced` for this run | actual exit-0 commands on frozen files |
| Disposable PostgreSQL behavior | `machine-enforced` for the marker-owned target | actual PostgreSQL 17.10 initial/restart harness; production remains outside scope |
| Cargo direct dependency graph | `machine-enforced` locally | locked metadata/tree plus forbidden-edge scan |
| Migration byte identity | `machine-enforced` locally | SHA-256/length constants and tests |
| Code and architecture review | `documented-only` independent evidence | both reports PASS; no remote required-review rule enforces them |
| Ticket allowlist/module ownership | `documented-only` | governance documents and local scan exist; no per-ticket commit or CI path gate |
| GitHub workflow | `documented-only` static configuration | tracked `.github/workflows/ci.yml` runs Node `npm run verify` only |
| Rust/PostgreSQL remote CI | `missing` | no job in the tracked workflow |
| Remote CI run/upstream synchronization | `unverified` | no remote/upstream exists |
| Branch protection/ruleset/merge queue | `missing` or `unverified` | no service evidence is available |
| Committed TASK-020 candidate | `missing` | the exact result is still cumulative and uncommitted |
| Future target conflict/combined CI | `unverified` | cannot test an uncommitted candidate against a remote target |
| Primary-branch merge authorization | `missing` for this operation | user authorization covers continued bounded local work, not a TASK-020 primary merge |

## Reviews And Policy

- Code/security review: `PASS`, P0-P3 all zero.
- Architecture review: `PASS`; one P1 governance wording inconsistency was
  corrected within the allowed constitution path and reverified, leaving no
  blocker or unapproved architecture decision.
- Required local human approval: none for reversible TASK-020 code/tests and
  the marker-owned disposable cluster.
- Required primary merge approval: still explicit and absent. TASK-020 also
  names commit/push/merge as a non-goal.
- Remote required review, Rust/PostgreSQL CI, branch protection, and merge
  queue: missing/unverified.

## Merge And Rollback

- Status under the integration skill: `BLOCKED` for merge readiness.
- Authorization source: the user's MVP-3 execution directive authorizes this
  bounded local slice; it does not authorize a primary-branch merge.
- Merge performed: no.
- No production or installed-service database state was changed, so there is
  no production rollback action.
- Do not use reset/clean/checkout to undo this work. Any later rollback must
  first create an attributable committed candidate or another reversible,
  reviewed preservation artifact and then revert the exact TASK-020 contract,
  port, Store, migration, test, and governance compatibility unit together.

Local TASK-020 integration is complete. Ticket/ledger/`HANDOFF.md` closure and
creation of the next bounded ticket remain the owning workflow's next actions.
