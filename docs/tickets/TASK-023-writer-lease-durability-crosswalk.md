---
ticket_id: TASK-023
spec_id: SPEC-002
spec_version: 27
module_id: writer-lease
constitution_version: 1.0
status: blocked
parallel_safe: false
depends_on:
  - TASK-014
  - TASK-038
allowed_paths:
  - docs/tickets/TASK-023-writer-lease-durability-crosswalk.md
  - docs/tickets/red-fixtures/TASK-023-writer-lease-durability-red.json
branch: feature/task-023-025-durable-repositories
base_commit: 845328dcc06d51c7554c93a09739a27ddd827941
---

# TASK-023 Writer Lease durability acceptance crosswalk

## Corrected mapping

TASK-023 is the Step 6 durable Writer Lease slot originally left after
TASK-022. It is **not** permission to build another Writer Lease repository.
TASK-038 has already amended Writer Lease to 1.1 and is implementing the sole
`WriterLeaseRepository` contract, independent `postgres-writer-lease` adapter,
and `db/extensions/writer-lease/v1.sql` profile. TASK-023 is therefore reduced
to a stable-checkpoint acceptance crosswalk and gap-only follow-up.

The semantic owner remains `lattice-writer-lease`. PostgreSQL may serialize
commands, transitions, current authority, fencing high-water and checkpoints;
it may not decide transition legality, allocate a second fence, synthesize a
receipt/head, or become a second lease owner.

## Live evidence observed on 2026-08-10

| Acceptance surface | Status | Evidence and boundary |
|---|---|---|
| Writer Lease 1.0 pure semantics | complete | TASK-014, ADR-012 and Writer Lease constitution 1.0 are present at the exact base |
| Writer Lease 1.1 canonical snapshot/checkpoint bytes | partial | implementation and focused tests exist only as uncommitted TASK-038 changes |
| Sole abstract repository contract | partial | uncommitted TASK-038 adds `WriterLeaseRepository::{execute,current_authority,assert_current}` |
| PostgreSQL repository adapter | partial | untracked TASK-038 `crates/lattice-postgres-writer-lease/` exists; no stable commit is available |
| Exact independent extension | partial | untracked TASK-038 SQL, manifest, installer and verifier exist; no current execution evidence was collected in this light-load pass |
| Concurrent acquire/restart/stale-fence proof | partial | one conditional live test contains these cases, but it was not run and may skip without environment variables |
| Heartbeat/release | partial | release/reacquire and stale heartbeat cases are present; a complete live heartbeat matrix is not proven |
| Suspect/revoke/recovery | missing | the observed PostgreSQL live test does not exercise `MarkSuspect`, holder-death revoke or newer-epoch revoke |
| Ticket/file consistency | missing | TASK-038 names `tests/setup_api.rs`, but that file is absent in the observed dirty tree |
| Stable checkpoint and acceptance | blocked | TASK-038 is dirty at `512732d5b71a5d373363b77bb23a29e4a8ae3b1b`; code presence is not a passed checkpoint |

## Ready gate

This ticket becomes ready only after all of the following are true:

1. TASK-038 publishes a clean local stable commit containing its final Writer
   Lease 1.1 contract, adapter, extension, tests and exact verification result.
2. This worktree is recreated or synchronized from that commit without copying
   the dirty TASK-038 worktree.
3. The crosswalk is rerun against the committed diff and the ticket's missing
   recovery/file-consistency items are either already closed or remain exact
   gap-only work.

If TASK-038 satisfies all TASK-023 acceptance rows, TASK-023 closes as
`complete-by-TASK-038` with evidence only. No duplicate crate, extension,
repository trait, migration profile, lease state machine or fence allocator is
allowed.

## Acceptance criteria after the ready gate

- [ ] The stable TASK-038 commit contains exactly one domain-owned repository
  trait and exactly one PostgreSQL implementation.
- [ ] Canonical snapshot/checkpoint bytes round-trip through the Writer Lease
  public verifier, and the adapter compares an independently retained current
  checkpoint rather than deriving trust from the history being checked.
- [ ] Two concurrent acquires for one vacant project yield exactly one applied
  lease and one durable terminal denial; another project remains independent.
- [ ] Release/reacquire, reconnect, process restart and PostgreSQL restart
  allocate strictly newer non-wrapping positive `BIGINT` fences.
- [ ] Exact retry returns the identical receipt; changed command content,
  stale daemon/epoch/fence/current head and historical self-projection deny.
- [ ] PostgreSQL time drives heartbeat/expiry; expiry only marks suspect.
  Holder-death or strictly newer-leadership evidence is required for revoke.
- [ ] Command, transition, snapshot, checkpoint, current head and fencing
  high-water commit atomically; unknown commit returns no success and converges
  only through a fresh client plus exact retry/reconciliation.
- [ ] Runtime uses only the fixed extension function allowlist, has no direct
  table mutation, and accepts no SQL, path, DSN, credential or caller-created
  authority.
- [ ] Every daemon-authorized mutation boundary that consumes a writer fence
  reasserts the same current lease/daemon/epoch/fence in its transaction.

## RED fixture and first executable RED

The data-only fixture is
`docs/tickets/red-fixtures/TASK-023-writer-lease-durability-red.json`.

The first light readiness RED at base `845328d` is:

```powershell
$required = @(
  'crates/lattice-postgres-writer-lease/src/lib.rs',
  'db/extensions/writer-lease/v1.sql'
)
if ($required.Where({ -not (Test-Path -LiteralPath $_) }).Count -gt 0) { exit 1 }
rg -n 'pub trait WriterLeaseRepository' crates/lattice-writer-lease/src/lib.rs
```

Expected now: non-zero because the clean base predates the TASK-038 contract.
After the stable checkpoint, the first behavioral RED is the first still-
missing row in the crosswalk, starting with live suspect/revoke recovery if it
remains absent. Any focused Cargo check must wait for a heavy-load slot and use
`CARGO_BUILD_JOBS=2`.

## Forbidden work

- No second Writer Lease owner, repository, PostgreSQL extension or migration.
- No edit to TASK-038, Postgres Store, global schema, root Cargo, harness,
  `PLANS.md` or `HANDOFF.md` before the stable checkpoint.
- No full Cargo, PostgreSQL, WSL, service, push, merge, deploy or release action
  in this readiness slice.
