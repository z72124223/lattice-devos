# TASK-022 Corrected Governance Independent Re-review

## Decision

`PASS — GOVERNANCE BLOCKER RELEASED FOR THE FIRST CHARACTERIZATION/RED STEP`.

Finding count:

| Severity | Open findings |
|---|---:|
| P0 | 0 |
| P1 | 0 |
| P2 | 0 |
| P3 | 0 |

Implementation blocker released: **yes, for governance only**.

All five P1 and all four P2 findings in
`GOVERNANCE_REVIEW_TASK_022_2026-08-03.md` are materially resolved by the
corrected governance set. This decision permits only TASK-022 TDD behavior 1:
freeze the actual Registry 1.1 observation/request/authority-receipt/command-
result characterization vectors, then introduce the first focused Registry 1.2
RED test. It does not claim that TASK-022, its Rust implementation, migration
`0005`, PostgreSQL adapter, schema-v4 catalog, live harness, code/security
review, architecture review, integration, or MVP-1 is complete.

ADR-020's file header remains `proposed` and the ticket remains
`governance-review` because this bounded re-review was authorized to write only
this report. Those labels correctly described the state before this decision;
updating workflow status is a subsequent bookkeeping action, not evidence that
the implementation exists.

## Review Boundary And Independence

- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch/HEAD verified during review:
  `feature/v2-rust-postgres-bootstrap` at
  `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Compared governance artifacts: ADR-020, TASK-022, SPEC-002 v24/AC-36,
  Project Registry 1.2 constitution, Postgres Store 1.4 constitution, approved
  V2 amendment, module index, PLANS, root README, current workflow audit, and
  the first independent governance review.
- Read-only implementation evidence was used only to verify the frozen
  `lattice-cjson-1` frame, existing Registry hash-domain convention, current
  v3 catalog baseline, and repository checks. No TASK-022 Rust, SQL, migration,
  database, branch, commit, push, merge, or deployment action was performed.
- Per the bounded review request, no Rust or PostgreSQL test/harness was run.
  The only repository write made by this review is this report.

## Re-review Of The First P1 Findings

### P1-1 — Acyclic commitment graph: resolved

ADR-020 now freezes separate projections and one normative construction order:

1. checkpoint command core containing only ordinal, complete typed request, and
   complete semantic `RegistryCommandReceipt`;
2. logical retained-state projection and exact byte count;
3. result checkpoint;
4. record set over the command persistence core plus any inserted observation,
   optional project replacement, and ordered reservation deletes/inserts;
5. adapter transaction digest; and
6. persistence receipt last.

ADR-020 lines 110-134 expressly exclude checkpoint references, record-set and
retained accounting, database/schema evidence, and adapter digests from the
checkpoint command core; make checkpoint references verification-only chain
evidence; exclude the record set's own digest and every adapter field; and
forbid substituting a physical command-row projection. TASK-022 lines 102-105,
SPEC-002 lines 381-392 and 1196-1202, Project Registry invariants 18-19, and
Postgres Store invariants 67-68 preserve that same dependency direction.

No digest is required as its own canonical input, and the semantic owner remains
upstream of the persistence adapter. The original self-reference blocker is
closed.

### P1-2 — Logical retained-byte algorithm and vacant fixtures: resolved

ADR-020 lines 144-179 now assign the algorithm to Project Registry under
`lattice.project-registry.logical-retained-state` schema version `1` and freeze:

- the exact top-level object fields;
- complete digest-keyed observations, counted once and sorted by digest;
- current projects sorted by Project ID and referring to observations by
  digest;
- checkpoint command cores in strict ordinal order;
- complete reservations sorted by dimension, digest, status, and Project ID;
- explicit canonical `null`, NFC UTF-8 text, and canonical decimal strings;
- `canonicalize(logical_state).len()` as the sole retained-byte measure; and
- exclusion of hash framing, counts/the byte field, checkpoint references and
  digests, record-set fields and digests, SQL overhead, and all adapter,
  database, schema, transaction, and persistence fields.

Project Registry constitution invariants 20-21, Postgres Store invariants
68-69, SPEC-002 lines 393-417 and 1202-1223, and TASK-022 lines 106-113 repeat
the ownership, ordering, de-duplication, null, UTF-8, decimal, and exclusion
rules. SQL may compare but cannot invent domain accounting.

The vacant canonical fixture was independently recomputed from the checked-in
`lattice-cjson-1` framing code and the existing Registry convention
`HashDomain::new(schema_id, "1")`:

```text
FAKE retained_bytes=103 digest=22ad9599c05ab384e720b8f1d91bdfbe1262360850602aa0f6b5fd79c1797f4f
LIVE retained_bytes=103 digest=5bb1f9d9adf7228bef7ea45cc029e79d71761c4dfed3abe086634917db38c173
```

Both values exactly match ADR-020 lines 181-190 and the repeated fixtures in
TASK-022, SPEC-002, both constitutions, the V2 amendment, module index, and
workflow audit. The original non-reproducible accounting blocker is closed.

### P1-3 — Independent retained current checkpoint: resolved

ADR-020 lines 62-68 now distinguish three operations:

- `RegistryCheckpoint::from_retained` reconstructs a separately read checkpoint
  without asserting currentness;
- `verify_untrusted_registry_snapshot` proves only internal self-consistency;
- `verify_untrusted_registry_snapshot_against_checkpoint` also compares the
  separately read singleton and is mandatory before durable current authority.

TASK-022 lines 92-98, SPEC-002 lines 360-369 and 1191-1196, Project Registry
public contracts lines 73-84, Postgres Store public contracts lines 195-204 and
invariant 55, the V2 amendment, and module index all retain this distinction and
require coherent-prefix/denial-tail rejection. A self-consistent older snapshot
can no longer stand in for the independently retained current anchor. The
rollback/currentness blocker is closed.

### P1-4 — Vacant high-water, singleton seed, and first lock: resolved

ADR-020 lines 87-102 and 252-257 freeze vacant high-water `0`, a non-negative
singleton high-water, and positive immutable command ordinals exactly `1..N`.
ADR-020 lines 388-397 require migration `0005` to seed exactly one Live vacant
singleton with zero high-water/counts, 103 retained bytes, and the frozen Live
digest; the other four Registry tables and command history begin empty, and the
singleton is the first-command lock target.

TASK-022 lines 85-87 and 136-142, Project Registry invariants 12-13, Postgres
Store invariants 54 and 64, SPEC-002 AC-36, the V2 amendment, module index,
PLANS, and workflow audit all agree. There is no longer a first-write race or a
positive-ordinal/vacant-state contradiction. The original seed/ordinal/locking
blocker is closed.

### P1-5 — Nine scalar signatures and PostgreSQL budget: resolved

ADR-020 lines 283-343 define the exact ordered scalar groups and all nine
function signatures. Independent arithmetic gives:

```text
P = 2
C = 8
H = 13
A = 7

prepare       = P2 + command2 + A7 + expected-base1 = 12
five reads    = P2 each                             = 2 each
stage-command = P2 + command6 + H13 + request3 + terminal16
                + base-C8 + result-C8 + record-set1 + A7
                + adapter2 + observation7           = 73
stage-project = P2 + project4 + drift5 + authority11 = 22
finalize      = P2 + command2 + base-C8 + result-C8 + tail7 = 27
```

This is exactly nine functions: one prepare, five reads, two stage functions,
and one finalizer. The maximum is 73, below the frozen PostgreSQL 100-input
limit. TASK-022 lines 124-127, Postgres Store public contracts lines 172-194
and invariant 69, PLANS, and the workflow audit agree and forbid composite/
table arguments, array row maps, JSON payloads, omitted positions, and alternate
overloads. The original implementability blocker is closed.

## Re-review Of The First P2 Findings

### P2-1 — Registry 1.1 golden terminology: resolved

ADR-020 lines 70-85 now says Registry 1.1 has only observation, request,
authority-receipt, and command-result hash subjects; existing `result_digest`
is the terminal semantic command-result commitment. Checkpoint and record-set
subjects are explicitly new Registry 1.2 vectors. TASK-022 lines 78-81,
SPEC-002 lines 348-355 and 418-423, both constitutions, the V2 amendment, module
index, and workflow audit use the same terminology. No historical record-set or
separate terminal-receipt digest is claimed.

### P2-2 — Lock/statement/idle/total transaction bounds: resolved

ADR-020 lines 345-361 freezes `lock_timeout = 5s`,
`statement_timeout = 30s`, local
`idle_in_transaction_session_timeout = 30s`, and a 45-second monotonic
begin-to-pre-commit deadline checked after read batches, after pure replay, and
before staging, finalization, and commit. Timeout/deadline failure rolls back as
typed `Unavailable`, is not commit-unknown, and receives no automatic retry.
ADR-020 lines 409-417 separately reserve outcome-unknown for a commit failure
with no database response.

TASK-022 lines 143-167, Postgres Store invariants 60, 66, and 70 plus its
acceptance matrix, and workflow-audit lines 43-46 agree on `5/30/30/45` and the
failure classification. Client-side replay is no longer outside every bounded
transaction interval.

### P2-3 — Exact schema-v4 catalog totals: resolved

The checked-in `0001`-through-`0004` baseline was enumerated read-only as 10
`control` tables and 11 catalog functions. ADR-020 lines 268-281 add exactly
five Registry tables plus 17 successor functions (`3 Store-v4 + 5 Ledger-v2 +
9 Registry-v1`) and freeze:

```text
15 total control tables
28 total retained catalog functions
17 runtime-executable functions
11 historical functions retained without runtime EXECUTE
```

TASK-022 lines 114-127 and 230-233, Postgres Store owned-data lines 107-112 and
invariant 63, PLANS, and the workflow audit agree. The five-table/nine-Registry-
function slice is no longer confused with the complete migration delta or
catalog verifier total.

### P2-4 — Current workflow-audit classification: resolved

Workflow-audit lines 3-26 now give a current top-level decision and classify
SPEC-002 v24, Project Registry 1.2, Postgres Store 1.4, TASK-022, corrected
ADR-020, the first review, re-review, and TDD implementation separately. Lines
28-50 summarize the corrected singleton, canonical/current checkpoint,
catalog/signature, timeout, and golden-vector decisions.

The old v23/1.1/1.3 rows are retained only under explicit `Original Audit
Snapshot (Preserved)` and `Original Workflow Stage Classification (Preserved
Snapshot)` headings. Lines 59-62 and 134-138 state that those rows are historical
and superseded by the current table. The stale snapshot is no longer presented
as current gate evidence.

## Cross-contract And Architecture Result

No contradictory or unimplementable requirement was found across the requested
artifact set:

- **Semantic ownership:** Project Registry 1.2 remains pure Rust and zero-I/O;
  Postgres Store 1.4 persists and independently verifies its plan without
  deciding identity/lifecycle outcomes.
- **Dependency direction:** ADR-020 lines 419-444 and both constitutions retain
  one-way `lattice-postgres-store -> lattice-project-registry`; there is no
  reverse domain dependency, adapter-to-adapter call, or Contracts/Ports change.
- **Global exception:** the Registry transaction remains separately typed and
  cannot forge or reinterpret a project-scoped `StoreScope`,
  `ProjectSnapshotId`, or `StoreTransactionReceipt`.
- **Migration compatibility:** `0001` through `0004` remain byte-identical;
  schema v4 uses append-only `0005`, preserves Store-v2 receipt profile 2 and
  Task Ledger replay, and replaces runtime grants only through exact successor
  functions while retaining historical definitions ungranted.
- **State and failure model:** exact replay precedes mutable admission; first-
  seen terminal commands, including denial/block/read-only outcomes, advance
  one global checkpoint; partial stages, corrupt projections, currentness
  mismatch, timeout, and unknown commit outcome remain distinct fail-closed
  cases.
- **Plan/ticket/status alignment:** PLANS lines 595-616, README lines 28-41,
  ticket status/front matter, and workflow audit all describe TASK-022 as the
  current governance slice with no implementation claim. The approved V2
  amendment and module index preserve the same scope and explicitly exclude the
  unrelated companion/playmate website.

## Verification Evidence

Read-only checks performed during this re-review:

| Check | Result |
|---|---|
| Recompute vacant logical-state length and Fake/Live checkpoint digests with the checked-in `lattice-cjson-1` frame and Registry hash-domain `1` | `103` bytes for both runtimes; both frozen digests matched exactly |
| Enumerate current `0001`-`0004` `CREATE TABLE control.*` / `CREATE FUNCTION control.*` baseline | `10` tables / `11` functions; planned `+5/+17` gives `15/28`, with `17` new runtime functions and `11` historical-ungranted |
| Independently expand all nine Registry scalar signatures | `12, 2, 2, 2, 2, 2, 73, 22, 27`; maximum `73 < 100` |
| `npm.cmd run check` | `check=ok files=272 constitutions=18 tickets=22 current_tasks=1` after adding this report |
| `git diff --check` | exit `0` after adding this report; the new untracked report was separately verified with zero trailing-whitespace lines, final newline present, and no UTF-8 BOM |
| Rust/PostgreSQL tests | deliberately not run for this document-only governance re-review |

## Released Step And Remaining Gates

The specific PLANS governance blocker is released. The next allowed work is the
first bounded TASK-022 characterization/RED step only:

1. freeze literal Registry 1.1 observation/request/authority-receipt/command-
   result vectors against the current implementation;
2. add the new Registry 1.2 vacant checkpoint/record-set fixtures; and
3. introduce the first focused failing pure Registry test before implementation.

TASK-022 remains incomplete until pure-domain and adapter RED/GREEN work, exact
schema-v4 migration/catalog/ACL implementation, focused/full Rust and Node
verification, marker-owned PostgreSQL 17.10 evidence, independent code/security
and architecture reviews, integration, workflow ledger, and handoff all pass.
Merge and every production/protected action remain outside this decision.

## Residual Risks

- The current tree is an intentional cumulative dirty MVP-0-through-TASK-022
  governance candidate, not a clean per-ticket diff. The allowlist and reviewed
  named artifacts remain documented controls rather than enforced isolation.
- The fixed vacant vectors prove the empty canonical shape. Non-vacant literal
  checkpoint and record-set vectors must be added before planner extraction, as
  TASK-022 already requires.
- The globally locked replay may process up to 64 MiB and can create contention
  or memory/latency pressure even with the frozen bounds; any later snapshot,
  archive, compaction, or limit increase requires a separately versioned design.
- Remote Rust/PostgreSQL CI, upstream synchronization, required reviews, branch
  protection, merge queue, release/rollback controls, and primary-branch merge
  authorization remain missing or unverified. None is supplied by this local
  governance PASS.
- Digest-keyed immutable observations still rely on collision resistance and
  require structural equality plus pure digest recomputation on every retained
  load and exact reuse.
