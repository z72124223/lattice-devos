---
module_id: postgres-writer-lease
name: PostgreSQL Writer Lease Repository
version: 2.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-26
---

## Mission

Persist and independently replay Writer Lease 1.1 commands, transitions,
current authority, monotonic fencing high-water, and checkpoints in PostgreSQL
through one exact extension and repository implementation, without acquiring
lease semantic ownership.
Version 1.1 preserves the v1 extension and semantic history while adding the
Writer-owned v2 schema-v5 bridge and exact current v2 runtime profile.
Version 1.2 adds an append-only v3 compatibility bridge for the exact future
global-schema-v6 foreman-coordination profile without widening v2 or changing
Writer Lease 1.1 semantic/fencing bytes.
Version 1.3 makes that bridge administrative and explicit: the Writer-owned
setup API may install/replay exact v3 bridge state and may rebind only through
the fixed zero-argument `writer_lease_rebind_v3` procedure. Postgres Store may
call that procedure only inside the same transaction as its exact-v5 transition
or exact-v6 idempotent retry; it cannot construct, parse, persist, or
reinterpret Writer state.
Version 1.4 adds the narrower product-bootstrap operation for an already-
present schema-v6 Writer v3 profile. It rejects an absent profile before
durable mutation, rebinds only bridge-pending, and verifies current as a no-op.
The same amendment re-pins the undeployed fixed rebind bytes to the corrected
TASK-105 schema-v6 manifest and changes no Writer semantic row shape.
Version 1.5 exposes an explicit current-profile runtime constructor that binds
only the Writer-v3 procedures after verified schema-v6 bootstrap. The
historical Writer-v2 constructor and procedure profile remain unchanged.
Version 1.6 re-pins the still-undeployed v3 rebind boundary to Store 1.17's
corrected schema-v6 manifest. It changes no Writer procedure or semantic row.
Version 1.7 re-pins that boundary to Store 1.18's PostgreSQL-valid, equivalently
bounded foreman child constraint. Writer behavior remains unchanged.
Version 1.8 exposes one read-only, Writer-owned closed bootstrap profile. It
fully verifies v2 predecessor, v3 bridge, v6 bridge-pending, or v6 current
state plus the fixed rebind boundary and replay-safe history before composition
may stop Runtime admission. It adds no durable state or Writer semantics.
Version 2.0 preserves the deployed/frozen v3 schema-v6 bytes and adds the
append-only v4 successor for exact Store schema v7. It exposes the closed
`V6V4Bridge` / `V7V4Current` bootstrap states, the fixed
`writer_lease_rebind_v4` boundary, and a version-closed `new_v4_v7` runtime
constructor without relabelling v3 or changing Writer Lease 1.1 semantics.

## Non-Goals

- Decide lease transition legality, identity, expiry, recovery, fencing, exact
  retry, snapshot, or checkpoint meaning; Writer Lease owns those semantics.
- Own Task Spec, Task Ledger, policy, orchestration, workspace, Codex, Git,
  verification, Gateway, rate policy, or task status.
- Add or modify a global Store migration, install itself during normal daemon
  startup, or make Postgres Store its persistence owner.
- Accept arbitrary SQL, table/function names, credentials, paths, actor data,
  lease state fragments, or caller-constructed receipts.
- Treat expiry alone, a receipt projected from itself, process memory, or an
  unverified row set as current writer authority.

## Owned Data

- The exact `db/extensions/writer-lease/v1.sql` and append-only
  `db/extensions/writer-lease/v2.sql`, `v3.sql`, `v4.sql`, and corresponding
  fixed rebind bytes, extension identities, checksums, explicit administrative
  state transitions, and read-only verifiers. Every predecessor file is
  immutable once the successor is introduced.
- Physical PostgreSQL tables, indexes, constraints, fixed functions, roles,
  ownership, ACLs, transactions, and bounded retry/poison state used only for
  Writer Lease persistence.
- Immutable physical command/transition rows, the current aggregate row,
  independently retained checkpoint, and monotonic fencing high-water value.
- The complete current lease identity projection used by same-transaction
  fenced writers; a copied digest/fence cannot authorize another Task binding.
- Adapter-level database/schema/extension persistence evidence.

Writer Lease 1.1 owns every semantic request, terminal receipt, authority
receipt/head, transition, snapshot, checkpoint, and recovery decision.

## Public Contracts

- Implement the Writer Lease 1.1 repository trait for exact command execution,
  replay-verified current-authority observation, and durable current-head
  assertion.
- Inspect one existing project in a repeatable-read read-only transaction and
  return `None` only when no durable head/history exists. A returned project
  summary means snapshot, independent checkpoint, current projection, and every
  physical command/transition row were replayed byte-exactly; inspection never
  creates a head, allocates a fence, or appends a command.
- Consume one caller-supplied already-authenticated PostgreSQL client and one
  exact verified target; read no environment, DSN, password, or credential
  source.
- Delegate all construction, transition, replay, exact retry, and checkpoint
  verification to Writer Lease 1.1.
- Commit the semantic command/receipt, optional transition, aggregate snapshot,
  independently retained checkpoint, and fencing high-water atomically in one
  bounded serializable transaction.
- Classify exact retry before mutable admission/currentness checks. Changed
  content under one command ID is denied without revealing the retained
  receipt.
- Install or verify only the exact extension manifest through explicit
  administrative entrypoints. Normal runtime never auto-installs or repairs it.
- Expose the typed v3 and v4 bridge/apply/rebind administrative operations only
  to the Writer-owned adapter. Store may invoke only the exact fixed rebind
  procedure selected by the governed migration generation inside the same
  transaction; that call is not a Writer repository API and grants no generic
  Writer mutation authority.
- Expose the strict existing-v3 rebind/verify operation used by product
  bootstrap. It cannot take the fresh-install branch; absence is a typed
  fail-closed result and leaves the schema-v6 database fingerprint unchanged.
- Expose one repeatable-read, read-only bootstrap profile that returns only
  `V5FallbackRequired`, `V5Bridge`, `V6BridgePending`, or `V6Current` after
  exact Writer-owned verification, plus `V6V4Bridge` and `V7V4Current` for the
  append-only v4 successor. Partial/corrupt Writer evidence never maps to
  fallback, and the session apply gate creates no durable row or ACL change.
- Construct schema-v6 runtime repositories only through the explicit v3
  constructor and exact `bind_runtime_v3`/`load_for_update_v3` procedures.
  The v2 constructor remains version-closed to its historical profile.
- Construct schema-v7 runtime repositories only through the explicit
  `new_v4_v7` constructor and exact `bind_runtime_v4`/
  `load_for_update_v4` procedures. The frozen v3 constructor remains
  version-closed to schema v6, so neither generation can be silently
  relabelled as the other.
- Use fixed function calls only; expose no generic CRUD, arbitrary row, SQL,
  schema/table name, raw client, migration, or credential API.

## Invariants

1. PostgreSQL contains at most one current `ACTIVE` or `SUSPECT` Writer Lease
   aggregate per project.
2. Fencing tokens are positive signed-BIGINT-safe, increase monotonically, and
   are never reset, rolled back, wrapped, or reused across release, reconnect,
   process restart, or database restart.
3. The current authority row, fencing high-water, command/transition rows,
   aggregate snapshot, and independent checkpoint advance atomically or not at
   all.
4. The adapter cannot construct or reinterpret a lease transition, receipt,
   authority head, snapshot, or checkpoint outside Writer Lease 1.1 public
   APIs.
5. An untrusted row set becomes usable only after context-free replay, an
   independent checkpoint/current-head match, and byte-exact comparison of
   every physical command and transition row against domain canonical bytes.
6. Runtime database identity, exact extension checksum, role, ownership, ACL,
   relation, column/default/tombstone, constraint, index, type, function body/
   result/configuration, and all-principal schema/table/function/column closure
   are verified before repository use.
7. The runtime role has no direct table mutation and may execute only the exact
   fixed Writer Lease function allowlist.
8. Unknown commit outcome never reports success or denial; exact retry or
   reconciliation is required.
9. Expiry is an observation, not proof of holder death. Revoke requires the
   exact Writer Lease 1.1 recovery evidence.
10. No secret, DSN, password, raw SQL, process output, or arbitrary diagnostic
    is persisted in semantic rows, receipts, errors, or `Debug`.
11. The extension does not alter global migrations, Store physical receipts,
    Task Ledger rows, Codebase Memory rows, or another module's ACLs.
12. A Fake Writer Lease, synthetic authority, or caller-supplied fencing token
    cannot be represented as live repository evidence.
13. Released history and absent history are not interchangeable. Terminal task
    acceptance may require an existing replayed project with no current
    authority and exact bounded high-water values.
14. A v3 or v4 bridge/pending state has zero runtime execute authority. Only the
    verified exact schema-v6/v3-current or schema-v7/v4-current profile restores
    its generation's runtime Writer functions; rebind is idempotent for that
    exact identity and fails closed on substituted profile, lease, fence,
    manifest, or catalog state.
15. The strict existing-v3 bootstrap entrypoint on Store schema v6 never
    installs a missing v3 profile. It may only rebind an exact pending v3
    profile or verify an exact current v3 profile; absence, partial state, or
    collision fails before durable Writer mutation. The separately typed v4
    apply is the only path that may append the schema-v7 successor.
16. A current schema-v6 runtime never falls back to a Writer-v2 procedure or
    ACL. Constructor choice fixes the v2/v3 bind and load-for-update pair for
    the repository lifetime; retained v1 semantic procedures remain shared.
17. Bootstrap inspection verifies the fixed v3 rebind boundary and replay-safe
    history for every retained v3 profile. Exact v2 predecessors are verified
    through their complete foundation, identity, manifest, catalog, ACL,
    ledger, and history profile; only physical Writer absence may bypass those
    Writer-row checks and request the complete Store/Memory fallback.
18. Writer-v2 bridge-pending and Writer-v2 current are distinct closed
    bootstrap states. A preflight or executor cannot substitute one for the
    other, and an already-installed Writer-v3 bridge verifies rather than
    recreates its fixed rebind procedure on retry.
19. Writer v4 apply accepts only the exact replay-verified schema-v6/v3-current
    predecessor and appends one matching v4 `UPGRADED` row. Its fixed rebind
    accepts only that exact `V6V4Bridge` after Store has staged exact schema v7,
    then appends the matching `REBOUND` row and yields `V7V4Current`.
20. The v4 ledger admits only exact ordinal pairs `2/3`, `4/5`, or `6/7` for the
    three retained predecessor histories. Skipped generations, schema v8+,
    arbitrary future manifests, and any edit to frozen v3 SQL/rebind bytes fail
    closed.

## Allowed Dependencies

- `lattice-writer-lease` 1.1 public planner, repository, snapshot, checkpoint,
  and replay APIs.
- `lattice-contracts` immutable project, authority, and persistence evidence.
- Exact pinned synchronous PostgreSQL, hashing, and bounded error libraries.
- Rust standard library.

## Forbidden Dependencies

- Postgres Store implementation, Task Domain, Task Ledger, Policy,
  Orchestrator, Gateway IPC, MCP, Codex, Workspace/Git, verification, Graphify,
  Hermes, Codebase Memory, Guardian, provider/model SDKs, product repositories,
  environment/credential loaders, dynamic SQL, migration discovery, or another
  concrete adapter.

## Failure, Compatibility, And Migration

Missing, partial, drifted, unknown-version, wrong-checksum, wrong-owner,
wrong-ACL, stale-head, serialization-exhausted, corrupt, overflowed, or
outcome-unknown state fails closed with bounded typed errors. No adapter repair,
row skipping, fallback fake, or synthetic genesis is allowed.

Version 1.0 is an independent extension and does not change the immutable
global migration history. Installation and rollback are explicit
administrative operations. Rollback is allowed only before live lease state is
admitted or through a later versioned migration that preserves fencing
high-water/currentness; dropping durable authority to make tests pass is
forbidden.

Version 1.1 accepts only the closed bridge sequence defined by SPEC-003 v5 and
TASK-076. The Writer owner may move an exact global-v3/Memory-v2/v1 profile to
v2 bridge state only when replay proves all retained state and no current
`ACTIVE` or `SUSPECT` lease exists. It later activates the same v2 identity
only after exact global-v5/Memory-v3 verification. Both steps hold global,
Memory, and Writer advisory locks in that order, preserve every semantic row
and high-water, append exact ledger provenance, and fail closed on ambiguity.
Every v2 bridge or pending profile, including `G3_M2_W2_BRIDGE`, never admits
runtime use. Only the exact W1 current profile and the exact final W2 current
profile are executable.

Version 1.2 accepts only the exact v2-current schema-v5/Memory-v3 predecessor,
the runtime-closed v3 bridge, the exact schema-v6 successor reserved by
ADR-025, and the final v3-current rebind. V2 continues to accept only global
schema 3/5. Unknown generation, skipped/duplicate ledger ordinal, missing
`0007_foreman_coordination`, wrong `FOREMAN_COORDINATION` stream or
`FOREMAN_SNAPSHOT_RECORDED` event identity, catalog/ACL drift, and cross-
generation replay fail closed. TASK-087 does not implement the event or global
migration.

Version 1.3 closes only the Writer-owned administrative seam: typed v3 apply
and rebind call the exact extension and fixed rebind procedure. Store may
sequence that procedure only in its schema-v6 transaction after exact-v5 prefix
verification or during exact-v6 idempotent retry. Store remains unable to
install, parse, persist, replay, or reinterpret Writer Lease state.

Version 1.4 preserves the general TASK-094 rebind API for its frozen callers
and adds one stricter TASK-105 product-bootstrap entrypoint. On global schema
v6, Writer absence is not a fresh-install opportunity: the strict entrypoint
rolls back without catalog, ledger, ACL, or identity mutation. Bridge-pending
may rebind and current may verify idempotently.
Its fixed rebind SQL accepts only the corrected seven-entry v6 manifest; the
prior pre-product digest is rejected rather than treated as current.

Version 1.5 adds only the explicit Writer-v3 runtime constructor required by
TASK-105 composition. It verifies the embedded v3 manifest and exact v6 target
before invoking the v3 procedure pair; it neither widens v2 nor changes Writer
domain semantics or physical rows.

Version 1.6 accepts only Store 1.17's corrected seven-entry schema-v6 manifest
at the existing fixed rebind procedure. Prior pre-product digests fail closed.

Version 1.7 accepts only Store 1.18's successor manifest at the same fixed
procedure and changes no Writer runtime, ACL, lease, or fencing behavior.

Version 1.8 adds only the read-only closed bootstrap classifier described
above. Rebind and apply still reclassify under their own Writer locks, so the
inspection result is not mutation authority and cannot bypass TOCTOU checks.

Version 2.0 supersedes the unreleased 1.9 proposal that would have widened v3
to schema v7. V3 SQL, rebind, identity, checksum, and schema-v6 runtime profile
remain immutable. The Writer-owned v4 apply appends an exact schema-v6 bridge;
the Store-owned v6-to-v7 transaction then calls only
`writer_lease_rebind_v4()`, which verifies the staged exact v7 target, updates
Writer identity, and appends the matching `REBOUND` row atomically. Runtime
construction uses only `new_v4_v7`. The classifier enumerates exact versions
5, 6, and 7 rather than accepting a future range.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Extension closure | exact bytes/checksum, install/no-op, partial/drift/collision, columns/defaults/tombstones, constraints/indexes, function body/result/proconfig, namespace objects, and all-principal ACL matrices | Security review | yes |
| Planner parity | PostgreSQL adapter uses Writer Lease 1.1 planner/replay/checkpoint without duplicate semantic builders | Architecture review | yes |
| Concurrent authority | two concurrent acquire attempts yield one current writer and durable exact denial/retry evidence | Engineering | yes |
| Fence safety | release/reacquire, reconnect, process/database restart, max/overflow, and stale-fence rejection prove monotonic non-reuse | Security review | yes |
| Atomic durability | command/transition/snapshot/checkpoint/high-water commit together; physical rows replay exactly; real commit-response interruption reconciles safely | Engineering | yes |
| Recovery | expiry, heartbeat, suspect, exact release, holder death, and newer-leadership matrices | Security review | yes |
| Runtime isolation | fixed runtime functions only, direct table denial, no dynamic SQL/credential/environment input | Architecture review | yes |
| Fresh replay | new client/process reconstructs exact current authority and checkpoint after PostgreSQL restart | Integration review | yes |
| Strict v6 bootstrap | absent Writer v3 fingerprint is unchanged; bridge-pending rebinds; current retry is read-only | Integration review | yes |
| Append-only v4 successor | frozen v3 byte/hash equality; exact v4 apply and `V6V4Bridge`; same-transaction v7 rebind; ordinal 2/3, 4/5, 6/7 histories; partial/substituted/future rejection | Compatibility and security review | yes |
| Strict v7 runtime | exact `V7V4Current` plus Store-v7/Memory-v3 manifest binding; only `new_v4_v7` and v4 runtime procedures are reachable | Architecture and integration review | yes |
| Full verification | format, strict lint, focused/workspace Rust tests, repository checks, and diff check | Engineering | yes |

## Change Policy

Extension identity, physical ownership, repository behavior, transaction
boundary, fencing persistence, currentness, dependency direction, roles/ACLs,
failure semantics, or acceptance gates require a versioned constitution
amendment, SPEC/ADR trace, security and architecture review, and responsible-
user authorization. This constitution cannot be weakened to relabel fake or
synthetic evidence as production authority.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.8 | 2026-08-25 | SPEC-009 v1, ADR-027, TASK-105 live correction | Add a read-only closed bootstrap profile with exact v2/v3 foundation, catalog, rebind-boundary and replay verification; composition receives no Writer rows and every executor reclassifies under lock | TASK-105 bounded implementation authority |
| 1.0 | 2026-08-09 | SPEC-003 v3, ADR-023, TASK-038 | Independent exact PostgreSQL extension and repository for Writer Lease 1.1 with atomic replay/currentness and monotonic fencing | User TASK-038-first direction |
| 1.1 | 2026-08-14 | SPEC-002 v33, SPEC-003 v5, ADR-023, TASK-076 | Preserve v1 history and add the Writer-owned v2 bridge/current profiles for global-v5/Memory-v3 without changing lease semantics or fencing bytes | User continuation authorization |
| 1.2 | 2026-08-21 | SPEC-002 v36, ADR-025, TASK-087 | Add append-only v3 bridge/current compatibility for exact future global-v6 foreman coordination while freezing v2 schema-3/5 behavior | Fixed-foreman delegation |
| 1.3 | 2026-08-24 | SPEC-002 v37, ADR-026, TASK-094 | Writer-owned typed v3 apply/rebind administration for exact-v5 transition and exact-v6 idempotent retry through one fixed Store transaction boundary | TASK-094 bounded repair authority |
| 1.4 | 2026-08-25 | SPEC-009 v1, ADR-027, TASK-105 | Add strict existing-v3 product-bootstrap rebind/verify, fail closed on schema-v6 Writer absence, and re-pin the fixed rebind boundary to the corrected seven-entry v6 manifest | TASK-105 bounded implementation authority |
| 1.5 | 2026-08-25 | SPEC-009 v1, ADR-027, TASK-105 | Add explicit version-closed Writer-v3 runtime construction for schema-v6 composition while preserving the historical v2 adapter path | TASK-105 bounded implementation authority |
| 1.6 | 2026-08-25 | SPEC-009 v1, ADR-027, TASK-105 live correction | Re-pin the still-undeployed Writer rebind boundary to Store 1.17 after closing the foreman event finalizer allowlist | TASK-105 bounded implementation authority |
| 1.7 | 2026-08-25 | SPEC-009 v1, ADR-027, TASK-105 live correction | Re-pin the still-undeployed rebind boundary to Store 1.18's PostgreSQL-valid equivalent foreman scalar bounds | TASK-105 bounded implementation authority |
| 1.9 | 2026-08-26 | ADR-023 Phase 3 amendment | Initial proposal to extend Writer v3 through Store v7; superseded before release because deployed predecessor bytes must remain immutable | User Phase 3 authorization |
| 2.0 | 2026-08-26 | ADR-023 Phase 3 P1 correction | Freeze v3 at schema v6 and add append-only Writer v4, exact `V6V4Bridge`/`V7V4Current`, fixed v4 rebind, and `new_v4_v7` runtime construction | User Phase 3 authorization |
