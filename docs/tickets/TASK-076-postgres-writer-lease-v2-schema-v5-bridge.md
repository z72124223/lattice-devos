---
ticket_id: TASK-076
title: PostgreSQL Writer Lease v2 schema-v5 compatibility bridge
spec_id: SPEC-002
spec_version: 33
related_spec_id: SPEC-003
related_spec_version: 5
module_id: postgres-writer-lease
constitution_version: 1.1
status: in_progress
parallel_safe: false
depends_on:
  - commit:a3599c18d9462732c3b82c9e7d302980657eeccc
  - commit:8b6d5171759e28339d6fe2b66fab0c3b6718c64c
  - TASK-038
branch: feature/task-076-postgres-writer-lease-v2
implementation_worktree: lattice-worktrees/task-076-postgres-writer-lease-v2
implementation_base: 8b6d5171759e28339d6fe2b66fab0c3b6718c64c
allowed_paths:
  - crates/lattice-postgres-writer-lease/src/lib.rs
  - crates/lattice-postgres-writer-lease/src/setup.rs
  - crates/lattice-postgres-writer-lease/src/adapter.rs
  - crates/lattice-postgres-writer-lease/tests/adapter_api.rs
  - crates/lattice-postgres-writer-lease/tests/extension_contract.rs
  - crates/lattice-postgres-writer-lease/tests/postgres_live.rs
  - crates/lattice-postgres-store/src/postgres_setup.rs
  - crates/lattice-postgres-store/tests/migration_contract.rs
  - crates/lattice-postgres-store/tests/postgres_live.rs
  - crates/lattice-postgres-store/tests/postgres_setup_api.rs
  - crates/lattice-postgres-store/tests/postgres_task_ledger.rs
  - crates/lattice-postgres-codebase-memory/src/setup.rs
  - crates/lattice-postgres-codebase-memory/tests/extension_contract.rs
  - crates/lattice-postgres-codebase-memory/tests/postgres_live.rs
  - crates/lattice-postgres-codebase-memory/tests/setup_api.rs
  - db/extensions/writer-lease/v2.sql
  - scripts/run-task019-postgres.ps1
  - scripts/test-task075-schema-v5-migration-reconciliation.ps1
  - scripts/test-task076-postgres-writer-lease-v2.ps1
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/specs/SPEC-003-chatgpt-mcp-gateway.md
  - docs/adr/ADR-022-exact-graphify-postgres-codebase-memory.md
  - docs/adr/ADR-023-bounded-mcp-task-dispatch-and-postgres-writer-lease.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/modules/postgres-codebase-memory/MODULE_CONSTITUTION.md
  - docs/modules/postgres-writer-lease/MODULE_CONSTITUTION.md
  - docs/tickets/TASK-050-autonomy-receipt-ledger-replay.md
  - docs/tickets/TASK-075-schema-v5-registry-autonomy-migration-reconciliation.md
  - docs/tickets/TASK-076-postgres-writer-lease-v2-schema-v5-bridge.md
  - PLANS.md
  - HANDOFF.md
---

# TASK-076 - PostgreSQL Writer Lease v2 schema-v5 compatibility bridge

## Authority And Objective

Preserve the accepted PostgreSQL Writer Lease v1 history while making the
previously valid global-v3 + Codebase-Memory-v2 + Writer-Lease-v1 profile
upgradeable to the TASK-075 global-v5 + Memory-v3 platform. The bridge must
retain One Gateway, One Truth, and One Writer: the Writer Lease adapter remains
the only owner of Writer Lease installation, identity advancement, lease rows,
fencing, replay, and runtime admission.

TASK-075 commit `a3599c18d9462732c3b82c9e7d302980657eeccc`
is the immutable implementation prerequisite. This ticket does not reopen its
Registry/autonomy/Memory semantics, does not mark TASK-050 or TASK-051
complete, and does not authorize merge, deployment, public networking,
credential mutation, or an external provider/model call.

TASK-076 implementation begins after the separately committed byte-preservation
prerequisite `8b6d5171759e28339d6fe2b66fab0c3b6718c64c`. That prerequisite marks
`db/migrations/0006_task_autonomy_receipt.sql` as binary so Git retains its
already accepted CRLF bytes and freezes its raw SHA-256 as
`c50f2a51380950b5f6c757b736b35b550d903319a46d2bcd9319938e02106a61`.
Neither that file nor `.gitattributes` is a TASK-076 implementation path; both
must remain unchanged from this implementation base.

## GitHub Checkpoint Publication Authorization - 2026-08-14

The user explicitly authorized this ticket's current progress to be committed,
pushed without force to `feature/task-076-postgres-writer-lease-v2`, and
published as a Draft pull request. The project-scoped standing authorization in
`PLANS.md` permits later clean checkpoint pushes to the same bounded feature
branch without another routine approval. It does not authorize merge, promotion
from Draft, force-push, tag/release, deployment, protected-branch mutation, or
repository/account/credential changes.

GitHub CLI authorization was verified for host `github.com`, account
`z72124223`, repository `z72124223/lattice-devos`, HTTPS transport, and an
operating-system-keyring credential with `repo`, `read:org`, and `gist` scopes.
Only this non-secret metadata may be retained. OAuth tokens, one-time device
codes, passwords, browser/mobile sessions, and confirmation values are excluded
from every repository artifact and publication record.

## Frozen History

- `db/extensions/writer-lease/v1.sql` remains byte-identical, including its
  path, SHA-256, manifest commitment, catalog, and seven v1 functions.
- Existing Writer Lease commands, transitions, aggregate snapshots,
  checkpoints, terminal receipts, current-authority receipts, repository
  request bytes, fencing high-water, and lease-revision high-water remain
  byte-identical and are never dropped, renumbered, rewritten, or rehashed.
- Pure `writer-lease` stays at 1.1. Its planner, replay, exact-retry,
  currentness, recovery, and fencing semantics do not change.
- `writer_lease_assert_current_v1` retains its exact 15-scalar signature and
  same-transaction Task Ledger fencing contract.
- Global migrations `0001` through `0006` (including the frozen `0006` raw
  SHA-256 above), Codebase Memory v1/v2/v3 SQL, and their retained receipts
  remain immutable from the TASK-076 implementation base.

## Closed Bridge State Machine

Only the following ordered states are recognized:

1. `G3_M2_W1_CURRENT`: exact global schema v3, exact Memory v2, and exact
   Writer Lease v1. Runtime remains admitted under the existing profile.
2. `G3_M2_W2_BRIDGE`: the Writer owner has verified v1, required no current
   `ACTIVE` or `SUSPECT` authority, preserved every semantic row and
   high-water, updated the extension identity to schema v2, and appended
   ledger ordinal 2 for the bridge. Runtime is quarantined because the v1
   bind/load functions no longer match the current identity and the v2
   successors admit only the final global-v5/Memory-v3 profile.
3. `G5_M2_W2_BRIDGE_PENDING`: the Store owner has re-verified the exact bridge
   under the common locks and advanced only the global manifest to schema v5.
   All runtime constructors reject this migration-only state.
4. `G5_M3_W2_BRIDGE_PENDING`: the Memory owner has re-verified the exact
   Writer bridge before and after its own v2-to-v3 transaction and advanced
   only Memory. All runtime constructors still reject this state.
5. `G5_M3_W2_CURRENT`: the Writer owner has re-verified the complete bridge,
   rebound the same v2 identity to global-v5/Memory-v3, appended exact ledger
   ordinal 3 event `REBOUND`, and reopened the exact current v2 runtime
   profile.

Fresh global-v5/Memory-v3 installation must converge on the exact same final
Writer v2 catalog, current identity, ACL, and runtime surface, but records one
truthful fresh-current ordinal 1 `INSTALLED` row rather than fabricating an
upgrade history. The verifier accepts only that one-row fresh history or the
exact three-row v1/bridge/rebind history. Partial, unknown, reordered, cross-
profile, extra, drifted, or substituted states are not bridge states and fail
closed without repair.

## Ownership And Locking

Every participating administrative runner takes transaction-scoped advisory
locks in this exact order before installed-profile classification or DDL:

1. global Store migration lock `0x4c41_5454_4943_4501`;
2. Codebase Memory extension lock `0x4c41_5443_4d45_4d31`;
3. Writer Lease extension lock `0x4c41_5457_4c45_4131`.

The Writer owner performs the v1-to-v2 bridge and final activation. The Store
runner may recognize the exact bridge/current companion profile only to govern
its global migration. The Memory runner may recognize the exact v2 bridge only
inside its migration transaction; while holding all three advisory locks it
also takes `SHARE` locks on the five Writer Lease tables, performs complete
catalog/owner/ACL/identity/ledger verification before and after Memory DDL,
and never plans, mutates, replays, or interprets lease semantics. The
recognizers are fixed versioned compatibility exceptions, not adapter-to-
adapter dependencies or generic extension discovery.

## Writer Lease V2 Physical Contract

- Add only `db/extensions/writer-lease/v2.sql` as the append-only v1-to-v2
  successor. The v1 file is not copied, edited, or renamed.
- Remove only the single-entry ledger constraints needed for multiple exact
  ordinals; keep the identity foreign key and the single current identity row.
- In an upgrade history, ledger ordinal 1 is the exact historical v1
  `INSTALLED` profile, ordinal 2 is the exact v2 bridge `UPGRADED` profile,
  and ordinal 3 is the exact v2 final `REBOUND` profile. A fresh current
  install has only ordinal 1 `INSTALLED` with the final v2/global-v5/Memory-v3
  identity. Every row binds database, global, Memory, extension SQL, and
  extension manifest identities.
- Add only `writer_lease_bind_runtime_v2` and
  `writer_lease_load_for_update_v2`, because the v1 variants hard-bind ledger
  ordinal 1. The other five v1 functions remain the current implementations.
- Revoke runtime EXECUTE only from the superseded v1 bind/load-for-update
  functions and grant it to the two v2 successors. The exact current runtime
  allowlist stays seven functions; the catalog retains nine functions.
- `PostgresWriterLease` switches only those two fixed calls and verifies the
  exact current v2 identity. No new public CRUD, SQL, credential, row, JSON,
  schema selector, or lease-semantic API is introduced.

## Failure And Recovery

- A v1-to-v2 transition requires replay-verified durable state and no current
  `ACTIVE` or `SUSPECT` lease. Released non-empty history is accepted and must
  replay byte-identically. Unknown or ambiguous state fails before mutation.
- Each step is one bounded serializable transaction. Failure before commit
  leaves the exact prior state. Commit-unknown returns the existing ambiguous
  setup class and is reconciled only by exact read-only profile verification.
- Both pending states are deliberate fail-closed recovery points: rerunning
  the owning next step is allowed; runtime, Task Submit, and fenced writes are
  not.
- Every v2 bridge or pending profile has no runtime schema usage and zero
  runtime function execution grants. Only exact W1 current or final W2 current
  is executable.
- A wrong owner, ACL, function body, constraint, index, identity, ledger row,
  manifest, database identity, global profile, Memory profile, receipt,
  checkpoint, high-water, or current-authority substitution is rejected.

## Acceptance Criteria

- [ ] V1 SQL bytes/hash/manifest and all existing semantic golden vectors are
      unchanged; v2 SQL/hash/manifest and the nine-function catalog are exact.
- [ ] Fresh global-v5/Memory-v3 install and exact v1 upgrade converge on one
      `G5_M3_W2_CURRENT` profile; exact current apply is a verified no-op.
- [ ] A non-empty released v1 database upgrades through all five states with
      commands, transitions, receipts, snapshots, checkpoints, fence high-
      water, and lease-revision high-water byte-identical after process and
      PostgreSQL restart.
- [ ] `ACTIVE`, `SUSPECT`, partial, extra, drifted, cross-profile, wrong-
      identity, and wrong-ledger inputs fail before mutation.
- [ ] Store and Memory take global -> Memory -> Writer locks, recognize only
      the exact bridge/current profiles, and keep both pending profiles closed
      to every runtime role.
- [ ] Memory verifies the full Writer v2 bridge before and after its own DDL
      while holding Writer table `SHARE` locks, without changing Writer rows.
- [ ] The old bind/load functions are ungranted, the two v2 successors are
      granted, the other five v1 functions remain granted, and direct table
      access remains denied.
- [ ] Concurrent/repeated runners converge without deadlock, duplicate ledger
      ordinals, fence reuse, or partial success; injected rollback and commit-
      unknown matrices remain recoverable and fail closed.
- [ ] The existing same-transaction Task Ledger current-authority assertion
      and TASK-050 fresh-process replay pass unchanged on the final profile.
- [ ] The dedicated marker-owned disposable PostgreSQL acceptance proves
      initial/restart cleanup, exact receipt closure, zero impact on existing
      listeners, and no external credentials, model, network, or publication.
- [ ] Focused tests, affected crate suites, strict Clippy, format, repository
      checks, diff check, code review, architecture review, and integration
      review report no unresolved P0/P1 findings.

## Verification Commands

```powershell
cargo test -p lattice-postgres-writer-lease --all-targets --locked
cargo test -p lattice-postgres-store --test migration_contract --locked
cargo test -p lattice-postgres-codebase-memory --all-targets --locked
cargo clippy -p lattice-postgres-writer-lease -p lattice-postgres-store -p lattice-postgres-codebase-memory --all-targets --locked -- -D warnings
cargo fmt --all -- --check
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-task076-postgres-writer-lease-v2.ps1 -SelfTestOnly
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-task076-postgres-writer-lease-v2.ps1
npm.cmd run check
git diff --check
```

The live command is authorized only for one marker-owned disposable loopback
PostgreSQL fixture at a dynamic port. It must exclude and never touch existing
listeners at ports `5432`, `64272`, and `55432`.

## Non-Goals

- No pure Writer Lease semantic change, new lease transition, fence reset,
  current-authority shortcut, generic migration coordinator, or second truth.
- No rewrite of global migrations, Memory SQL, historical Writer v1 SQL, or
  retained receipt bytes.
- No public MCP/tool/schema change, model/provider call, Git merge/force-push,
  deployment, public listener, credential/account mutation, or release. Only
  the explicitly authorized bounded feature-branch checkpoint publication is
  permitted.
- No TASK-050, TASK-051, TASK-052, TASK-053, Hermes, Registry, autonomy, or
  Codebase Memory product-feature expansion beyond the exact compatibility
  recognition required by this bridge.
