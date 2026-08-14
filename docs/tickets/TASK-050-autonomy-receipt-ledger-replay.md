---
ticket_id: TASK-050
title: Autonomy receipt durable Task Ledger event and fresh-process replay
spec_id: SPEC-002
spec_version: 35
related_spec_id: SPEC-003
related_spec_version: 5
module_id: task-ledger
constitution_version: 2.3
status: in_progress
human_gate: approved_task050_profile_and_ports_amendment
governance_amendment_authorized_at: 2026-08-15
governance_amendment_authority: direct_user_reply
governance_amendment_source_thread_id: 019ffee6-488a-70e0-8990-9aa9133892a7
governance_amendment_authorized_at_utc: 2026-08-14T16:40:08Z
governance_amendment_decision: task050_profile_owner_ports_and_fresh_latticed_repair
parallel_safe: false
depends_on:
  - TASK-075
  - TASK-038
  - commit:175633ca40352a314a0b699c7cb53697c239d481
integration_sources:
  task_075_implementation: a3599c18d9462732c3b82c9e7d302980657eeccc
  task_075_combined_candidate: f32531002a0c6588e96dc9fe0229db7e0ed546e0
branch: feature/task-050-autonomy-receipt-ledger-replay
implementation_worktree: lattice-worktrees/task-050-autonomy-receipt-ledger-replay
implementation_head: 714f3b9057db47e694adacf9aef5f37e09f31712
closure_branch: feature/task-076-postgres-writer-lease-v2
closure_worktree: lattice-worktrees/task-076-postgres-writer-lease-v2
combined_revalidation_head: f32531002a0c6588e96dc9fe0229db7e0ed546e0
authorized_path_reconciliation:
  - path: crates/lattice-postgres-store/src/migrations.rs
    source_thread_id: 019ff693-b2c3-7a81-9704-49f1e6e3f2d1
    authorized_at_utc: 2026-08-12T16:10:10.180Z
  - path: crates/lattice-postgres-store/tests/migration_contract.rs
    source_thread_id: 019ff693-b2c3-7a81-9704-49f1e6e3f2d1
    authorized_at_utc: 2026-08-12T16:10:10.180Z
  - path: crates/lattice-orchestrator/tests/controlled_task.rs
    source_thread_id: 019ff72b-9704-7b42-92c7-b6aaaa980dd1
    authorized_at_utc: 2026-08-12T18:32:34.806Z
allowed_paths:
  - docs/tickets/TASK-050-autonomy-receipt-ledger-replay.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-011-task-ledger-event-receipt-and-resource-ownership.md
  - docs/adr/ADR-019-durable-postgres-task-ledger-and-outbox.md
  - docs/modules/task-ledger/MODULE_CONSTITUTION.md
  - docs/modules/lattice-ports/MODULE_CONSTITUTION.md
  - docs/modules/orchestrator-runtime/MODULE_CONSTITUTION.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/modules/latticed/MODULE_CONSTITUTION.md
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-contracts/tests/task_ingress_contracts.rs
  - crates/lattice-orchestrator/src/autonomy.rs
  - crates/lattice-orchestrator/src/lib.rs
  - crates/lattice-orchestrator/tests/autonomy_control.rs
  - crates/lattice-orchestrator/tests/controlled_task.rs
  - crates/lattice-task-ledger/src/lib.rs
  - crates/lattice-task-ledger/tests/task_ledger.rs
  - crates/lattice-ports/src/lib.rs
  - crates/lattice-postgres-store/src/postgres_setup.rs
  - crates/lattice-postgres-store/src/migrations.rs
  - crates/lattice-postgres-store/src/task_ledger.rs
  - crates/lattice-postgres-store/src/live.rs
  - crates/lattice-postgres-store/tests/migration_contract.rs
  - crates/lattice-postgres-store/tests/postgres_task_ledger.rs
  - apps/lattice-runtime/src/task_control.rs
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/tests/task_control.rs
  - apps/lattice-runtime/tests/mcp.rs
  - db/migrations/0006_task_autonomy_receipt.sql
  - scripts/run-task050-autonomy-receipt-acceptance.ps1
  - scripts/test-task050-autonomy-receipt-acceptance.ps1
  - PLANS.md
  - HANDOFF.md
---

## Authority And Objective

## Current Implementation State

The original implementation source remains `714f3b9`; its autonomy-at-ordinal
`0005` history is not accepted. TASK-075 re-authored that behavior as the exact
schema-v5 `0006`, and TASK-076 supplied the Writer Lease v2 bridge required to
reach the combined candidate without rewriting historical receipts or fencing
state. Both dependencies are complete and the exact combined candidate
`f32531002a0c6588e96dc9fe0229db7e0ed546e0` emitted the embedded TASK-050 PASS
marker. Independent review proved that the current runner launches the
Postgres Store `postgres_task_ledger` test binary rather than a fresh canonical
`latticed` process, so that marker is retained only as partial baseline
evidence and is not closure evidence. This ticket remains `in_progress` for a
bounded repair and fresh-process revalidation; it does not unlock TASK-051
until its own completion is recorded.

The user decision relayed from coordination thread
`019ff693-b2c3-7a81-9704-49f1e6e3f2d1` requires Autonomy Intent/Receipt to
exist as one new versioned formal Task Ledger event and the only durable truth.
Existing external MCP task Status output remains unchanged.

Implement the smallest vertical slice that appends one authoritative autonomy
receipt to the existing Task stream, commits its fixed scalar persistence in
the same PostgreSQL transaction as the Ledger event/command/head/checkpoint,
and reconstructs the same internal receipt after a process and PostgreSQL
restart without invoking a model, Git, GitHub, verification, or another
external effect.

The accepted implementation base must contain
`175633ca40352a314a0b699c7cb53697c239d481`, whose pure
`AutonomyIntent`/`AutonomyReceipt` classifier is non-durable. This ticket does
not treat that commit or its tests as persistence evidence.

## Approved Repair Scope - reviewed 2026-08-14, authorized 2026-08-15

Independent code/security and architecture review found no P0 issue and four
closure blockers:

1. Autonomy-receipt classification, canonicalization, and hashing currently
   have overlapping implementations in Contracts, Orchestrator, Task Ledger,
   and Postgres Store. Task Ledger must become the sole semantic owner, while
   Postgres Store maps durable scalar rows and delegates verification.
2. No authoritative, versioned `TASK_CREATED` profile discriminator currently
   distinguishes a historical profile, where `autonomy_receipt = None` is
   valid, from an autonomy-required profile. Required-profile missing,
   duplicate, late, or unknown values must fail closed through transition,
   replay, and Status projection.
3. The public `TaskLifecyclePort` and `TaskLifecycleEvidence` contract changed
   without a versioned Ports constitution amendment or matching SPEC trace.
4. The existing acceptance runner does not launch and restart fresh canonical
   `latticed`, so it does not yet prove the public MCP/Status path, both
   `ASK_USER` and `PROCEED`, or zero prohibited downstream effects.

The prior authorization records under `authorized_path_reconciliation` make
the three restored implementation/test paths part of this ticket's durable
allowlist. They do not authorize the new profile/Ports governance decision.

The user approved the narrow Human Gate on 2026-08-15: make Task Ledger the owner of
an exact versioned authoritative profile discriminator; preserve historical
optional receipt semantics while making autonomy-required profiles fail
closed; version and trace the Ports contract; add only
`docs/modules/lattice-ports/MODULE_CONSTITUTION.md` to `allowed_paths`; remove
the out-of-bound public Contracts SHA-256 helper so Contracts remains at 1.13;
and keep the public MCP wire at exactly four tools and six output fields. This
authorization is now consumed by the governance and repair work below; it does
not authorize any other path, migration, public contract, merge, or release.

## Task-created Profile Contract

- Task Ledger is the sole owner of the semantic domain
  `lattice.task-ledger.task-created-profile`, version `1.0`. Its carrier is the
  existing hash-bound `TASK_CREATED.action`; no event field, database column,
  or public MCP field is added.
- `CONTROLLED_CODEX_CANARY` maps to the historical
  `HistoricalAutonomyOptionalV1` profile.
- `CONTROLLED_CODEX_CANARY_AUTONOMY_V1` maps to
  `AutonomyReceiptRequiredV1`.
- Other existing action families are `NotApplicable` and retain their existing
  bytes and generic Ledger behavior. Unknown values in the reserved
  `CONTROLLED_CODEX_CANARY*` namespace fail with
  `LEDGER_UNKNOWN_TASK_CREATED_PROFILE` during append, replay, and Status.
- A required profile with only `TASK_CREATED` is
  `PendingRequiredReceipt`: it may be reconciled only by exactly one typed
  `AUTONOMY_RECEIPT_RECORDED` append at sequence `2`. It cannot report terminal
  success, progress, dispatch, or execute an external effect. Missing, late,
  duplicate, or unknown receipt/profile state fails closed.
- Historical optional profiles may have no receipt. If an autonomy receipt is
  present, it is still unique and sequence `2`.
- Task Ledger exposes typed classification and autonomy-append planning over a
  verified stream. Generic `AppendCommand::new` cannot construct or forge an
  `AUTONOMY_RECEIPT_RECORDED` event. Postgres Store consumes the verified
  scalar plan and performs I/O only; Orchestrator supplies a pure recommendation
  and does not own canonical receipt or hash semantics.

## Canonical Event Contract

### Event identity

- `LedgerEventKind`: `AUTONOMY_RECEIPT_RECORDED`.
- Event payload schema: `lattice.autonomy-receipt/1.0`.
- Event `action`: `RECORD_AUTONOMY_RECEIPT_V1`.
- Event `outcome`: `RECORDED`.
- Event `reason_code`: `AUTONOMY_DECISION_RECORDED`.
- Receipt hash domain: `lattice.autonomy-receipt` version `1.0`.
- Authority hash domain: `lattice.autonomy-authority` version `1.0`.
- The Ledger event `subject_digest` must equal the canonical autonomy receipt
  digest. Diagnostic text is absent and cannot carry any authoritative field.

### Canonical autonomy receipt subject

The closed subject has exactly these top-level fields; unknown, duplicate,
missing, mistyped, zero-digest, or non-canonical values fail closed:

1. `schema_version`: fixed `lattice.autonomy-receipt/1.0`.
2. `binding`: exact `project_id`, `project_snapshot_id`, `task_id`,
   `task_revision`, and `task_spec_digest` from the stream identity.
3. `intent`: exact `version`, `task_kind`, `risk_class`,
   `execution_preapproved`, `requires_new_authority`, and
   `irreversible_or_high_risk` values consumed by the pure classifier.
4. `observed_task_state`: the replay-verified Task Domain state immediately
   before this event.
5. `decision`: closed `disposition`, `reason`, `model`, and `verification`
   values. `ASK_USER` requires `model = null` and `verification = null`;
   `PROCEED` requires both non-null and equal to a fresh recomputation from the
   canonical intent and observed state.
6. `authority_digest`: the digest defined below.

The event-owned PostgreSQL subject row must persist fixed scalar fields needed
to reconstruct this subject. Generic JSON/JSONB, diagnostic payloads, a file,
process cache, MCP session, or a second autonomy table with independent state
is forbidden. The subject row is part of the Task Ledger event record and must
be atomically bound to its `stream_id`, event sequence, event digest, command
receipt, full head, and complete Ledger checkpoint.

### Canonical authority digest

`authority_digest` hashes one closed `lattice.autonomy-authority/1.0` subject
with exactly:

- the same five-field `binding`;
- `authority_mode`, fixed to `P0_PROCESS_START_PROFILE_V1` in this slice;
- non-zero `process_start_authority_digest`;
- non-zero `ingress_profile_adapter_commitment`;
- non-zero live `store_authority_head_digest`;
- `policy_decision_receipt_digest` and `policy_owner_head_digest`, both
  explicit `null` for the fixed P0 R0 canary;
- `approval_receipt_digest` and `approval_owner_head_digest`, both explicit
  `null` for the fixed P0 R0 canary;
- `writer_lease_receipt_digest`, `writer_lease_head_digest`, and decimal-string
  `writer_fencing_token`, all non-null for `PROCEED` and all explicit `null`
  for `ASK_USER`.

For `PROCEED`, `writer_lease_head_digest` is the domain-separated commitment of
the exact 15-scalar tuple that the fixed
`writer_lease_assert_current_v1` predicate independently proves current in the
same transaction: project, snapshot, task, revision, spec digest, attempt,
lease, holder, worktree, holder process and process-start identity, daemon and
epoch, fence, and Writer receipt digest. Structural receipt fields outside that
predicate are not claimed as independently current and cannot change the
durable autonomy subject.

`execution_preapproved = true` is never authority by itself. It is accepted
only when the complete authority subject recomputes to `authority_digest` and
all required owner heads are independently current. R1/R2/R3 or any non-P0
authority mode is outside this ticket and fails closed rather than projecting
synthetic Policy or Approval evidence.

## Ordering, Idempotency, Lease, And Fence Rules

- An `AutonomyReceiptRequiredV1` Task must contain exactly one V1 autonomy receipt event after
  `TASK_CREATED` and before any writable workspace, Codex, verification, Git,
  downstream, or other external effect.
- `PROCEED` may append only with the exact current live Writer Lease authority
  bound to the same project/snapshot/task/spec and asserted inside the same
  PostgreSQL transaction before Ledger mutation. Missing, stale, released,
  suspect, substituted, wrong-project, wrong-fence, reused, zero, or synthetic
  lease evidence fails closed with zero event and zero later effect.
- `ASK_USER` appends without a Writer Lease and must produce zero downstream
  effects. Supplying ambient Writer Lease authority to this path is rejected.
- A changed second V1 receipt for the same Task is rejected. Exact retry of the
  same command and canonical request returns the byte-identical terminal
  command receipt without another event or subject row.
- Unknown commit outcome returns no success receipt, poisons the current store
  instance, and requires a fresh connection plus exact-command reconciliation.
- Historical optional task-control streams remain valid with internal
  `autonomy_receipt = None`. Not-applicable actions retain generic Ledger replay
  compatibility but cannot form `TaskLifecycleEvidence` or normal Task Status.
  A receipt-required profile cannot advance without exactly one valid event at
  sequence `2`.
- Unknown Ledger event kind, receipt/authority schema version, event-owned row
  version, malformed scalar, orphan row, row/event digest mismatch, missing
  row, duplicate row, reordered event, checkpoint drift, or an old binary that
  cannot parse the new event fails closed. No unknown-event skip or downgrade
  is allowed.

## Internal Status Projection And Stable MCP Wire Contract

- Extend internal `TaskLifecycleEvidence` with one neutral closed
  `TaskLifecycleAutonomyEvidence`: `Unadmitted`,
  `HistoricalOptional(Option<AutonomyReceiptProjection>)`, or
  `RequiredComplete(AutonomyReceiptProjection)`. Do not use independent profile
  and receipt `Option` fields that can represent an invalid combination. The receipt contains
  the canonical receipt digest, authority digest, event digest, observed state,
  disposition, reason, model, and verification; it is reconstructed only from
  verified Ledger replay.
- Fresh required admission returns only
  `TaskLifecycleAdmission::PendingRequiredReceipt { binding,
  ledger_head_digest }` until sequence `2` is durable. This bounded result is
  not lifecycle evidence and authorizes only the typed receipt reconciliation;
  normal load, transition, dispatch, and Status continue to fail closed.
- Receipt-required Status requires one valid receipt. Historical optional
  profiles use `HistoricalOptional(None)`; they do not synthesize a receipt. Not-applicable or
  missing profile evidence at the task-control boundary fails closed rather
  than being mistaken for a receipt-optional controlled canary.
- `lattice_task_submit` and `lattice_task_status` must still emit exactly the
  existing six MCP fields: `schema_version`, `status`, `task_state`,
  `task_ref`, `ledger_head_digest`, and `result_digest`.
- The public schema remains `lattice.task.status.v1`; four tool names, input
  schemas, six-field output schema, and `task_ingress_schema_digest()` remain
  byte-identical. `autonomy_receipt` is internal only and must be rejected if
  supplied by an MCP caller or emitted on the wire.

## Preconditions And Fail-Closed Boundary

1. TASK-075 completed its exact schema-v5 reconciliation on the combined
   candidate. The accepted migration order is Project Registry schema-v4
   `0005` followed by `db/migrations/0006_task_autonomy_receipt.sql` as schema
   v5. Any other ordinal, edited Registry `0005`, missing profile provenance,
   or substituted combined candidate fails closed.
2. The implementation base must contain `175633c` and the accepted TASK-038
   Task Ledger/Writer Lease/PostgreSQL boundaries. Task-owned paths must have no
   unknown drift. The existing unrelated dirty
   `scripts/test-task038-four-tool-acceptance.ps1` is explicitly excluded and
   must not be modified, staged, cleaned, reset, or used as TASK-050 evidence.
3. SPEC-002 v35, ADR-011/019/020, Task Ledger 2.3, and Postgres Store 1.10 must
   remain aligned with TASK-075's closed event, mixed historical replay,
   event-owned subject persistence, per-command Registry profile provenance,
   and unchanged MCP wire contract. Do not broaden `allowed_paths` implicitly.
   ADR-022, Contracts 1.13, and Postgres Codebase Memory 1.2 must preserve
   v2/global-v3 receipt identity while admitting exact v3/global-v5.
4. Migration `0006` must prove fresh install and exact prefix upgrades through
   Registry schema v4 without rewriting historical events, receipts, heads,
   checkpoints, Registry persistence receipts, or Store receipts. A database
   with autonomy at ordinal `0005` must already have failed closed under
   TASK-075 before any DDL; TASK-050 does not repair it.
5. No current verified PostgreSQL test runtime, current live Store authority,
   or current Writer Lease authority may be inferred from documents or old
   receipts. Machine acceptance must create disposable current evidence.

## Acceptance Criteria

- [x] Governance first: SPEC-002, ADR-011/019, and the five module
  constitutions agree on the new event owner, subject persistence, mixed
  historical compatibility, dependency direction, and six-field wire freeze.
- [ ] Closed-contract tests reject every unknown/extra/missing field and every
  mutation of binding, intent, observed state, decision, authority, or digest.
- [ ] Ports tests prove the closed autonomy-evidence sum type cannot represent
  required-without-receipt, profileless-with-receipt, or admitted
  not-applicable lifecycle evidence.
- [ ] Task Ledger tests own the exact task-created profile mapping and canonical
  authority/receipt hashes, add/parse only `AUTONOMY_RECEIPT_RECORDED`, deny a
  generic forged append, preserve all existing event/hash fixtures, and reject
  unknown profile, event, or payload versions.
- [ ] Replay proves `TASK_CREATED -> AUTONOMY_RECEIPT_RECORDED`, exactly-one
  semantics, exact retry, changed-command substitution denial, orphan/duplicate
  subject denial, digest tamper, reordering, truncation, and trusted-checkpoint
  rollback detection.
- [ ] Authority tests prove `execution_preapproved` alone has zero authority;
  `PROCEED` requires the exact current live lease/head/fence and `ASK_USER`
  rejects ambient writer authority and produces zero later effects.
- [ ] PostgreSQL tests atomically commit event-owned scalar subject, event,
  command receipt, head, projection, checkpoint, and physical Store receipt;
  injected failure at every boundary leaves no partial durable record.
- [ ] Fresh install plus exact v1/v2/v3/v4-to-schema-v5 upgrade through
  Registry `0005` and autonomy `0006` passes without historical rewrite;
  partial, edited, wrong-order, misplaced-autonomy-`0005`, unknown,
  active-at-upgrade, or rollback-incompatible state fails closed.
- [ ] A disposable PostgreSQL 17 runtime physically restarts; fresh canonical
  `latticed` processes prove both `ASK_USER` and `PROCEED`, reconstruct a
  byte-equal internal autonomy projection through Status, and produce zero
  model, Git, GitHub, verification, Graphify, Hermes, Memory, or
  product-worktree effect.
- [ ] Rust MCP tests prove exactly four tools and byte-identical closed input
  and six-field `lattice.task.status.v1` output. Internal receipt data never
  appears in discovery, arguments, results, errors, logs, or acceptance files.
- [ ] Format, changed-slice strict Clippy, focused Rust tests, PostgreSQL
  integration, `npm.cmd run check`, dependency/forbidden-reference scan,
  independent code/security review, architecture review, and final diff check
  pass with no unresolved P0/P1 finding.

## TDD Behaviors

1. RED generic autonomy-event forgery plus unknown task-created profile;
   GREEN Task Ledger-owned classifier and typed canonical append plan.
2. RED unknown event/payload/authority versions and receipt-field mutation;
   GREEN closed typed canonical contracts and digests.
3. RED missing/duplicate/orphan autonomy subject and changed exact retry;
   GREEN Task Ledger append, event hash, receipt, checkpoint, and replay.
4. RED unfenced/stale/substituted `PROCEED` and writer-bearing `ASK_USER`;
   GREEN exact authority/fence policy with first-failure suppression.
5. RED partial PostgreSQL persistence and mixed-version corruption; GREEN
   atomic schema-v5 `0006` persistence plus non-rewriting historical replay.
6. RED process-cache-only Status and seventh MCP output field; GREEN internal
   fresh-process projection with byte-identical existing six-field wire output.
7. RED any model/Git/GitHub/downstream effect in the acceptance canary; GREEN
   restart-safe receipt-only machine acceptance and cleanup.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Canonical profile/receipt owner | `cargo test -p lattice-task-ledger --test task_ledger` | Exact profile mapping, generic-forgery denial, typed plan, closed decisions and all digest/authority mutations pass |
| Advisory classifier | `cargo test -p lattice-orchestrator --test autonomy_control` | Pure recommendation remains aligned without owning canonical receipt/hash semantics |
| Ledger semantics | `cargo test -p lattice-task-ledger` | New event, exact retry, replay and corruption matrix pass; old fixtures unchanged |
| Lifecycle projection | `cargo test -p lattice-runtime --test task_control` | Exactly-one ordering, lease/fence policy and internal projection pass |
| MCP compatibility | `cargo test -p lattice-runtime --test mcp` | Four tools and exact six-field public output remain unchanged |
| PostgreSQL durability | `cargo test -p lattice-postgres-store --test postgres_task_ledger` | Atomic schema-v5 `0006`, restart, mixed replay, tamper, unknown-version and rollback gates pass against disposable PostgreSQL |
| Focused machine acceptance | `powershell -NoProfile -File scripts/test-task050-autonomy-receipt-acceptance.ps1` | Fresh process returns equal internal receipt and zero prohibited effects |
| Repository gate | `npm.cmd run check` plus documented Rust format/changed-slice Clippy checks | Current local verification passes without using the dirty TASK-038 script |

Commands are acceptance requirements, not claims that this planning ticket ran
them. If a command or package name has drifted at implementation start, stop
and update this ticket before substituting another command.

## Non-Goals

- No new MCP tool, tool name, input field, output field, public schema version,
  generic prompt, project selector, model selector, shell, SQL, path, Git,
  credential, lease, fence, or authority input.
- No model invocation, model routing implementation, scheduler, worker,
  heartbeat, orphan recovery, arbitrary task template, or remote service.
- No product-code modification, Git commit/push/PR effect, GitHub write,
  deployment, release, merge, payment, account, credential, public exposure,
  security-control change, or protected rollback.
- No TASK-037, Hermes, Graphify, Codebase Memory, GH-9 runtime, outbox delivery,
  or second Task/Policy/approval/writer truth.
- No modification of migrations `0001` through Registry `0005`, historical
  event bytes, existing command/Registry persistence receipts, or the unrelated
  TASK-038 acceptance script.

## Dependencies And Overlap

`parallel_safe: false`. TASK-075 is complete; this ticket now closes the
already implemented Task Ledger event set, canonical hashing, the sole
PostgreSQL Ledger transaction, TaskLifecycle
projection, and the P0 Status compatibility boundary. It cannot run in
parallel with another Task Ledger schema/migration, TaskLifecycle, Postgres
Store, MCP Status, Writer Lease assertion, or autonomy-contract change.

## Human Gate

Consumed on 2026-08-15 for the exact Task Ledger-owned versioned profile
discriminator, Ports constitution 1.9/path, owner-boundary repair, Contracts
SHA-helper removal, and unchanged four-tool/six-field MCP contract recorded
above. Any other change to the event name/version, public MCP output, listed
`allowed_paths`, authority profile, migration number `0006`, TASK-075's frozen
Registry `0005`/profile rules, external effect scope, or protected action is a
new decision and blocks this ticket.
