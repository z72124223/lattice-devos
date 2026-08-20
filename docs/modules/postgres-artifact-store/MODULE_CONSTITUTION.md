---
module_id: postgres-artifact-store
name: PostgreSQL Artifact Store Adapter
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-20
---

## Mission

Persist complete Artifact Store metadata snapshots, immutable transition
receipts and independent checkpoints through fixed serializable PostgreSQL
operations while delegating every semantic decision to Artifact Store 1.1.

## Non-Goals

- Decide identity, provenance, quota, staging, retention, read or delete
  legality.
- Store, read, name, scan, publish, quarantine or unlink artifact bytes.
- Accept caller SQL, DSN, credentials, table names, observation time or
  authority Booleans.
- Mutate Registry, Ledger, Policy, Approval, Writer Lease or product data.

## Owned Data

- Exact extension identity and append-only install ledger.
- Per-store untrusted canonical metadata snapshots and byte hashes.
- Independently retained limit, snapshot and checkpoint commitments.
- Immutable successful compare-and-swap transition rows.

## Public Contracts

- Implement only Artifact Store's component-free `ArtifactRepository` trait.
- Load returns state only after bounded pure replay against retained checkpoint.
- Compare-and-swap uses one fixed serializable transaction and exact expected
  checkpoint; a stale writer changes nothing. Initial state must be the exact
  vacant owner and every non-retry write must retain all prior receipts while
  adding exactly one aggregate-committed command.

## Invariants

1. Snapshot rows are untrusted until canonical-byte, SHA-256, pure replay and
   independent checkpoint comparison all pass.
2. SQL checks physical row shape and expected versions only; it never creates
   an Artifact receipt, head, quota delta or lifecycle outcome.
3. Commit-response uncertainty returns no success and requires a fresh load or
   exact retry.
4. Runtime has fixed function execution only and no table DML.
5. Raw artifact bytes, caller paths, secrets and credentials never enter this
   schema, diagnostics or public inputs.
6. Exact catalog, function body, constraint, index, type and explicit/effective
   ACL signatures must match PostgreSQL 17.10; extra namespace objects fail
   closed rather than being repaired.
7. Every load cross-checks the physical transition count, contiguous
   predecessor chain and final snapshot/checkpoint hashes against the head.
8. This metadata adapter grants no live provenance authority. Registry,
   effect, daemon and capability-owner currentness must be composed by their
   owners in a later shared transaction; no Boolean substitute is accepted.

## Allowed Dependencies

- Rust standard library.
- `lattice-artifact-store` 1.1 and immutable `lattice-contracts` values.
- Exact pinned `postgres` and `sha2` crates.
- Embedded `db/extensions/artifact-store/v1.sql` bytes.

## Forbidden Dependencies

Filesystem adapters, provider adapters, product repositories, Policy, Ledger,
Registry, Approval, Writer Lease, Orchestrator, CLI and arbitrary SQL clients.

## Failure, Compatibility, And Migration

Partial, extra, drifted, wrong-owner/ACL/function/identity/ledger, corrupt,
rollback, substituted, stale, serialization-exhausted or ambiguous state fails
closed without automatic repair or deletion. Future versions are append-only.

## Acceptance Gates

Focused API/static tests, marker-owned PostgreSQL install/no-op/CAS/concurrency/
corruption/restart/ACL tests, strict Clippy, format and repository checks.

## Change Policy

Schema, ACL, function, snapshot/checkpoint, CAS, retry, ambiguity or dependency
changes require a versioned constitution and SPEC/ADR trace.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-20 | SPEC-002 v37, ADR-025, TASK-025 | Durable Artifact metadata snapshots, checkpoints and serializable CAS | User TASK-023-025 development directive |
