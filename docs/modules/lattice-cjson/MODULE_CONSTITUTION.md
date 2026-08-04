---
module_id: lattice-cjson
name: LATTICE Canonical JSON
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Provide the single pure Rust implementation of the deterministic
`lattice-cjson-1` value-to-byte algorithm and its schema-domain-separated
SHA-256 framing.

## Non-Goals

- Choose Task Spec, event, approval, memory, or release hash subjects.
- Parse arbitrary wire JSON or silently coerce raw JSON numbers.
- Own task/event semantics, persistence, policy, authorization, or I/O.
- Reproduce the historical V1 JavaScript canonicalizer.

## Owned Data

- The `lattice-cjson-1` algorithm identifier and typed canonical value model.
- Unicode NFC normalization, object-key ordering, minimal escaping, and
  duplicate-normalized-key rejection.
- The generic `lattice-hash-1` schema ID/version domain-separation frame and
  SHA-256 digest representation.

Task Domain owns which Task Spec fields enter `spec_hash`. Task Ledger owns
event/receipt subject selection, predecessor/event hashes, and replay. Those
modules consume this mechanical algorithm without transferring semantic
ownership.

## Public Contracts

- Canonicalize only null, Boolean, string, array, and object values; no raw
  integer or floating-point variant exists.
- Normalize keys and string values to Unicode NFC.
- Sort object keys by normalized UTF-8 bytes and reject duplicate keys after
  normalization.
- Preserve array order and distinguish an absent object field from explicit
  `null`.
- Emit UTF-8 JSON with no insignificant whitespace and minimal escaping.
- Hash the exact length-prefixed `lattice-hash-1` frame frozen by ADR-008.
- Reject empty or oversized schema identifiers/versions and oversized
  canonical payload lengths before framing.

## Invariants

1. The same accepted typed value and domain always produce identical bytes and
   digest.
2. Object insertion order never changes bytes; array order does.
3. NFC-equivalent strings produce identical bytes; NFC-equivalent keys in one
   object are a hard error.
4. Schema ID or schema version changes the framed bytes and digest.
5. Raw JSON numbers cannot enter the algorithm. Schema modules must encode
   validated integer/decimal values as normalized decimal strings.
6. The implementation performs no filesystem, database, network, process,
   clock, randomness, or provider I/O.
7. V1 and V2 hashes never share an unversioned code path.

## Allowed Dependencies

- Rust standard library.
- Exact-version `unicode-normalization` 0.1.25 for Unicode NFC.
- Exact-version RustCrypto `sha2` 0.11.0 for SHA-256.

## Forbidden Dependencies

- `lattice-contracts`, task/policy/ledger modules, ports, adapters, Serde JSON,
  database/network/process clients, operating-system services, credentials, or
  product repositories.

## Failure, Compatibility, And Migration

Canonicalization returns typed errors and never emits partial success. Changing
accepted value kinds, NFC behavior, key ordering, escaping, framing bytes, or
digest algorithm requires a new canonical algorithm identifier and golden
fixtures; it cannot silently alter `lattice-cjson-1`.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Golden bytes | Unicode, order, escape, null/missing, array, and framing fixtures | Engineering | yes |
| Collision denial | NFC-equivalent duplicate keys reject | Engineering | yes |
| Digest separation | schema/version and V1/V2 separation fixtures | Engineering | yes |
| Dependency inspection | only exact approved pure Rust crates | Architecture review | yes |
| Full Rust verification | workspace format, lint, and tests | Engineering | yes |

## Change Policy

Algorithm semantics, framing, digest, public value kinds, dependency direction,
or limits require a versioned constitution amendment, SPEC/ADR trace,
architecture review, and responsible-user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-002 v4, ADR-005/008, TASK-010 | Pure shared canonical-byte mechanism with semantic ownership retained by callers | User execution directive |
