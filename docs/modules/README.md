> 歷史架構與模組索引：保留演進證據；其中 OpenClaw 入口、逐工單治理及舊工具安裝核准流程不再作為現行要求。
> 目前產品邊界與驗證方式以 [AGENTS.md](../../AGENTS.md) 及其工程契約為準。

# Module Scope Routing

## Current V2 Governance State

The user approved the V2 direction and module amendment proposal on
2026-07-29. Active V2 contracts are the versioned constitutions located under
`docs/modules/<module-id>/MODULE_CONSTITUTION.md` and explicitly referenced by
the current V2 ticket.

TASK-008 activates:

- `lattice-core-bootstrap` 1.0;
- `lattice-cli` 1.0.

TASK-009 activates:

- `lattice-contracts` 1.0;
- `lattice-ports` 1.0.

TASK-010 activates:

- `lattice-cjson` 1.0;
- `task-domain` 2.0, subsequently review-hardened to 2.1 by TASK-011.

TASK-011 activates:

- `task-domain` 2.1;
- `policy-engine` 2.1 through the pure Rust `lattice-policy` crate.

TASK-012 activates:

- `project-registry` 1.1 through the pure
  `lattice-project-registry` crate;
- `lattice-contracts` 1.2 shared Project ID/class/lifecycle, physical Git-ref
  identity, fixed producer/version, and full Registry authority receipt/head
  values;
- `policy-engine` 2.3 exact Registry receipt/independent-current-head
  consumption.

TASK-013 activates:

- `task-ledger` 2.0 through the pure `lattice-task-ledger` crate and a
  visibly non-durable deterministic fake;
- `lattice-contracts` 1.3 fixed-producer full Task Ledger stream/resource
  receipt/head representations;
- `policy-engine` 2.4 exact Task Ledger resource
  receipt/independent-current-head consumption.

TASK-014 activates:

- `writer-lease` 1.0 through a pure transition planner, public aggregate
  verifier, and visibly non-durable deterministic fake;
- `lattice-contracts` 1.4 fixed-producer full Writer Lease identity,
  signed-BIGINT value, authority receipt, and head representations;
- `policy-engine` 2.5 exact Writer Lease
  receipt/independent-current-head consumption.

TASK-015 activates:

- `approval-verifier` 1.0 through a pure challenge/proof/nonce/currentness
  planner, aggregate verifier, and visibly non-durable deterministic fake;
- `lattice-contracts` 1.5 complete typed approval subjects plus fixed-producer
  approval receipt/head representations;
- `policy-engine` 2.6 exact Approval Verifier receipt/independent-current-head
  consumption and R3 fail-closed behavior pending Review Runtime.

TASK-016 activates:

- `artifact-store` 1.0 through a pure project-scoped object/reference/
  retention/sweep semantic owner and visibly non-durable in-memory fake;
- `lattice-contracts` 1.6 neutral project-scoped artifact object/generation,
  immutable provenance reference, bounded length, fixed producer, and complete
  receipt/head representations.

TASK-017 activates:

- `gateway-ipc` 1.1 through a bounded NFC-preserving canonical codec, injected
  `GatewayService`, and visibly fake in-memory loopback;
- `lattice-contracts` 1.7 neutral action-specific gateway peer/request/reply
  representations;
- `lattice-ports` 1.2 complete typed gateway service signature and
  component-free Rust-core service error;
- `openclaw-adapter` 2.0 and `orchestrator-runtime` 2.0 governance boundaries,
  without implementing or claiming live OpenClaw compatibility.

TASK-018 activates:

- `postgres-store` 1.0 through a typed project-scoped physical transaction
  contract and visibly non-durable deterministic in-memory fake;
- `lattice-contracts` 1.8 neutral Store scope, daemon authority, physical head,
  request commitment, disposition, receipt, and fake-durability values;
- `lattice-ports` 1.3 typed `transact`/`current_head` methods and Store-specific
  errors while retaining the Contracts-only dependency.

TASK-018 performs no SQL, database connection, migration execution, domain
repository mutation, or live/durable claim. TASK-019 must explicitly adopt or
supersede the inert migration draft and version the Store before adding a
driver, manifest runner, roles, runtime admission, or disposable-database
evidence.

TASK-019 activates `postgres-store` 1.1 only for the exact-manifest PostgreSQL
17 schema/admission foundation. The existing `0001_bootstrap.sql` remains
byte-identical and is manifest-recorded as `SUPERSEDED`; a new runner-owned-
transaction migration is the first executable entry. Runtime startup verifies
but never auto-migrates. Bootstrap admission is `STOPPED` with no leader, and
the ticket adds no live `ControlStore` or durable receipt. Those remain
TASK-020+.

TASK-020 activates `postgres-store` 1.2, `lattice-contracts` 1.9, and
`lattice-ports` 1.4 for one physical live Store slice. Store contract v1
remains fake-only; v2 adds database/schema-bound PostgreSQL durability.
Immutable `0003` upgrades only an exact empty v1 prefix, and runtime reaches
physical heads/terminal receipts only through the three fixed prepare,
finalize, and current-head functions. Domain repositories, Guardian
activation, production targets, providers, products, and release remain later
tickets.

TASK-021 governs Task Ledger 2.1 and Postgres Store 1.3 for the first durable
domain repository. Task Ledger adds the pure runtime-aware vacant/plan/apply,
verified retained-command/outbox replay, and independently retained checkpoint
surface without I/O. Postgres Store advances the global schema to v3, freezes
historical Store-v2 receipt evidence, and atomically persists Ledger command,
optional event/outbox, projection/checkpoint, and physical receipt through
fixed runtime functions. Live resource observation, outbox claim/delivery,
other repositories, activation, providers/products, production, and release
remain later tickets.

TASK-022 governs Project Registry 1.2 and Postgres Store 1.4 for the sole
approved global durable repository exception. Project Registry adds one
runtime-aware vacant/plan/apply/checkpoint/untrusted-replay surface while
preserving Registry 1.1 observation, request, authority-receipt, and
command-result hashes. The existing `result_digest` is the terminal semantic
commitment; checkpoint and record-set subjects are new in Registry 1.2, not
retroactive 1.1 hashes. Postgres Store depends one way on that pure planner/verifier and never
turns the global aggregate into a project-scoped `StoreScope`, synthetic
`ProjectId`/`ProjectSnapshotId`, or Store receipt. Contracts 1.9 and Ports 1.4
do not change.

The commitment graph is acyclic and ordered: checkpoint command core
(ordinal, complete typed request, semantic `RegistryCommandReceipt`) -> exact
logical-retained-state canonical bytes -> result checkpoint -> record set ->
adapter transaction digest -> persistence receipt. The logical state contains
canonical observation/project/command/reservation arrays: observations are
digest-keyed, ordered, and counted once; projects refer to observation digests;
commands are strict ordinal `1..N`; reservations use dimension/digest/status/
Project-ID order; optional values are `null`, text is NFC UTF-8, and unsigned
values are decimal strings. Checkpoint/record-set fields, counts/the byte field,
SQL overhead, and adapter evidence are excluded from logical byte accounting.

Vacant high-water is `0`. The exact vacant Live logical state is
`{"commands":[],"observations":[],"projects":[],"reservations":[],"runtime":"LIVE","schema_version":"1"}`
at 103 bytes; frozen vacant checkpoint digests are
`22ad9599c05ab384e720b8f1d91bdfbe1262360850602aa0f6b5fd79c1797f4f`
for Fake and `5bb1f9d9adf7228bef7ea45cc029e79d71761c4dfed3abe086634917db38c173`
for Live. `RegistryCheckpoint::from_retained` reconstructs the independently
read singleton. Plain `verify_untrusted_registry_snapshot` proves only
self-consistency; durable authority requires
`verify_untrusted_registry_snapshot_against_checkpoint` so an older coherent
prefix cannot hide a removed denial or observation tail. Project Registry
remains pure Rust, zero-I/O, and free of any PostgreSQL dependency.

Schema v4 adds exactly five normalized Registry tables, including immutable
complete observations, plus one singleton global serialization/checkpoint row.
Runtime access is exactly three Store-v4, five Task-Ledger-v2, and nine Project-
Registry-v1 fixed functions; older functions remain ungranted catalog history.
Every new command uses one bounded `SERIALIZABLE` transaction, current-
transaction staging provenance, all-or-none finalization, and commit-unknown
poison/reconnect reconciliation. Store-v2 receipt profile 2 and historical
Store/Ledger receipts remain byte-identical. The bounded limits are 4,096
projects, 65,536 first-seen terminal commands, 67,108,864 logical-state
canonical bytes, and 131,072 UTF-8 bytes per already-NFC canonical root.

TASK-022 lifts only Postgres Store's prior Project Registry persistence
non-goal. Writer Lease, Approval, Artifact, Memory, outbox delivery, real
Windows/Git inspection, activation, providers/products, production,
publication, deployment, and release remain outside the ticket. The governance
amendment does not itself claim Rust/SQL/live PostgreSQL acceptance evidence.

All active V2 modules are listed in SPEC-002 version 24. `lattice-cjson`
owns only canonical-byte mechanics; Task Domain owns its Task Spec hash
subject, while Task Ledger retains event/receipt hash semantics. Policy
consumes Task Domain's immutable public subject through ADR-009 and owns only
deterministic decisions. Project Registry owns full project
identity/lifecycle state while contracts owns only shared representation.
TASK-012 activates a deterministic fake Registry owner with accepted/pending
identity reservation, zero-mutation `Denied` outcomes, and defensive
state-changing `Blocked` suspension. TASK-022 preserves that pure owner and
adds only the governed PostgreSQL global persistence exception; it is still not
a real Windows/Git inspection adapter, and authenticated Orchestrator
composition remains later.
TASK-013 limits Task Ledger legality to pure event/receipt/replay/resource
semantics; TASK-021 adds its first PostgreSQL repository without moving that
legality into SQL. Live resource observation and atomic effect/outbox claim or
delivery remain in a future Orchestrator/PostgreSQL boundary.
TASK-014 limits Writer Lease to pure transition/fencing/recovery semantics and
fake composition. Runtime admission/daemon leadership transitions and physical
process evidence remain outside the module; PostgreSQL still owns serialized
durability, database time, and same-transaction mutation admission.
TASK-015 limits Approval Verifier to pure subject/challenge/proof/nonce/time
semantics and fake composition. OS authentication, trust-root/key access,
database clock/durability/atomic claim, OpenClaw IPC, Review Runtime, and
Guardian activation remain outside the module. Approval Verifier cannot turn
review Booleans into review authority.
TASK-016 limits Artifact Store to pure project-scoped object/reference/
provenance/aggregate-quota/idempotency/currentness/typed-reference-authority/
retention/delete-claim/unknown-outcome reconciliation semantics and fake
composition. Provider provenance is not trust or authority; initial
publication/reference, retain/release/read, and delete claim require typed
fixed-owner receipt/current-head pairs rather than caller counts, Booleans, or
bare digests. Object/task/project/store quotas use explicit accounting and
retain worst-case claim/reconciliation/orphan use until verified terminal
evidence. PostgreSQL reference/quota/claim durability and the owned-root
filesystem streaming/atomic publish/verified read/link containment/exact
unlink mechanics remain outside this slice; no public real deletion exists.

The seven pre-existing V1 constitutions remain characterization contracts for
the preserved Node prototype unless they receive a separately reviewed V2
amendment. A V1-specific name or action is legacy evidence, not an allowed V2
policy dependency.

Missing constitutions fail closed: a crate or application may be scaffolded
only when the active ticket names its governing V2 constitution.
