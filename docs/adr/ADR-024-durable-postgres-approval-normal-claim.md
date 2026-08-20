# ADR-024: Durable PostgreSQL Approval repository and normal effect claim

- Status: accepted
- Date: 2026-08-20
- Decision owners: LATTICE maintainers
- Related: SPEC-002 v36, ADR-013, TASK-024, Approval Verifier 1.1,
  Postgres Approval Verifier 1.0

## Context

Approval Verifier 1.0 already owns typed subjects, fake challenges/proofs,
global nonce binding, time windows, availability, revocation, exact command
receipts, replay, checkpoints, normal consume planning, and the absence of a
general protected consume command. It deliberately performs no I/O. TASK-024
must make those semantics durable and make a normal approval claim atomic with
one exact transition/effect claim without turning SQL into a second verifier.

The current durable platform uses independent, checksummed PostgreSQL
extensions with fixed runtime functions, serializable transactions,
independently retained checkpoints, closed ACLs, database-owned observations,
and exact retry after ambiguous commits. The Approval adapter must follow that
physical convention while preserving every Approval Verifier 1.0 byte and
meaning.

## Decision

1. Approval Verifier advances to 1.1 only to add component-free repository
   requests/errors/traits, bounded canonical snapshot/checkpoint bytes, and a
   typed normal effect-claim receipt. Existing 1.0 commands, receipts, hashes,
   fake runtime, proof algorithms, replay, and denial meaning are unchanged.
2. `lattice-postgres-approval-verifier` is the sole physical adapter. The pure
   crate never depends on PostgreSQL, Store, Ledger, Policy, Guardian, or any
   concrete adapter.
3. PostgreSQL serializes one global Approval aggregate row. This deliberately
   favors correctness over parallelism: the row lock is the single global
   nonce/currentness serialization point across approvals, projects, and
   lanes. Domain replay still decides nonce legality and every state change.
4. The durable head stores complete canonical snapshot bytes plus SHA-256 and
   independently retained command high-water, command tail, nonce-binding
   digest, and snapshot digest. Loading always verifies byte hash, parses with
   bounds, replays all commands, compares the independent checkpoint, and
   cross-checks immutable physical command/effect rows before returning state.
5. High-level repository intents omit database-owned values. Inside one
   serializable transaction, the adapter locks the aggregate, reads
   `transaction_timestamp()` and exact current `control.runtime_admission`,
   constructs the existing pure command, calls public `plan_command` and
   `apply_plan`, and persists only the resulting verified state/receipt.
6. Issue accepts a bounded TTL and binds database issue/expiry time. Verify and
   revoke bind database observation time. The proof remains visibly fake in
   TASK-024; live OS/Guardian proof adapters are explicitly outside this ADR.
   Durable PostgreSQL evidence must never be described as live authentication.
7. Normal claim accepts one typed effect kind, effect identity, and effect
   digest. The adapter derives the domain claim digest from that intent plus
   database observation, current daemon/epoch/admission, approval/head, and the
   resulting consume receipt. The aggregate update, command row, and immutable
   effect-claim row commit together or not at all.
8. Exact retry is checked against canonical repository-intent bytes before a
   new time/admission observation changes the derived command. An identical
   retry returns the stored terminal receipt/effect claim; changed command or
   effect content fails closed. Commit-response loss returns no success and is
   reconciled only by a fresh client plus the same exact intent.
9. The normal repository has no protected consume request, variant, SQL
   function, or trait method. Sending a protected head to normal claim reaches
   the existing pure `NormalClaimRequired` terminal denial and writes no effect
   claim. Guardian `claim_activation` remains a later, separate owner.
10. The extension exposes only fixed versioned load/apply/current functions to
    `lattice_runtime`; runtime receives schema usage and function execution but
    no table, sequence, generic DML, arbitrary SQL, schema selector, DSN,
    credential, raw nonce, raw key, assertion, or caller Boolean surface.

## Physical profile

The append-only v1 extension owns schema `approval_verifier` with exact
identity/ledger, one global head, immutable command receipts, and immutable
normal effect claims. All stored digests use fixed 32-byte binary columns;
versions and high-waters are positive/nonnegative non-wrapping `BIGINT`s.
Tables are owned by `lattice_migrator`, have no runtime DML grants, and are
mutated only through security-definer functions with fixed `search_path`, row
security, lock timeout, statement timeout, and explicit constraints.

Setup takes an extension advisory lock only after the existing global Store
profile is read-only verified. Fresh install and exact-current no-op are the
only accepted setup states in v1. Partial, extra, wrong-owner, wrong-ACL,
wrong-function, wrong-identity, wrong-ledger, or mixed state fails before
mutation; repair is not automatic.

## Consequences

- Global nonce/currentness safety is simple and auditable, at the cost of a
  global write serialization point. Sharding requires a later semantic ADR.
- PostgreSQL durability does not upgrade fake proof evidence into live
  authentication. Product composition must keep that distinction explicit.
- Exact normal effect claim can be composed later with the specific Task
  Ledger/Registry/effect owner; TASK-024 proves the immutable atomic claim row,
  not execution of an external effect.
- Protected claim remains impossible through the normal adapter.

## Rejected alternatives

- Per-approval rows without a global nonce serialization point: rejected
  because cross-project/lane nonce uniqueness would race.
- SQL reimplementation of subject/proof/nonce/claim legality: rejected as a
  second semantic owner.
- Caller-supplied time, daemon/admission, or `identity_verified` Boolean:
  rejected because it forges currentness/authority.
- A general claim function covering normal and protected approvals: rejected
  because it bypasses Guardian activation ordering.
- Persisting raw nonce, token, key, proof assertion, or credentials: rejected
  because the pure contract and secret boundary forbid it.

## Verification

- Approval 1.0 golden vectors remain byte-identical while 1.1 repository
  vectors prove bounds, exact retry, changed intent, and protected denial.
- Static extension tests freeze SQL bytes/manifest, tables/functions,
  constraints, owners, ACLs, and absence of generic/direct runtime DML.
- Marker-owned PostgreSQL 17.10 live tests prove install/no-op, issue/verify,
  global nonce denial, two-claimer serialization, exact normal effect claim,
  protected unchanged state, corruption rejection, commit-unknown recovery,
  fresh connection/process, restart replay, and contained cleanup.
