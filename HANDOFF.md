# LATTICE DevOS TASK-021 Handoff

## Outcome

TASK-021 is complete. LATTICE now has its first durable domain repository:
Task Ledger 2.1 remains the pure Rust semantic owner, while Postgres Store 1.3
atomically persists each terminal command, optional event and outbox admission,
projection/checkpoint, and applied physical Store receipt in PostgreSQL.
SPEC-002 AC-03, AC-04, and AC-35 are complete.

This does not complete MVP-1, MVP-2, MVP-3, or the full platform. MVP-1 is
12/22 tickets (54.5%); AC-05 and AC-19 remain open. Outbox claim/delivery,
live resource observations, the other durable repositories, daemon activation,
OpenClaw/Codex/Graphify/Hermes/Codebase Memory live integration, Guardian
autonomy, production, release, and deployment remain later work.

LATTICE remains the user's general local autonomous AI development platform:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated companion/playmate website remains excluded from the product,
architecture, roadmap, implementation, tests, and project evidence.

## Completed In TASK-021

- Task Ledger 2.1 exposes one Fake/Live pure vacant/plan/apply/checkpoint
  boundary, retains verified appended and denied commands, and derives exactly
  one outbox admission only for an appended `EFFECT_INTENT` with outcome
  `RECORDED`.
- Postgres Store 1.3 adds exact transaction-control-free schema v3 while
  keeping `0001` through `0003` byte-identical and preserving the immutable
  Store-v2 receipt profile for historical exact replay.
- Runtime receives exactly three Store-v3 and five Task-Ledger-v1 fixed
  functions, with zero direct protected-table SELECT/DML and no generic SQL,
  DSN, credential, environment-discovery, or raw-client surface.
- Each new command runs in one bounded `SERIALIZABLE` transaction. Store and
  Ledger finalization are two ordered fixed calls in that same transaction;
  the Ledger finalizer accepts only the Store terminal created by the current
  transaction and any later failure rolls both back.
- Every read/write re-observes dynamic global schema/full-manifest evidence in
  the transaction and compares it with constructor-frozen evidence. Store-v2
  receipt evidence remains separate from global-v3 evidence.
- Outbox replay verifies event digest, command ID, and request digest linkage.
  Duplicate, missing, cross-project/snapshot, checkpoint, terminal, event,
  command, or outbox corruption fails closed and is never auto-repaired.
- Explicit database responses remain known retryable or terminal outcomes;
  only no database response at commit yields `CommitOutcomeUnknown` and
  poisons the adapter. Transactions/functions use 5-second lock and 30-second
  statement timeouts.
- Fresh Store genesis is correctly distinct from the vacant Ledger checkpoint
  until the first atomic mutation. A vacant stream with a same-ID wrong-scope
  physical orphan now fails closed through the three-argument fixed read-head
  function and a direct live regression.

## Review History And Repairs

The initial independent code review blocked closure with P1=4 and P2=2:
constructor-only global evidence, overly broad unknown-commit classification,
incomplete outbox linkage checks, acceptance of a prior Store terminal,
wrong-scope physical-load gaps, and missing bounded timeouts. Architecture
review also blocked on transaction provenance and bounded failure semantics.

All findings received direct repairs/regressions. Live acceptance then exposed
and repaired the fresh Store-genesis/vacant-checkpoint finalizer mismatch and a
test-only ACTIVE-admission restart cleanup gap. Final re-review found one more
P2 vacant wrong-scope orphan read gap; the SQL/Rust/live compatibility unit was
repaired and the full PostgreSQL matrix rerun. Final code/security and
architecture reviews report P0=0, P1=0, P2=0, P3=0; local integration passes.

## Verification Evidence

- PostgreSQL 17.10 marker-owned harness: latest frozen initial and restart
  phases pass, including migration/upgrade, old Store replay, new/exact/
  changed/stale/outbox commands, same/cross-stream concurrency, rollback,
  bounded retry, commit-ack loss, coherent manifest drift, lock timeout,
  current-transaction `xmin`, ACL, wrong-scope orphan, corruption, and cleanup.
- Rust workspace: 432/432 tests across 52 binaries.
- Postgres Store package: 57/57 tests; migration contract: 15/15.
- Preserved Node verification: 44/44 tests.
- `cargo fmt` and strict workspace/all-targets Clippy: pass, zero warnings.
- `cargo audit`: 109 locked dependencies checked against 1,178 advisories;
  zero known vulnerabilities.
- `0004_task_ledger_repository.sql`: 111,742 bytes, SHA-256
  `cd658ed2f4624cd0a829c818c1cf96d8ac3829264046976bdde3b2fc7feea6e5`.
- Full four-entry manifest:
  `09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407`.
- `0001`/`0002`/`0003` retain their TASK-020 bytes and hashes.
- No unmerged path, conflict marker, tracked whitespace error, PowerShell parse
  error, reverse adapter dependency, or temporary focus switch remains.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_021_2026-08-02.md`
- `docs/reviews/GOVERNANCE_REVIEW_TASK_021_2026-08-02.md`
- `docs/reviews/CODE_REVIEW_TASK_021_2026-08-02.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_021_2026-08-02.md`
- `docs/reviews/INTEGRATION_TASK_021_2026-08-02.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

TASK-021 stayed aligned with PLANS Step 6, SPEC-002 v23, ADR-019, Task Ledger
2.1, and Postgres Store 1.3. It adds durable Ledger/outbox-admission truth
without giving PostgreSQL domain meaning or adding outbox delivery, another
writer/truth/gateway, provider/product work, or the unrelated website.

| Gate | Current classification |
|---|---|
| Local Rust/Node tests, format, lint, audit, migration hashes | machine-enforced for this run |
| Disposable PostgreSQL transaction/concurrency/fault/restart behavior | machine-enforced for the exact marker-owned target |
| Fixed functions, direct-table denial, roles, ACLs, catalog and scope | machine-enforced locally plus independent review |
| Module ownership and dependency direction | independently reviewed plus local scans |
| Ticket allowlist | documented plus local scan; no clean per-ticket commit |
| Remote Rust/PostgreSQL CI and branch protection | missing/unverified |
| Primary merge readiness | blocked; no committed candidate, remote, or merge authorization |

## Git, Runtime, And Cleanup State

- Branch: `feature/v2-rust-postgres-bootstrap`; HEAD:
  `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`;
  feature is four committed commits ahead and not behind.
- Remote/upstream: none. MVP-0 through TASK-021 remain one cumulative
  uncommitted dirty result; no merge was performed.
- No disposable PostgreSQL/Cargo/test process remains. The installed Windows
  PostgreSQL 17 service was not replaced or stopped.
- Two stopped ignored diagnostic roots remain under `target/`, both without
  `postmaster.pid` or listeners. Exact removal was blocked by local policy and
  was not bypassed; this is a disclosed hygiene note, not live database state.

## Next Bounded Slice

TASK-022 governance must freeze the durable Project Registry repository before
implementation. It must reuse Project Registry's pure identity/reconciliation
semantics and Postgres Store's schema-v3 transaction/authority boundary,
preserve one-way dependency and project isolation, and prove restart,
concurrency, drift/collision, corruption, and exact replay. It must not start
Writer Lease, Approval, Artifact, OpenClaw/Codex/Graphify/Hermes/Memory,
Guardian, production, release/deploy, or unrelated website work.

---

# Archived TASK-020 Handoff

## Outcome

TASK-020 is complete. LATTICE now has one exact live PostgreSQL 17 physical
`ControlStore`: Contracts 1.9 and Ports 1.4 preserve the fake while Postgres
Store 1.2 supplies schema-v2 migration, fixed-function runtime access,
durable apply/stale terminal receipts, exact replay, bounded pre-commit retry,
unknown-commit reconciliation, and restart evidence. SPEC-002 AC-34 is
complete.

This is a physical durability boundary, not a domain repository. AC-03,
AC-04, AC-05, and AC-19 remain open, and MVP-1, MVP-2, MVP-3, and the full
platform are not yet complete.

LATTICE remains the user's general local autonomous AI development platform:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated companion/playmate website is excluded from the product,
architecture, roadmap, implementation, tests, and project evidence.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-020 complete, TASK-021 next.
- MVP-2 exact-version local components and Codebase Memory: planned after the
  MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2 gate.

## Completed In TASK-020

- Preserved Store contract v1 as visibly fake/non-durable and introduced exact
  v2 live PostgreSQL persistence evidence without changing unrelated contract
  behavior.
- Changed only the synchronous `ControlStore::current_head` observation to
  explicit mutable access; Ports remains Contracts-only and driver-free.
- Kept `0001` and `0002` byte-identical and added one exact
  `0003_live_control_store.sql` expansion. Fresh targets and verified empty
  exact-v1 prefixes upgrade; drifted, partial, reordered, edited, unknown, or
  non-empty sources fail closed.
- Added exactly three fixed `SECURITY DEFINER` runtime functions with safe
  ownership/search-path/ACL properties. Runtime has no direct physical or
  terminal table access and cannot migrate or self-activate.
- Added `PostgresControlStore` over a caller-supplied authenticated client. It
  rechecks exact ACTIVE daemon authority and the locked physical head inside a
  bounded `SERIALIZABLE` transaction, and returns durable evidence only after
  successful commit.
- Exact retry remains byte-identical after admission, epoch, or head changes;
  changed transaction-ID reuse reveals no retained receipt. Unknown commit
  response returns no receipt, poisons the instance, and requires reconnect
  plus the exact request for reconciliation.
- Expanded the marker-owned PostgreSQL harness for fresh/upgrade/rollback,
  permissions, apply/stale/replay/substitution, concurrency, serialization
  exhaustion, overflow, corruption, response loss, restart, and exact cleanup.

## Verification Evidence

- Full Rust workspace: 409/409 tests.
- Preserved Node suite: 44/44 tests.
- `cargo fmt` and strict all-target/all-feature Clippy: pass with zero warnings.
- `cargo audit`: 109 locked dependencies checked against 1,178 advisories;
  zero known vulnerabilities.
- Duplicate dependency roots: zero.
- PostgreSQL 17.10 marker-owned initial/restart harness: self-test and complete
  TASK-020 live/fault/upgrade matrix pass.
- `0003_live_control_store.sql`: 29,518 bytes, SHA-256
  `00ae3eedd76704f26b1df58955d9d594c98f0ba525be93b15d8c9ebb1f2115c1`.
- PowerShell AST, source-scope, secret/DSN/raw-client/dynamic-SQL, conflict,
  temporary-marker, whitespace, dependency, and governance checks pass.
- Independent code/security and architecture reviews pass with P0-P3 all zero;
  local combined integration passes.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_020_2026-08-02.md`
- `docs/reviews/CODE_REVIEW_TASK_020_2026-08-02.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_020_2026-08-02.md`
- `docs/reviews/INTEGRATION_TASK_020_2026-08-02.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

TASK-020 stayed aligned with PLANS Step 6, SPEC-002 v22, ADR-018, and the
versioned Contracts/Ports/Postgres Store constitutions. It makes physical
PostgreSQL durability real without confusing it with Ledger, Registry, Lease,
Approval, Artifact, Guardian, provider/product, release, or production
authority.

| Gate | Current classification |
|---|---|
| Local Rust/Node format, tests, strict lint, audit, and migration hashes | machine-enforced for this run |
| Disposable PostgreSQL transaction/concurrency/fault/restart evidence | machine-enforced for the exact marker-owned target |
| Fixed runtime functions, direct-table denial, roles, ACLs, and catalog shape | machine-enforced locally plus independent review |
| One Gateway/Truth/Writer and dependency/domain boundaries | independently reviewed and locally scanned |
| TASK-020 ticket allowlist | documented plus local scan; no per-ticket committed diff |
| Remote Rust/PostgreSQL CI, branch protection, remote synchronization | missing/unverified |
| Primary-branch merge readiness | blocked; no committed candidate, remote, or authorization |

## Git And Scope State

- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Primary/V2 committed ahead/behind: `0/4`; remote/upstream: none.
- MVP-0 through TASK-020 remain one inspectable cumulative uncommitted result.
- No reset, clean, branch switch, commit, push, merge, production database or
  credential mutation, publication, deployment, deletion, or protected action
  occurred.

## Next Bounded Slice

TASK-021 must first freeze the durable Task Ledger adapter boundary before any
code change. It will add one domain-owned composition layer that persists the
canonical Task Ledger request, event/head, terminal command receipt, and
outbox admission atomically in PostgreSQL, reuses the pure Ledger verifier as
the semantic owner, and proves idempotency, concurrency, corruption denial,
unknown-commit reconciliation, and restart replay. It must not give the pure
domain crate I/O, make Postgres Store depend on a domain, grant runtime generic
SQL, or introduce OpenClaw/Codex/Graphify/Hermes/provider/product/release work.

---

# Archived TASK-019 Handoff

## Outcome

TASK-019 Postgres Store 1.1.5 is complete for its exact-manifest PostgreSQL
17 schema, permission, compatibility, and STOPPED-admission foundation.
SPEC-002 AC-33 is complete. This does not complete a live `ControlStore`,
durable domain repositories, AC-03/04/05/19, MVP-1, MVP-2, MVP-3, or the whole
platform.

LATTICE remains a general local AI development platform for the user's
computer:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated companion/playmate website is not part of this product,
architecture, roadmap, implementation, or test target.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-019 is complete.
- MVP-2 exact-version local components and Codebase Memory: planned after the
  MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2 gate.

## Completed In TASK-019

- Pinned the synchronous `postgres` 0.19.14 driver with default features off
  and SHA-256 0.11.0 while retaining the Contracts-only Ports boundary.
- Added a compile-time exact migration manifest. The fixed `0001` remains
  byte-identical and `SUPERSEDED`; transaction-control-free `0002` creates only
  database identity/history/compatibility, physical transaction foundations,
  and STOPPED/no-leader admission.
- Added the explicit administrative runner and read-only repeatable-read
  verifier with exact target sentinel, transaction-scoped concurrency lock,
  no-op retry, uncertain/committed-unverified reconciliation, and full catalog
  drift closure.
- Enforced real LOGIN-to-NOLOGIN capability separation, CONNECT-only bootstrap,
  cluster-wide ACL/default-ACL/ownership closure, exact protected-function
  denial, `max_prepared_transactions = 0`, and no notification authority.
- Added a marker-owned PostgreSQL 17.10 harness that uses a fresh non-5432
  loopback cluster, leaves the installed service untouched, restarts its own
  cluster, proves real LOGIN permissions, and deletes only its exact root after
  stopped-state and marker verification.
- Kept the deterministic fake and `ControlStore` unchanged; no live/durable
  receipt, domain write, self-activation, production credential, provider,
  product, or website behavior was added.

## Verification Evidence

- Postgres Store focused tests: 35/35.
- Full Rust workspace: 401/401.
- Preserved Node suite: 44/44.
- Format and strict all-target/all-feature Clippy pass with zero warnings.
- Two fresh PostgreSQL 17.10 trials report
  `TASK019_HARNESS_SELF_TEST=PASS` and `TASK019_POSTGRES_HARNESS=PASS` for both
  initial and restart phases.
- PowerShell AST, dependency tree, duplicate dependency, migration hash,
  debug-marker, temporary-artifact, diff, and governance checks pass.
- Independent code/security and architecture reviews pass with P0-P3 all zero;
  local combined integration passes.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_019_2026-08-01.md`
- `docs/reviews/CODE_REVIEW_TASK_019_2026-08-01.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_019_2026-08-01.md`
- `docs/reviews/INTEGRATION_TASK_019_2026-08-01.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

TASK-019 stayed aligned with PLANS, ADR-017, and Postgres Store 1.1.5. It made
PostgreSQL evidence real without confusing a schema foundation with a live
Store, domain truth, leader activation, or production release.

| Gate | Current classification |
|---|---|
| Manifest, runner, verifier, role/catalog/settings and harness behavior | machine-enforced locally |
| Disposable PostgreSQL transaction/concurrency/restart/permission evidence | machine-enforced locally twice |
| One Gateway/Truth/Writer and dependency boundaries | independently reviewed and locally scanned |
| Live physical Store and durable/domain receipts | missing/deferred to TASK-020+ |
| `cargo-audit`, remote Rust CI, branch protection | unavailable or missing/unverified |
| Merge readiness | blocked; no committed candidate, remote, or authorization |

## Git And Scope State

- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Primary/V2 committed ahead/behind: `0/4`; remote/upstream: none.
- MVP-0 through TASK-019 remain one inspectable uncommitted dirty result.
- No reset, clean, branch switch, commit, push, merge, production database or
  credential mutation, publication, deployment, deletion, or protected action
  occurred.

## Next Bounded Slice

TASK-020 will version the affected contracts before implementing the live
physical PostgreSQL `ControlStore`. It must revalidate exact ACTIVE daemon
authority and physical head in the same transaction, retain exact terminal
receipts for reconciliation, expose only narrow runtime operations, and keep
all Registry/Ledger/Lease/Approval/Artifact legality and Guardian activation
outside this slice.

---

# Archived TASK-018 Handoff

## Outcome

TASK-018 Postgres Store 1.0 is complete for its typed zero-I/O MVP-1 boundary.
SPEC-002 AC-32 is complete. This does not complete durable PostgreSQL
AC-03/04/05/19, MVP-1, MVP-2, MVP-3, or the whole platform.

LATTICE remains a general local AI development platform for the user's
computer:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated playmate website is not part of this product, architecture, or
roadmap. Ten old `ACCESS_PLAYMATE` strings remain only as explicit V1
compatibility/denial fixtures; active V2 and TASK-018 paths have zero coupling.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-018 is complete.
- MVP-2 exact-version local components and Codebase Memory: planned after the
  MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2 gate.

## Completed In TASK-018

- Contracts 1.8 adds bounded canonical Store transaction, daemon, scope,
  authority, physical-head, commitment, disposition, and receipt values.
- Ports 1.3 exposes only typed `ControlStore::transact` and `current_head`
  through Store-specific failures and remains Contracts-only.
- Postgres Store 1.0 implements a deterministic in-memory fake with Store-owned
  genesis/head hashes, complete request hashing, exact replay, changed-ID
  substitution denial, project/snapshot/owner/aggregate isolation, atomic head
  and receipt apply, stable stale denial, bounded capacity/serialization, and
  explicit before/after-apply fault reconciliation.
- Every receipt is fixed to `RuntimeKind::Fake` and `NonDurableFake`; no driver,
  SQL, connection, migration runner, or durable constructor exists.
- Project governance now rejects duplicate ticket IDs, invalid current-marker
  cardinality, a marker without its unique ticket, a current ticket without its
  module constitution, non-canonical constitution paths, and duplicate module
  IDs without forcing future modules active early.

## Review Repairs

Independent review found and closed bounded/canonical snapshot identity,
replay-integrity ordering, changed-ID substituted-scope probing, arbitrary
physical-head injection, revision-zero genesis override, canonical constitution
path, and stale-disposition documentation drift. Each behavioral finding
received a RED/GREEN regression. Final code/security and architecture reviews
pass with P0=0, P1=0, P2=0, P3=0.

## Verification Evidence

- Focused locked package tests: Contracts 42, Ports 5, Store 14 (61 total).
- Full locked Rust workspace: 380/380.
- Preserved Node verification: 44/44.
- Format, strict workspace Clippy, dependency tree, forbidden driver, scoped
  I/O/SQL/credential/provider/product/website, migration inactivity,
  governance, and diff/untracked hygiene checks pass.
- Migration unchanged: SHA-256
  `7BFF021FC17F738551309C906578C8015B2DD0307D27D239C21DF1697C4D09C8`,
  Git blob `5c1bb61e220980b2087d4ec7a3c61a50a9d23ec5`.
- Independent local combined integration: `PASS`.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_018_2026-08-01.md`
- `docs/reviews/CODE_REVIEW_TASK_018_2026-08-01.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_018_2026-08-01.md`
- `docs/reviews/INTEGRATION_TASK_018_2026-08-01.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

TASK-018 stayed aligned with PLANS and ADR-016: it froze the physical
transaction boundary before real database work and did not duplicate domain
legality or durable truth.

| Gate | Current classification |
|---|---|
| Store contract/fake scope, hashing, atomicity, replay, faults | machine-enforced locally |
| Dependency/no-I/O/driver/migration inactivity | locally tested and scanned |
| Current-ticket/constitution governance | machine-enforced locally |
| PostgreSQL durability, restart/concurrency, roles/time/admission | missing/deferred to TASK-019+ |
| Remote Rust CI and branch protection | missing/unverified |
| Merge readiness | blocked; no committed candidate, remote, or authorization |

## Git And Scope State

- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Primary/V2 committed ahead/behind: `0/4`; remote/upstream: none.
- MVP-0 through TASK-018 remain one inspectable uncommitted dirty result.
- No reset, clean, branch switch, commit, push, merge, database mutation,
  publication, deployment, credential/account/payment change, deletion, or
  protected action occurred.

## Next Bounded Slice

`PLANS.md` marks `CURRENT TASK-019 GOVERNANCE`. SPEC-002 v16, ADR-017,
Postgres Store 1.1, and TASK-019 now freeze the exact-manifest PostgreSQL 17
schema/admission foundation. The bounded implementation adds a pinned
synchronous driver, an explicit administrative runner, read-only runtime
verifier, STOPPED/no-leader bootstrap, role separation, and a marker-owned
disposable 17.10 cluster. It does not add a live `ControlStore`, durable
receipt, production credential/role/database change, daemon activation,
public exposure, or unrelated website work.

## Restart Context

- Overall goal remains active through MVP-3; do not mark it complete.
- Current MVP: MVP-1 offline control core.
- Completed ticket: `docs/tickets/TASK-018-postgres-store-boundary.md`.
- Current ticket:
  `docs/tickets/TASK-019-postgres-manifest-admission-foundation.md`.
- Continue bounded reversible local work automatically; protected actions and
  primary merge remain fail-closed.

---

# Archived LATTICE DevOS TASK-017 Handoff

## Outcome

TASK-017 Gateway IPC 1.1 / wire protocol 1.0 is complete for its bounded
pure/fake MVP-1 scope. SPEC-002 AC-31 is complete. This does not complete the
live portion of AC-07, MVP-1, MVP-2, MVP-3, or the whole platform.

LATTICE remains a general local AI development platform for the user's
computer:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated playmate website is not part of this repository, architecture,
tests, or roadmap.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-017 is complete.
- MVP-2 exact-version local OpenClaw/Codex/Graphify/Hermes plus Codebase Memory:
  planned after the MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2
  authority/containment gate.

No routine human decision blocks TASK-018. Credentials, account/payment
changes, public exposure/publication/deployment, irreversible real effects,
security-control changes, protected release activation, and primary-branch
merge remain separately protected.

## Completed In TASK-017

### Contracts 1.7 And Ports 1.2

- Added neutral bounded peer/request/reply values for exactly Submit, Plan,
  Status, normal Approve/Reject, and task Stop. Actions derive from closed typed
  bodies; no arbitrary action, SQL, shell, path, provider, daemon, or release
  escape hatch exists.
- Bounded gateway-reused task/snapshot/attempt identifiers to 256 bytes and
  rejected all-zero authority/freshness/receipt/observation/terminal digests.
- Bound reply action, request, command, correlation, subject, page size,
  disposition, and evidence before canonical hashing.
- Changed `GatewayService` to accept server-derived peer context and a complete
  request, returning a bound reply through component-free
  `GatewayServiceError`; Rust-core failures can no longer be mislabeled as
  OpenClaw failures.

### Gateway IPC 1.1 / Wire Protocol 1.0

- Added a strict canonical JSON codec with a raw 1 MiB frame cap, depth 32,
  node 8,192, array 256, no numbers, exact fields, duplicate/unknown/version/
  action rejection, complete trailing-data checks, and redacted bounded errors.
- Added allocation-free NFC preflight for values and keys. Non-NFC request and
  reply identities, including normalization-expanding exact-bound inputs, fail
  before canonical hashing/allocation, replay insertion, or service dispatch.
- Added domain-separated request/reply digests and mechanical Task Spec 2.1
  canonical-document digest/binding verification without copying Task Domain
  semantics or exposing raw source.
- Added a pure in-memory fake client/server. Role authorization precedes replay;
  exact `(project, actor, command)` retry returns identical terminal bytes;
  changed content denies without another service call. Replay storage is capped
  at 1,024 entries while retained exact retries remain readable.
- Recovery can route only bounded Status and task Stop. ProtectedChange routes
  normally; raw `PROTECTED_RELEASE` is unrepresentable and rejected by the
  codec before service dispatch.
- Project status observes at most both the request page size and global 100-
  item cap. Stop preserves REQUESTED, ALREADY_TERMINAL, and
  RECONCILIATION_REQUIRED without claiming process interruption or completion.

### Repository Governance Repair

- `scripts/check-project.mjs` now rejects duplicate `ticket_id` values and any
  `PLANS.md` state with other than one `CURRENT TASK-nnn` marker.
- Three disposable Node regressions prove duplicate denial, multiple-marker
  denial, and the valid unique/single-marker case.
- Contracts owns neutral in-process representations and constructor-level
  identifier/cursor/page bounds. Gateway IPC owns wire layout, parser/encoded-
  frame limits, NFC enforcement, hash subjects, and replay. SPEC v14, ADR-015,
  module constitutions, routing index, ticket, and plan agree on that split.

## Review Repairs

The initial independent review found nine blockers: replay before role
authorization, zero digest sentinels, reply hashing before bounds, oversized
reused IDs, ignored request page size, unbounded replay storage, contradictory
protected semantics, incomplete reply/substitution matrices, and duplicate
TASK-017 tickets.

Final review additionally found and closed non-NFC identity/size expansion,
typed-encoder fast-fail ordering, false external component attribution for
Rust-core errors, Contracts/wire ownership drift, dependency/version drift,
and missing machine enforcement for ticket/current-marker uniqueness.

Every accepted behavioral finding received a failing regression before repair.
Final independent code/security and architecture reviews report `PASS`, with
P0=0, P1=0, P2=0, and P3=0.

## Files Added Or Materially Changed

- Gateway code/tests: `crates/lattice-gateway-ipc/**`.
- Shared interfaces/tests: `crates/lattice-contracts/{src,tests}` and
  `crates/lattice-ports/{src,tests}`.
- Workspace dependency graph: `Cargo.toml`, `Cargo.lock`, and Gateway manifest.
- Machine governance: `scripts/check-project.mjs` and
  `test/project-governance-check.test.js`.
- Governance/delivery: SPEC-002 v14, ADR-015, Gateway/Contracts/Ports and
  related module documents, TASK-017, PLANS, workflow audit, final review and
  integration reports, workflow ledger, and this handoff.

## Verification Evidence

All final local commands completed successfully:

- focused locked suites: Contracts 36, Gateway IPC 31, Ports 3 (70 total).
- `cargo test --workspace --locked`: 358 Rust tests.
- strict workspace Clippy, all targets/features, locked, `-D warnings`.
- `cargo fmt --all -- --check`.
- `npm.cmd run verify`: 41 Node tests; project check reports 221 files,
  17 constitutions, 17 unique tickets, and one current task.
- Gateway Cargo tree: only Contracts, Ports, cjson, exact serde/serde_json, and
  exact Unicode normalization plus approved transitives.
- forbidden filesystem/network/process/database/Git/provider/product and
  unrelated-website scans: zero scoped implementation matches.
- `git diff --check`: pass.
- independent code/security and architecture review: `PASS`, zero P0-P3.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_017_2026-08-01.md`
- `docs/reviews/CODE_REVIEW_TASK_017_2026-08-01.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_017_2026-08-01.md`
- `docs/reviews/INTEGRATION_TASK_017_2026-08-01.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

The work stayed aligned with PLANS Step 5 and the MVP-1 goal: the gateway
protocol is frozen before live transport, PostgreSQL, Codex, Graphify, Hermes,
or Codebase Memory composition.

| Gate | Current classification |
|---|---|
| Pure codec/fake action, binding, role, retry, limit, and fault behavior | machine-enforced locally |
| Project/actor/command isolation and bounded replay | machine-enforced locally |
| Unique ticket IDs and exactly one current task | machine-enforced locally |
| Dependency/no-I/O direction | locally linted and inspected |
| Contracts/wire ownership and fake/live boundary | documented plus structurally checked |
| Live OpenClaw transport, ACL, peer identity, restart, compatibility | missing/deferred under AC-07 |
| PostgreSQL durability and composed One Truth | missing/deferred |
| Remote Rust CI and branch protection | missing/unverified |
| Merge readiness | blocked; no committed candidate, remote, or authorization |

## Git And Scope State

- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD/base: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Primary/V2 committed ahead/behind: `0/4`; remote/upstream: none.
- MVP-0 through TASK-017 remain one inspectable uncommitted dirty result.
- TASK-017-identifiable paths fit its final allowlist; exact shared-file
  per-ticket diff is partial/documented-only because prior V2 work is also
  uncommitted.
- No reset, clean, branch switch, commit, push, merge, installation, live
  database mutation, publication, deployment, credential/account/payment
  change, real delete, or protected action occurred.

## Incomplete And Explicitly Open

- AC-07 remains open for real OpenClaw package/schema/binary, OS-local
  transport/ACL/peer authentication, session restart/disconnect, and durable
  terminal receipt evidence.
- TASK-018 through TASK-031 remain for PostgreSQL stores, filesystem/Git/scope,
  review and Codex fakes, offline end-to-end composition, compatibility, and
  the MVP-1 exit gate.
- Exact-version live OpenClaw, Codex, Graphify, Hermes, and Codebase Memory
  remain MVP-2.
- Guardian-protected improvement, A/B activation, canary, and rollback remain
  MVP-3.

## Next Bounded Slice

`PLANS.md` marks `CURRENT TASK-018 GOVERNANCE`.

TASK-018 must freeze a typed, zero-I/O Postgres Store 1.0 boundary and
deterministic fake before any database connection:

1. re-audit TASK-017 closure and current repository/Git state;
2. define transaction request/result/error and exact project/authority/
   idempotency binding without copying domain legality;
3. keep PostgreSQL as the future sole durable writer/truth while the TASK-018
   fake remains visibly non-durable and performs no database I/O;
4. freeze migration ownership and compatibility boundaries for TASK-019;
5. update SPEC/ADR/module constitution/ticket before RED tests;
6. repeat TDD, focused/full verification, independent reviews, integration,
   ledger, and handoff.

## Restart Context

- Overall goal remains active through MVP-3; do not mark it complete.
- Current MVP: MVP-1 offline control core.
- Current marker: `CURRENT TASK-018 GOVERNANCE` in `PLANS.md`.
- Completed ticket: `docs/tickets/TASK-017-gateway-ipc.md`.
- Continue bounded, reversible local work automatically; do not introduce the
  unrelated playmate website.

---

# Archived LATTICE DevOS TASK-016 Handoff

## Outcome

TASK-016 Artifact Store 1.0 is complete for its bounded pure/fake MVP-1 scope.
SPEC-002 AC-30 is complete. This is not completion of AC-19, MVP-1, MVP-2,
MVP-3, or the full platform.

The product remains a general local AI development platform for the user's
computer:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated playmate website is not part of this repository, architecture,
tests, or roadmap.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-016 is complete.
- MVP-2 exact-version local OpenClaw/Codex/Graphify/Hermes plus Codebase Memory:
  planned after the MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2
  authority/containment gate.

No routine human decision blocks the next bounded local ticket. Credentials,
account/payment changes, public exposure/publication/deployment, irreversible
real effects, security-control changes, protected release activation, and
primary-branch merge remain separately protected.

## Completed In TASK-016

### Contracts 1.6

- Added neutral immutable project-scoped object/generation, provenance,
  reference, read, sweep, availability/delete, fixed owner receipt, and full
  current-head representations.
- Enforced positive signed-BIGINT-safe counters, exact SHA-256 identity,
  canonical time, runtime/producer/status/action closure, and complete binding
  validation.

### Artifact Store 1.0

- Added one public `FakeArtifactStore` owner. Lifecycle, history, quotas,
  staging, bytes, and terminal maps are composed atomically behind it; lower
  mechanisms cannot be a public second writer.
- Verified length and SHA-256 before publication. Raw bytes remain confined to
  a separately redacted in-memory fake backend and never enter retained
  requests, receipts, snapshots, checkpoints, errors, or `Debug`.
- Implemented project-isolated content generations, immutable per-use
  references, complete provenance, typed fixed-owner current authority,
  release terminality, active/suspect read lifecycle, safe delete planning,
  exact claim token, unknown-outcome reconciliation, and higher-generation
  reintroduction.
- Implemented exact applied and denied command retry before stale/time checks;
  changed content under one scoped command key rejects permanently.
- Enforced hard and configured byte/manifest/field/bundle/object/reference/
  read/staging/command/history limits at object, task, project, and store
  scopes. Task object/active-byte attribution is active-reference-only;
  retained project/store capacity and worst-case claimed/reconciliation/orphan
  capacity remain held until verified terminal evidence.
- Included holder IDs, complete persisted lifecycle strings, and the 64-byte
  domain-separated delete claim token in `FieldBytes` accounting.
- Added strict raw snapshots containing complete sanitized terminal lifecycle
  receipts. Context-free replay reconstructs lifecycle, history, quotas,
  staging, command tasks, retired scopes, and terminals, validates all
  digests/joins, and then compares an independent compact checkpoint.
- The checkpoint retains only store/limit/snapshot/replay-bound/trust-anchor
  commitments; it contains neither an owner clone, metadata row set, nor
  payload. Untrusted canonical size is preflighted before allocation, including
  control-character escape expansion.

### Review Repairs

Independent review found and closed:

- checkpoint construction that temporarily copied payload bytes;
- canonical-byte bounds checked after output allocation;
- replay that returned a trusted owner clone instead of rebuilding raw input;
- missing full lifecycle receipts needed for context-free exact retry;
- missing holder/lifecycle/claim-token `FieldBytes` projection;
- direct applied-after-replay retry evidence.

Each accepted behavioral finding received a failing regression before repair.
Final code/security and architecture re-reviews report `PASS`, with P0=0,
P1=0, P2=0, and P3=0.

## Files Added Or Materially Changed

- Workspace/contracts: `Cargo.toml`, `Cargo.lock`,
  `crates/lattice-contracts/src/lib.rs`, and Contracts tests.
- Artifact Store owner and mechanics:
  `crates/lattice-artifact-store/src/{lib,aggregate,history,quota,quota_owner,semantics,snapshot}.rs`.
- Strict context-free restore:
  `src/aggregate/snapshot_restore.rs`, `src/semantics/snapshot_restore.rs`,
  `src/snapshot_parse.rs`, `src/snapshot_contract.rs`, and
  `src/snapshot_quota.rs` under the Artifact Store crate.
- Artifact Store behavior suites under
  `crates/lattice-artifact-store/tests/`, including owner, delete, read, quota,
  staging, history, lifecycle, and replay matrices.
- Governance/delivery: SPEC-002 v12, ADR-014, Artifact Store and Contracts
  constitutions, TASK-016, PLANS, workflow audit, final code/security review,
  final architecture review, integration report, workflow ledger, and this
  handoff.

## Verification Evidence

All commands below completed successfully:

- `cargo test -p lattice-contracts --locked`: 32 tests.
- `cargo test -p lattice-artifact-store --locked`: 97 tests.
- `cargo test -p lattice-artifact-store --test artifact_owner_replay --locked`:
  8 tests.
- `cargo test --workspace --all-targets --all-features --locked`: 322 Rust
  tests.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.
- `cargo fmt --all -- --check`.
- `npm.cmd run verify`: `check=ok`, 38 preserved Node tests.
- `cargo tree -p lattice-artifact-store --edges normal --locked`: only
  Contracts, cjson, SHA-256, time, and approved transitives.
- forbidden-I/O, provider/product dependency, and unrelated-website scans:
  zero scoped implementation/dependency matches.
- payload fixture containment assertions: raw snapshot, checkpoint, and owner
  debug contain no fixture bytes; replayed verified read reports `MissingBytes`.
- `git diff --check`: pass.

Verification artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_016_2026-07-30.md`
- `docs/reviews/CODE_REVIEW_TASK_016_2026-07-30.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_016_2026-07-30.md`
- `docs/reviews/INTEGRATION_TASK_016_2026-07-30.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Workflow Ledger

| Stage | Status | Evidence |
|---|---|---|
| Inspect repository/Git | valid | workflow audit, branch/base/status |
| Clarify decisions | valid | SPEC v12 and ADR-014 |
| Specification | valid | AC-30 complete; durable/live AC-19 remains open |
| Module governance | valid | Artifact Store 1.0 and Contracts 1.6 |
| Ticket decomposition | valid | bounded non-parallel TASK-016 |
| Branch/worktree plan | valid | dirty V2 worktree preserved; V1 untouched |
| TDD implementation | valid | RED/GREEN behavior and review regressions |
| Focused/full verification | valid | 32 Contracts, 97 Artifact, 322 Rust, 38 Node |
| Code/security review | pass | zero remaining P0-P3 |
| Architecture review | pass | zero P0-P3; no amendment |
| Local integration | pass/partial | combined result passes; no committed candidate |
| Remote CI/merge | missing/blocked | no remote, CI, branch protection, candidate, or authorization |

## Alignment And Enforcement Truth

The work stayed aligned with PLANS Step 5 and the overall MVP-1 goal: one pure
artifact authority was frozen before PostgreSQL, filesystem effects, OpenClaw,
Codex, Graphify, Hermes, or Codebase Memory consumes it.

| Gate | Current classification |
|---|---|
| Pure artifact authority/quota/retry/replay/checkpoint behavior | machine-enforced locally |
| Project isolation and fixed owner/runtime contracts | machine-enforced locally |
| Dependency/no-I/O direction | locally linted and inspected |
| Governance semantics and ownership | documented plus structurally checked |
| PostgreSQL transactions/durability/restart | missing/deferred under AC-19 |
| Real filesystem containment/staging/delete | missing/deferred |
| Live provider/authority authentication | missing/deferred |
| One Gateway/One Truth/One Writer at composed runtime | documented-only in this slice |
| Remote Rust CI and branch protection | missing/unverified |
| Merge readiness | blocked; no committed candidate or authorization |

## Git And Scope State

- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- HEAD/base: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Primary branch: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`
- Primary/V2 committed ahead/behind: `0/4`
- Remote/upstream: none.
- The shared MVP-0 through TASK-016 result remains uncommitted and dirty.
- Identifiable TASK-016 paths fit its allowlist; exact per-ticket shared-file
  scope is `partial/documented-only` because prior V2 work is also uncommitted.
- No reset, clean, branch switch, commit, push, merge, installation,
  publication, deployment, credential/account/payment change, PostgreSQL
  mutation, real delete, or protected action occurred.

## Incomplete And Explicitly Open

- AC-19 remains open for PostgreSQL metadata/reference transactions,
  serialization, durability, restart, and same-transaction effect admission.
- Real filesystem staging/flush/rename/link containment/read/delete and orphan
  reconciliation remain open.
- Fake OpenClaw IPC, remaining PostgreSQL stores, workspace/scope enforcement,
  fake Codex/reviewer, offline end-to-end orchestration, and the MVP-1 exit gate
  remain incomplete.
- Exact-version OpenClaw, Codex, Graphify, Hermes, and Codebase Memory remain
  MVP-2.
- Guardian-protected improvement/activation/rollback remains MVP-3.

## Next Bounded Slice

At TASK-016 closure, `PLANS.md` then marked `CURRENT TASK-017 GOVERNANCE`;
TASK-017 is now complete as recorded at the top of this file.

TASK-017 should freeze and implement a pure/fake OpenClaw IPC boundary without
installing or invoking OpenClaw:

1. re-audit TASK-016 closure and confirm the slice still serves MVP-1;
2. define the only normal typed gateway actions for task submission, status,
   approval routing, and stop;
3. keep the CLI as a recovery/test client over the same contract, not another
   normal gateway;
4. ensure IPC grants no direct PostgreSQL/Git/provider/credential/protected-
   release authority and cannot own a Codex writer thread;
5. update SPEC/ADR/module constitution/ticket before implementation;
6. repeat TDD, focused/full verification, independent reviews, integration
   report, ledger, and handoff.

## Restart Context

- Overall goal remains active through MVP-3; do not mark it complete.
- Current MVP: MVP-1 offline control core.
- Historical next marker at TASK-016 closure: `CURRENT TASK-017 GOVERNANCE`.
- Completed ticket: `docs/tickets/TASK-016-artifact-store.md`.
- First checks:
  - `git status --short --branch`
  - `git rev-parse HEAD`
  - `cargo test --workspace --all-targets --all-features --locked`
  - `npm.cmd run verify`
- Continue bounded, reversible local work automatically.
- Do not introduce the unrelated playmate website.
