---
ticket_id: TASK-094
title: Writer Lease v3 apply/rebind and Store schema-v6 transition repair
spec_id: SPEC-002
spec_version: 38
module_id: postgres-writer-lease
constitution_version: 1.3
additional_modules:
  - module_id: postgres-store
    constitution_version: 1.13
status: in_progress
parallel_safe: false
depends_on:
  - commit:aeff4131d0a78e740980b47ab10f56c5aa96cb18
  - commit:e13e6d8ffb0ffeb4ae1eea7e33f535d1848f7d0f
branch: feature/task-094-writer-v3-apply-rebind
implementation_worktree: lattice-worktrees/task-094-writer-v3-apply-rebind
implementation_base: aeff4131d0a78e740980b47ab10f56c5aa96cb18
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
allowed_paths:
  - PLANS.md
  - HANDOFF.md
  - crates/lattice-postgres-writer-lease/src/lib.rs
  - crates/lattice-postgres-writer-lease/src/setup.rs
  - crates/lattice-postgres-writer-lease/tests/extension_contract.rs
  - crates/lattice-postgres-writer-lease/tests/postgres_live.rs
  - crates/lattice-postgres-store/src/postgres_setup.rs
  - crates/lattice-postgres-store/src/migrations.rs
  - crates/lattice-postgres-store/Cargo.toml
  - Cargo.lock
  - crates/lattice-postgres-store/tests/migration_contract.rs
  - crates/lattice-postgres-store/tests/schema_v6_profile.rs
  - crates/lattice-postgres-store/tests/postgres_project_registry.rs
  - crates/lattice-postgres-store/tests/postgres_live.rs
  - apps/lattice-runtime/tests/task094_writer_v3_transition.rs
  - db/extensions/writer-lease/v3.sql
  - db/extensions/writer-lease/v3-rebind.sql
  - db/migrations/0007_foreman_coordination.sql
  - scripts/test-task094-writer-v3-transition.ps1
  - docs/adr/ADR-026-writer-owned-schema-v6-rebind.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/modules/postgres-writer-lease/MODULE_CONSTITUTION.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-094-writer-v3-apply-rebind.md
  - docs/reviews/WORKFLOW_LEDGER_TASK_094_2026-08-21.md
  - docs/reviews/CODE_REVIEW_TASK_094_2026-08-21.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_094_2026-08-21.md
  - tools/engineering-status-dashboard/branch-guide.zh-TW.json
---

# TASK-094 - Writer Lease v3 apply/rebind and Store schema-v6 transition repair

## Objective

Close TASK-079's production migration blocker without moving Writer-owned
extension state into Store. Add the append-only Writer v3 administrative
bridge API, then let the Store v5-to-v6 runner invoke only one fixed
Writer-owned rebind function inside the same transaction as migration `0007`.

## Frozen scope and identities

- Base is exact clean TASK-079 commit
  `aeff4131d0a78e740980b47ab10f56c5aa96cb18`.
- Writer v1/v2 SQL and Store migrations `0001` through `0006` remain
  byte-identical.
- The only successor is Writer schema 3 plus global schema 6, Memory schema 3,
  ordinal 7 `0007_foreman_coordination`, stream `FOREMAN_COORDINATION`, and
  event `FOREMAN_SNAPSHOT_RECORDED`.
- TASK-079 event/epistemic semantics, TASK-050 dirty worktree, TASK-051,
  TASK-033, and TASK-078 exporter are out of scope.

## Acceptance criteria

- [x] Writer exposes fixed v3 bridge and rebind administration; exact retry is
      idempotent, changed identity/profile fails closed, and bridge/pending
      exposes zero runtime Writer authority.
- [x] Store distinguishes exact six-row `ExactV5Prefix` from seven-row
      `ExactV6Full` and advances only the exact predecessor.
- [x] Migration `0007`, compatibility publication, and Writer-owned v3 rebind
      commit atomically or roll back together through the fixed SQL boundary.
- [x] A marker-owned PostgreSQL failure injection holds an active Writer head,
      forces the rebind precondition to fail after ordinal `0007` is staged,
      and proves the exact v5 bridge history, compatibility, Writer identity,
      ledger, and runtime ACL fingerprint remains unchanged.
- [x] Wrong/partial/colliding history, stale lease or fence, inconsistent
      retry, and catalog/ACL drift fail closed with no partial effects.
- [x] Writer v1/v2 and migrations `0001`-`0006` byte/hash invariants pass.
- [ ] Focused/all-target/workspace tests, strict Clippy, format, repository
      checks, and author-recorded code/architecture review pass; independent
      merge review remains a separate parent-owned gate.
- [x] Any live PostgreSQL claim uses a marker-owned disposable cluster on a
      dynamic non-5432 loopback port with explicit PID ownership preflight;
      otherwise live remains honestly `NOT_RUN`.

## Authority boundary

Local implementation, exact dependency use, focused disposable PostgreSQL
acceptance, one logical commit, and non-force push of this feature branch are
authorized. Default-branch merge, cherry-pick into TASK-079, deployment,
release, force push, credential change, public exposure, TASK-051 reuse, and
task archival are not authorized.

## Current local evidence

- Exact v6 manifest: `4a004488543ce39266ec046607a938958da51567fe747cb22f2e731f30b36ed7`;
  repaired `0007` bytes: 217177, SHA-256
  `21de6f201996a71ec048f0c7976b0802180182e8be5e613147daefd735baf52e`.
- The implementation was already present in this worktree when the bounded
  repair resumed. No historical RED result is claimed or reconstructed. Its
  local behavior is instead re-verified from this tree before the ticket can
  advance.
- The current live PostgreSQL run, if recorded here, must name its fresh
  marker-owned root, dynamic non-5432 port, exact focused command, and teardown
  result. A previous receipt is not evidence for this candidate.
- `docs/reviews/*TASK_094*` records the author review boundary. A parent-owned
  independent review, remote CI, delivery run, push, merge, and archive are not
  completed by this ticket checkpoint.
- Current local commands: Writer Lease all-targets `16/16`; Store migration
  contract `41/41`; schema-v6 profile `5/5`; Store all-targets `109 passed,
  2 ignored`; focused live `1/1`. Scoped strict Clippy, format, repository
  check, and diff check pass. Workspace strict Clippy remains blocked by 17
  pre-existing `lattice-hermes-adapter` diagnostics outside this ticket's
  allowlist. The workspace test process completed, but the runner did not
  return a terminal exit receipt to this worker; do not treat it as a verified
  PASS until the parent reruns it.
- Review repair requires Postgres Store constitution 1.13: the sole mutation
  exception is the fixed Writer-owned rebind call inside the exact v5-to-v6
  migration transaction; it grants no Writer state ownership or generic SQL.
- Review-repair live receipt: run `bace41835c794136b99e8e1312108236`, port
  `57281`, first observed the intentional rebind SQLSTATE `55000` rollback and
  exact-v5 bridge fingerprint, then completed v5-to-v6. Teardown proved
  `root_absent=True` and `listener_survivors=0`.
- Parent foreman independent live command receipt (not a raw log artifact):
  run `8125d6fe95264766b7b06161caa16a05` used marker root
  `C:\Users\f7212\AppData\Local\Temp\lattice-task094-pg-8125d6fe95264766b7b06161caa16a05`,
  dynamic port `55198`, and PostgreSQL PID `21176`. From the TASK-094 root it
  ran `& '.\scripts\test-task094-writer-v3-transition.ps1' -RunId
  '8125d6fe95264766b7b06161caa16a05' -Port 55198 -RepositoryRoot
  (Get-Location).Path`; the port came from a loopback `TcpListener` port-0
  allocation/release and excluded 5432/58743. Exit was 0; FRESH_V5, MEMORY_V3,
  WRITER_V2, WRITER_V3_BRIDGE, REBIND_FAILURE_ATOMICITY, and STORE_V6 passed.
  Teardown reported `root_absent=True` and `listener_survivors=0`; the parent
  then observed the marker root absent while 5432 PID 5200 and 58743 PID 25912
  remained listening.

## 2026-08-25 boundary-repair evidence

- The prior Store-owned TASK-094 live phase is superseded and must not be
  reused as current evidence. The equivalent cross-adapter fixture now lives
  only at `apps/lattice-runtime/tests/task094_writer_v3_transition.rs`, where
  the composition root legally depends on both adapters; Store has no Writer
  adapter dependency, Writer semantic-row parser, direct Writer DML, or direct
  Writer table lock.
- Fresh local live receipt: run `fb5817a389794a5a8e637bfff9288a61`, marker
  root `C:\Users\f7212\AppData\Local\Temp\lattice-task094-pg-fb5817a389794a5a8e637bfff9288a61`,
  dynamic port `58375`, PID `2760`, exit `0`. It passed FRESH_V5, MEMORY_V3,
  WRITER_V2, WRITER_V3_BRIDGE, REBIND_FAILURE_ATOMICITY, and STORE_V6. The
  active-head call asserted SQLSTATE `55000`; runner rollback kept history,
  compatibility, Writer identity/ledger and runtime ACL fingerprints exact at
  the v5 bridge. Identity drift reaches the Writer transaction; ledger and ACL
  drift fail earlier at Store catalog closure; all leave the measured state
  unchanged. Exact-v6 retry calls the same procedure and remains idempotent.
  Teardown reported `root_absent=True`, `listener_survivors=0`.
- This is a local candidate only. Product runtime currently orders Store
  migration before Writer-v3 bridge/bootstrap; a separate product-based,
  governed integration task must explicitly compose Writer-v3 bridge before
  Store v6/rebind. TASK-094 neither changes that production composition nor
  claims deployment readiness.

## 2026-08-25 architecture-review follow-up

- Local architecture review found and this follow-up corrects a documentation
  consistency gap: both Store and Writer constitutions now expressly authorize
  the same fixed Writer-owned procedure for exact-v5 transition and exact-v6
  idempotent retry only. SPEC-002 frontmatter now uses canonical
  `foreman-state` 1.2 and its Module Impact table names Task Ledger 2.4 and
  Foreman State 1.2. The migration contract statically asserts one ordered
  Writer-owned procedure lock block over all five tables. This is an author
  repair record, not independent approval.
