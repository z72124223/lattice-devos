# ADR-008: Canonical Encoding Mechanism And Semantic Ownership

- Status: accepted for TASK-010 under the user's 2026-07-29 directive to execute
  the approved LATTICE plan through MVP-3
- Date: 2026-07-29
- Decision owner: user
- Related: SPEC-002 v4/v6/v11, ADR-004, ADR-005, ADR-013, TASK-010,
  TASK-011, TASK-015

## Context

ADR-005 requires one versioned `lattice-cjson-1` algorithm. The approved module
proposal also says Task Ledger owns event canonical bytes. Implementing the
mechanism inside Task Domain would force unrelated ledger/approval/memory
modules to depend on task semantics; putting it in `lattice-contracts` would
violate that module's serialization- and hashing-free constitution; duplicating
it would permit byte drift.

## Decision

Create a pure technical Rust module and crate named `lattice-cjson`.

- `lattice-cjson` owns only typed value-to-byte canonicalization, generic
  schema-domain framing, and SHA-256 mechanics.
- Task Domain owns the exact Task Spec fields and schema domain that form
  `spec_hash`.
- Task Ledger owns event/receipt subject selection, predecessor/event hashes,
  replay, and corruption semantics.
- Approval Verifier, Codebase Memory, and the Guardian own their own subject
  semantics and may reuse only the shared byte mechanism.
- `lattice-contracts` remains free of serialization and hashing.

TASK-015 moves the complete neutral typed approval subject representation into
Contracts 1.5 so Policy and Approval Verifier compare one type graph. This does
not move semantic ownership: Approval Verifier alone selects its challenge,
proof, receipt, command, aggregate, and checkpoint hash domains and field
sets. Contracts still performs no serialization or hashing.

The exact `lattice-hash-1` frame is:

```text
ASCII "lattice-hash-1\0"
u16be(len("sha256")) || ASCII "sha256"
u16be(len("lattice-cjson-1")) || ASCII "lattice-cjson-1"
u16be(len(schema_id)) || UTF-8 schema_id
u16be(len(schema_version)) || UTF-8 schema_version
u64be(len(canonical_bytes)) || canonical_bytes
```

Schema ID and version must be non-empty, contain no NUL, and fit the unsigned
16-bit length. Canonical bytes must fit the unsigned 64-bit length. The length
framing is part of the frozen cross-language contract.

### TASK-011 Task Spec 2.1 amendment

Independent Policy review found that a currency-free
`budget.max_external_cost` allows the same immutable Task Spec to be
reinterpreted under different currencies. Before any V2 release, Task Spec
schema `2.1` therefore adds `budget.accounting_currency` as a canonical
uppercase ISO-style three-letter code and includes it in canonical bytes and
`spec_hash`. Schema `2.0` fixtures remain historical characterization only;
they are not silently re-hashed as `2.1`. MVP-1 performs no conversion.
Task Domain also owns and exports the 256-byte maximum for canonical decimal
budget strings, divided into at most 127 integer digits and 128 fractional
digits. Policy parses against those same bounds. Aligning any two valid values
therefore uses at most 255 digits, with one additional checked carry, so
construction cannot create a Task Spec that Policy rejects solely because of
divergent precision or scale limits.

## Dependency Direction

```text
lattice-cjson
  -> exact unicode-normalization
  -> exact sha2

lattice-task-domain
  -> lattice-contracts
  -> lattice-cjson
  -> exact time 0.3.54 parsing/formatting only

task-ledger/approval-verifier/codebase-memory/self-upgrade-guardian
  -> lattice-cjson mechanism
```

No dependency arrow transfers semantic data ownership. The shared mechanism is
not a truth source, writer, policy engine, or approval authority.

## Compatibility

- Historical V1 JavaScript canonicalization remains a separately versioned,
  read-only characterization path.
- `lattice-cjson-1` never silently reproduces or accepts V1 raw-number,
  non-NFC, or unframed behavior.
- Changing framing or canonical byte semantics requires a new algorithm/frame
  identifier and new golden fixtures.

## Consequences

- Every V2 hash consumer can share one byte algorithm without depending on
  unrelated domain semantics.
- Two small exact-version pure Rust dependencies enter the trusted build and
  must be pinned and visible in `Cargo.lock`.
- Task Domain additionally uses exact `time` 0.3.54 only to parse and emit
  canonical UTC RFC 3339 strings; it does not read a clock.
- Caller modules remain responsible for schema validation, numeric/timestamp
  normalization, subject selection, and authorization.

## Verification

- Cross-language golden framing bytes and SHA-256 digest.
- Unicode, key-order, collision, escaping, null/missing, array-order, and
  domain-separation fixtures.
- Cargo dependency inspection and full workspace format/lint/test gates.
