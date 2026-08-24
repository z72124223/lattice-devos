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
status: completed
parallel_safe: false
depends_on: []
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
bridge API, then let the Store exact-v5 transition invoke one fixed Writer-owned
rebind function inside the same transaction as migration `0007` and let
exact-v6 idempotent retry invoke that same function before catalog/ACL verify.

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

## Commit provenance (not TASK dependencies)

- Resumption base: `aeff4131d0a78e740980b47ab10f56c5aa96cb18`.
- Related inspected predecessor: `e13e6d8ffb0ffeb4ae1eea7e33f535d1848f7d0f`.
- `depends_on: []` is deliberate: these are commit-provenance references, not
  unresolved LATTICE TASK dependencies accepted by the delivery finisher.

## Acceptance criteria

- [x] Writer exposes fixed v3 bridge and rebind administration; exact retry is
      idempotent, changed identity/profile fails closed, and bridge/pending
      exposes zero runtime Writer authority.
- [x] Store distinguishes exact six-row `ExactV5Prefix` from seven-row
      `ExactV6Full` and advances only the exact predecessor.
- [x] Exact-v5 migration `0007`, compatibility publication, and Writer-owned
      v3 rebind commit atomically or roll back together through the fixed SQL
      boundary; exact-v6 retry calls the same idempotent procedure.
- [x] A marker-owned PostgreSQL failure injection holds an active Writer head,
      forces the rebind precondition to fail after ordinal `0007` is staged,
      and proves the exact v5 bridge history, compatibility, Writer identity,
      ledger, and runtime ACL fingerprint remains unchanged.
- [x] Wrong/partial/colliding history, stale lease or fence, inconsistent
      retry, and catalog/ACL drift fail closed with no partial effects.
- [x] Writer v1/v2 and migrations `0001`-`0006` byte/hash invariants pass.
- [x] Focused/all-target/workspace tests, scoped strict Clippy, format,
      repository checks, author reviews, and parent-gated independent
      code/architecture reviews pass. Full-workspace strict Clippy was attempted
      and exited 1 solely on 17 unchanged `lattice-hermes-adapter` diagnostics
      outside this ticket's allowlist; it is not a PASS and is not expanded here.
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
- `docs/reviews/*TASK_094*` records both the author work and the later
  parent-gated independent code/architecture result. Remote CI, delivery run,
  push, merge, and archive remain outside this local completion.
- Earlier candidate command counts in this section are superseded by the
  terminal evidence below; they remain only as a chronological repair record.
- Review repair requires Postgres Store constitution 1.13: the sole mutation
  exception is the fixed Writer-owned rebind call for exact-v5 transition or
  exact-v6 idempotent retry; it grants no Writer state ownership or generic SQL.
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
  Foreman State 1.2. The migration contract statically asserts exactly one
  ordered Writer-owned procedure lock block over all five tables. This is an
  author repair record, not independent approval.

## Terminal local closure — 2026-08-25

- Local status is `completed`; `delivery_archive` remains `keep_open`. The
  completion is limited to this feature branch and is not a remote delivery,
  merge, deployment, release, or archive claim.
- Root workspace evidence: `cargo test --workspace --all-targets --all-features
  --locked` exited 0 at `8753772fb499bc745b4406856192ee5bb9785b03`. Later
  `32d2b109014ac2bc89cf936628a259815fe2112d` changed only the static migration
  contract and passed `migration_contract` 42/42; `f19719c7bf968ce557d84b87d317946f43844bf3`
  is documentation-only. `npm.cmd run verify` exited 0 with Node 120/120 and
  unchanged production bytes.
- Focused evidence: Store all-targets 110 passed, 2 ignored; Writer all-targets
  16/16; runtime composition 1/1; Store and Writer scoped strict Clippy,
  format, repository checks, and diff check pass. Full-workspace strict Clippy
  was attempted and exited 1 solely on 17 unchanged `lattice-hermes-adapter`
  diagnostics outside this allowlist; it is explicitly not a PASS.
- Parent-owned root live receipt (not a raw-log artifact): run
  `691ee93d56794439999db7c424a5588d`, dynamic port `59124`, PostgreSQL PID
  `22684`, exited 0. FRESH_V5, MEMORY_V3, WRITER_V2, WRITER_V3_BRIDGE,
  REBIND_FAILURE_ATOMICITY, and STORE_V6 all passed. Teardown reported
  `root_absent=True`, `listener_survivors=0`; postcheck retained listeners 5432
  PID 5200 and 58743 PID 25912, with no listener on 59124.
- Parent-gated independent code and architecture reviews of exact
  `f19719c7bf968ce557d84b87d317946f43844bf3` report P0=P1=P2=P3=0 and
  feature-delivery clear. The separate product bootstrap ordering of Writer v3
  before Store v6/rebind remains TASK-105's integration prerequisite; this
  ticket makes no deployability claim.
