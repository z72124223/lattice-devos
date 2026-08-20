---
ticket_id: TASK-025
spec_id: SPEC-002
spec_version: 37
module_id: artifact-store
constitution_version: 1.1
status: completed
parallel_safe: false
depends_on:
  - TASK-016
  - TASK-024
allowed_paths:
  - docs/tickets/TASK-025-artifact-repository-owned-root-filesystem.md
  - docs/tickets/red-fixtures/TASK-025-artifact-owned-root-red.json
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-025-durable-artifact-metadata-and-owned-root-bytes.md
  - docs/modules/artifact-store/MODULE_CONSTITUTION.md
  - docs/modules/postgres-artifact-store/MODULE_CONSTITUTION.md
  - docs/modules/artifact-owned-root/MODULE_CONSTITUTION.md
  - PLANS.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - HANDOFF.md
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-artifact-store/src/repository.rs
  - crates/lattice-artifact-store/src/aggregate.rs
  - crates/lattice-artifact-store/src/lib.rs
  - crates/lattice-artifact-store/src/snapshot.rs
  - crates/lattice-artifact-store/tests/repository_contract.rs
  - crates/lattice-postgres-artifact-store/Cargo.toml
  - crates/lattice-postgres-artifact-store/src/lib.rs
  - crates/lattice-postgres-artifact-store/src/setup.rs
  - crates/lattice-postgres-artifact-store/src/adapter.rs
  - crates/lattice-postgres-artifact-store/tests/adapter_api.rs
  - crates/lattice-postgres-artifact-store/tests/extension_contract.rs
  - crates/lattice-postgres-artifact-store/tests/postgres_live.rs
  - crates/lattice-artifact-owned-root/Cargo.toml
  - crates/lattice-artifact-owned-root/src/lib.rs
  - crates/lattice-artifact-owned-root/tests/owned_root.rs
  - db/extensions/artifact-store/v1.sql
  - scripts/run-task019-postgres.ps1
  - scripts/test-task025-artifact-durability.ps1
branch: feature/task-023-025-durable-repositories
base_commit: 845328dcc06d51c7554c93a09739a27ddd827941
---

# TASK-025 Artifact repository and disposable owned-root filesystem adapter

## Corrected mapping

TASK-025 closes the remaining Step 6 Artifact durability slice through two
strictly separate mechanisms coordinated by the composition root:

1. PostgreSQL persists authoritative Artifact Store metadata, references,
   quota projections, staging reservations, current heads, delete claims,
   command receipts and independent checkpoints.
2. A disposable owned-root filesystem adapter performs only byte staging,
   digest/length verification, flush, atomic no-clobber publish, verified read,
   exact one-object unlink and orphan quarantine.

`lattice-artifact-store` remains the sole semantic owner of project-scoped
identity, provenance, quota, generation, reference lifecycle, retention,
delete-claim token, unknown outcome and reconciliation. The filesystem is
never truth, a directory scan never grants authority, and no caller path can
select a target.

## Current-state matrix

| Acceptance surface | Status | Evidence and boundary |
|---|---|---|
| Pure identity/provenance/quota/delete semantics | complete | TASK-016, ADR-014 and Artifact Store 1.0 |
| Project/generation isolation and exact delete claim | complete | pure/fake owner covers current-head, token and reconciliation semantics |
| Pure replay/checkpoint | complete | strict snapshot replay and independent checkpoint comparison exist |
| Durable metadata repository contract | complete | bounded canonical snapshots, exact vacant initialization and one-command successor verification |
| PostgreSQL metadata/reference/quota repository | complete | exact PostgreSQL 17.10 catalog/ACL profile, serializable CAS, immutable physical chain, restart replay |
| Disposable owned-root verifier | complete | exact marker/root identities, product-root separation, link/reparse/device/ADS denial |
| Safe staging/publish/read | complete | pinned same-volume temporary file, flush, atomic no-clobber rename, concurrent-winner verification and handle/path rechecks |
| Safe exact unlink/reconciliation | complete | exact claim-bound object isolation, owner-random delete staging, identity recheck, one-file unlink and reconciliation failures |
| Arbitrary/recursive cleanup | blocked by design | explicitly forbidden; absence is not yet a machine-enforced adapter test |
| Stable physical profile convention | complete | TASK-038/TASK-024 gates resolved; exact PostgreSQL 17.10 catalog and ACL signatures are frozen and live-verified |

## Ready gate and dependency order

1. TASK-038 stable checkpoint.
2. TASK-023 crosswalk completion.
3. TASK-024 durable normal-claim boundary completion.
4. Freeze Artifact Store repository bytes/trait and the physical adapter split
   in versioned SPEC/ADR/constitutions.
5. Expand `allowed_paths` only after exact PostgreSQL profile and filesystem
   module names are evidence-backed. This readiness ticket authorizes no such
   implementation path yet.

## Ready gate resolution on 2026-08-20

TASK-023 completed at `78b4e2e` and TASK-024 completed at `13d0194` with its
official PostgreSQL acceptance. SPEC-002 v37, ADR-025, Artifact Store 1.1,
PostgreSQL Artifact Store 1.0 and Artifact Owned Root 1.0 now freeze the
component-free metadata repository, serializable snapshot/checkpoint CAS and
path-free byte-only adapter. The exact paths above replace the readiness-only
allowlist. PostgreSQL and filesystem adapters depend one-way on public pure or
shared contracts and never on each other.

## Acceptance criteria after the ready gate

- [x] Artifact Store exposes bounded canonical repository requests,
  snapshot/checkpoint bytes and one component-free semantic repository contract
  without moving identity, provenance, quota or delete meaning into adapters.
- [x] PostgreSQL atomically persists object/reference/current-head/quota/
  staging/delete-claim/receipt/checkpoint state. It validates only an exact
  owner-produced successor and accepts no caller currentness Boolean. Live
  Registry/effect/daemon/capability-owner rechecks remain a composition-root
  transaction prerequisite and are not claimed by AC-47 or this metadata-only
  repository slice because no durable capability-owner repository exists yet.
- [x] The filesystem adapter accepts only an already verified opaque owned-root
  capability. It accepts no caller-supplied absolute/relative path, glob,
  cleanup root, SQL, credential or product repository handle.
- [x] Root admission binds an owner marker and physical file identity; rejects
  ancestor/descendant overlap with every registered product root, case-fold
  collision, reparse/junction/symlink, hardlink, alternate data stream, device
  and non-regular-file cases.
- [x] Internal paths derive only from project namespace, algorithm, digest and
  generation. User filename/media/schema/provenance fields never become path
  components.
- [x] Staging uses an exclusive owner-controlled name on the same verified
  volume, enforces bounded streaming length/digest, flushes data and directory
  metadata where supported, and publishes by atomic no-clobber rename.
- [x] A concurrent publish loser verifies the winning exact bytes before reuse;
  crash after seal but before metadata creates a quarantined non-authoritative
  orphan that cannot be promoted by scanning.
- [x] Reads recheck handle/file identity and digest. Delete operates only after
  the exact durable delete claim/current head/token and zero-read/reference
  state, then unlinks exactly one verified regular file.
- [x] Unknown filesystem/database outcomes enter
  `RECONCILIATION_REQUIRED`, retain worst-case quota and never imply deletion,
  availability or safe cleanup.
- [x] No recursive deletion API or directory-authority fallback exists. Test
  cleanup may unlink only its enumerated exact adapter-created files, then
  remove already-empty known directories one level at a time after re-verifying
  the owned root identity and containment; recursion is never used.

## RED fixture and first executable RED

The data-only fixture is
`docs/tickets/red-fixtures/TASK-025-artifact-owned-root-red.json`.

The first light readiness RED is:

```powershell
$artifact = Get-Content -LiteralPath 'crates/lattice-artifact-store/src/lib.rs' -Raw
$adapter = Get-ChildItem -LiteralPath 'crates' -Directory | Where-Object {
  $_.Name -match 'artifact.*filesystem|filesystem.*artifact'
}
if ($artifact -notmatch 'pub trait Artifact[^\r\n]*Repository' -or -not $adapter) { exit 1 }
```

Expected now: non-zero because neither durable repository contract nor
filesystem adapter exists. After the ready gate, the first behavioral RED is
the fixture's `UNVERIFIED_ROOT` case: root admission must fail before any file
creation, traversal, rename, unlink or recursive operation. Focused compilation
must wait for a heavy-load slot and use `CARGO_BUILD_JOBS=2`.

## Forbidden work

- No caller-selected path and no arbitrary or recursive cleanup.
- No filesystem identity/provenance/quota/retention/delete authority.
- No directory scan promotion, cross-project deduplication or product-root
  access.
- No schema, Postgres Store, root Cargo, harness, `PLANS.md` or `HANDOFF.md`
  edit before the dependency gates.
- No full Cargo, PostgreSQL, WSL, service, push, merge, deploy, release or
  permanent deletion in this readiness slice.

## Completion evidence on 2026-08-21

- Pure repository vectors reject non-canonical/substituted bytes, non-vacant
  initialization, sibling replacement and multi-command jumps.
- Owned-root tests cover nine Windows cases including junction roots,
  hardlinked/substituted/ADS markers, product overlap, empty/over-limit
  streams, two-thread no-clobber publication, verified read, claim-bound exact
  unlink and quarantine.
- The marker-owned PostgreSQL 17.10 harness proves exact install/no-op catalog
  and ACL profiles, extra-object drift rejection, successor CAS, two-writer
  conflict, transaction guard, snapshot and physical-transition corruption,
  restart replay, closed runtime DML and a twelve-event holder receipt.
- Official acceptance command:
  `powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/test-task025-artifact-durability.ps1`.
- Official acceptance returned
  `TASK025_ARTIFACT_DURABILITY_ACCEPTANCE=PASS`. The 12-event holder receipt is
  `target/task019-holder-receipts/6fb51317e0854149b6c10dbef3b09a68.jsonl`
  with raw SHA-256
  `acc984195098c204e4a40e040eb00da9df6d6c6dd7d58ab8946cce850b659d44`.
