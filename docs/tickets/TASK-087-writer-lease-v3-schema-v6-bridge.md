---
ticket_id: TASK-087
title: Writer Lease v3 global-schema-v6 compatibility bridge
spec_id: SPEC-002
spec_version: 36
module_id: postgres-writer-lease
constitution_version: 1.2
status: completed
parallel_safe: false
depends_on: []
evidence_references:
  - commit:65f2902504e5ef5acba6f258b736905fd4d12a4d
  - evidence:23a552e
  - evidence:92d93b1
branch: feature/task-087-writer-lease-v3-schema-bridge
implementation_worktree: lattice-worktrees/task-087-writer-lease-v3-schema-bridge
implementation_base: 65f2902504e5ef5acba6f258b736905fd4d12a4d
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
allowed_paths:
  - crates/lattice-postgres-writer-lease/src/lib.rs
  - crates/lattice-postgres-writer-lease/src/setup.rs
  - crates/lattice-postgres-writer-lease/tests/extension_contract.rs
  - crates/lattice-postgres-store/src/lib.rs
  - crates/lattice-postgres-store/src/migrations.rs
  - crates/lattice-postgres-store/src/schema_v6_profile.rs
  - crates/lattice-postgres-store/tests/migration_contract.rs
  - crates/lattice-postgres-store/tests/schema_v6_profile.rs
  - db/extensions/writer-lease/v3.sql
  - docs/adr/ADR-025-writer-lease-v3-global-schema-v6-compatibility.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/modules/postgres-writer-lease/MODULE_CONSTITUTION.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-087-writer-lease-v3-schema-v6-bridge.md
  - docs/reviews/WORKFLOW_LEDGER_TASK_087_2026-08-21.md
  - docs/reviews/CODE_REVIEW_TASK_087_2026-08-21.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_087_2026-08-21.md
  - PLANS.md
  - HANDOFF.md
---

# TASK-087 - Writer Lease v3 global-schema-v6 compatibility bridge

## Objective

Create the append-only Writer Lease v3 compatibility contract and Store
catalog/ACL profile required before TASK-079 may add global schema v6. Preserve
accepted v2/global-v5 behavior and keep the future foreman append inside the
existing Task Ledger transaction and fixed Writer Lease fencing assertion.

## Frozen predecessor and successor identity

- Base is clean/synchronized TASK-076 commit `65f2902504e5ef5acba6f258b736905fd4d12a4d`.
- Writer v1/v2 SQL, migrations `0001`-`0006`, schema-v5 manifest, Task Ledger
  2.3 events/hashes, and `writer_lease_assert_current_v1` stay byte-identical.
- Writer v2 continues to recognize only its existing global-schema 3 and 5
  profiles; no v2 predicate is widened to schema 6.
- Writer v3 reserves only global schema 6 whose immediate predecessor is exact
  schema v5 and whose successor descriptor is ordinal `7`, ID
  `0007_foreman_coordination`, path
  `db/migrations/0007_foreman_coordination.sql`, stream
  `FOREMAN_COORDINATION`, and event `FOREMAN_SNAPSHOT_RECORDED`.
- TASK-087 does not add migration `0007`, the foreman event, Ledger rows, a
  Port, or a foreman table. TASK-079 remains the semantic/physical event owner.

## Closed compatibility states

1. `G5_M3_W2_CURRENT`: accepted TASK-076 current runtime profile.
2. `G5_M3_W3_BRIDGE`: Writer-owned append-only v3 bridge; runtime has zero
   Writer schema usage and zero Writer function EXECUTE grants.
3. `G6_M3_W3_BRIDGE_PENDING`: future Store migration advanced only through
   exact approved `0007`; runtime remains closed.
4. `G6_M3_W3_CURRENT`: Writer re-verifies the complete global catalog/ACL,
   stream/event identity and v3 extension ledger, then rebinds and opens the
   same seven-function Writer runtime surface.

Fresh schema-v6 installation may converge directly on current v3 with one
truthful `INSTALLED` ledger row. Upgrade history retains v1/v2 and v3 bridge/
rebind rows in exact ordinal order. Unknown, skipped, duplicated, reordered,
cross-generation, partial, or substituted histories fail closed.

## Acceptance criteria

- [x] v1/v2 extension and `0001`-`0006` migration bytes/hashes are unchanged.
- [x] offline tests prove v2 accepts only frozen schema 3/5 states and v3
      accepts only the exact schema-v6/ordinal-7/stream/event contract.
- [x] absent `0007`, wrong ordinal/ID/path, unknown generation, wrong predecessor
      manifest, missing stream/event, missing runtime ACL, direct table ACL, or
      catalog drift fails closed.
- [x] v3 bridge/pending has zero runtime Writer authority; only exact v2-current
      and v3-current profiles expose the seven governed functions.
- [x] stale lease, wrong fence and cross-generation replay remain rejected by
      unchanged same-transaction fencing and retained Writer semantics.
- [x] migration ordering, idempotency, rollback-safe transitions and fresh/
      upgrade ledger shapes are deterministic and tested.
- [x] focused/affected tests, format, strict Clippy, repository checks, security
      review and architecture review pass.

## Verification

```powershell
cargo test -p lattice-postgres-writer-lease --all-targets --locked
cargo test -p lattice-postgres-store --test migration_contract --locked
cargo test -p lattice-postgres-store --test schema_v6_profile --locked
cargo clippy -p lattice-postgres-writer-lease -p lattice-postgres-store --all-targets --locked -- -D warnings
cargo fmt --all -- --check
npm.cmd run check
git diff --check
```

The live PostgreSQL gate runs only after current-machine evidence proves there
is no other global writer or heavyweight database acceptance. Otherwise it is
`NOT_RUN`, never inferred PASS.

TASK-087 recorded `NOT_RUN`: TASK-079's migration `0007` is intentionally
absent, the engineering dashboard was partial, and PostgreSQL processes were
already present. A schema-v6 runtime result therefore cannot yet be truthful.

## Authority boundary

Local implementation, focused verification, one logical commit and non-force
push to this feature branch are authorized. No TASK-079 merge, TASK-050/051/078
modification, primary-branch merge, deployment, release, public exposure,
credential change, force push, or task archival is authorized.
