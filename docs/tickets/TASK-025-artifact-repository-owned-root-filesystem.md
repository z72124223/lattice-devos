---
ticket_id: TASK-025
spec_id: SPEC-002
spec_version: 27
module_id: artifact-store
constitution_version: 1.0
status: blocked
parallel_safe: false
depends_on:
  - TASK-016
  - TASK-024
allowed_paths:
  - docs/tickets/TASK-025-artifact-repository-owned-root-filesystem.md
  - docs/tickets/red-fixtures/TASK-025-artifact-owned-root-red.json
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
| Durable metadata repository contract | missing | no Artifact repository trait/error boundary exists at base `845328d` |
| PostgreSQL metadata/reference/quota repository | missing | no concrete durable Artifact adapter/profile exists |
| Disposable owned-root verifier | missing | no adapter proves marker plus physical root identity and product-root separation |
| Safe staging/publish/read | missing | no live same-volume exclusive staging, flush, no-clobber rename or verified read evidence |
| Safe exact unlink/reconciliation | missing | no live link/ADS/device/non-regular/TOCTOU checks or exact one-file unlink evidence |
| Arbitrary/recursive cleanup | blocked by design | explicitly forbidden; absence is not yet a machine-enforced adapter test |
| Stable physical profile convention | blocked | wait for TASK-038 stable checkpoint and TASK-024 closure before schema/Cargo/harness choices |

## Ready gate and dependency order

1. TASK-038 stable checkpoint.
2. TASK-023 crosswalk completion.
3. TASK-024 durable normal-claim boundary completion.
4. Freeze Artifact Store repository bytes/trait and the physical adapter split
   in versioned SPEC/ADR/constitutions.
5. Expand `allowed_paths` only after exact PostgreSQL profile and filesystem
   module names are evidence-backed. This readiness ticket authorizes no such
   implementation path yet.

## Acceptance criteria after the ready gate

- [ ] Artifact Store exposes bounded canonical repository requests,
  snapshot/checkpoint bytes and one component-free semantic repository contract
  without moving identity, provenance, quota or delete meaning into adapters.
- [ ] PostgreSQL atomically persists object/reference/current-head/quota/
  staging/delete-claim/receipt/checkpoint state and rechecks Registry, effect,
  daemon/admission, capability and owner currentness in the same transaction.
- [ ] The filesystem adapter accepts only an already verified opaque owned-root
  capability. It accepts no caller-supplied absolute/relative path, glob,
  cleanup root, SQL, credential or product repository handle.
- [ ] Root admission binds an owner marker and physical file identity; rejects
  ancestor/descendant overlap with every registered product root, case-fold
  collision, reparse/junction/symlink, hardlink, alternate data stream, device
  and non-regular-file cases.
- [ ] Internal paths derive only from project namespace, algorithm, digest and
  generation. User filename/media/schema/provenance fields never become path
  components.
- [ ] Staging uses an exclusive owner-controlled name on the same verified
  volume, enforces bounded streaming length/digest, flushes data and directory
  metadata where supported, and publishes by atomic no-clobber rename.
- [ ] A concurrent publish loser verifies the winning exact bytes before reuse;
  crash after seal but before metadata creates a quarantined non-authoritative
  orphan that cannot be promoted by scanning.
- [ ] Reads recheck handle/file identity and digest. Delete operates only after
  the exact durable delete claim/current head/token and zero-read/reference
  state, then unlinks exactly one verified regular file.
- [ ] Unknown filesystem/database outcomes enter
  `RECONCILIATION_REQUIRED`, retain worst-case quota and never imply deletion,
  availability or safe cleanup.
- [ ] No recursive deletion API or directory-authority fallback exists. Test
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
