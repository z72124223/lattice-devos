---
module_id: task-domain
name: Task Domain
version: 2.1
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Define and validate the immutable Rust Task Spec V2, deterministic
specification-hash subject, task states, dependency graph, and legal
transitions so every LATTICE component uses the same workflow language.

## Non-Goals

- Persist events, receipts, current status, or projections.
- Decide actor authorization, policy, approval sufficiency, or provider
  capability.
- Own the generic canonical-byte algorithm or event/receipt hash semantics.
- Run Git, databases, filesystems, processes, models, network calls, or
  OpenClaw.
- Maintain a second mutable task record.

## Owned Data

- Task Spec schema/version `lattice.task-spec/2.1`; V2.0 remains read-only
  characterization.
- V2 Task Spec field validation and normalized immutable field ownership.
- V2 task state/transition rules plus explicit read-only V1 transition
  compatibility.
- Task dependency DAG semantics and stable cycle evidence.
- Selection of fields and schema domain used for Task Spec `spec_hash`.

`lattice-cjson` owns only the mechanical canonical-byte and generic
domain-framing algorithm. Task Ledger owns persisted task events, event
canonical subjects, replay, and projections. This module mutates no external
data.

## Public Contracts

- Validate and own a proposed Task Spec V2 with exact fields, normalized
  collection semantics, repository-relative scope, typed capabilities, string
  numeric budgets, one canonical accounting currency, and explicit approval
  requirements.
- Derive `spec_hash` from its immutable fields using the
  `lattice.task-spec/2.1` `lattice-cjson-1` domain.
- Validate V2 transitions and expose the frozen V1 transition matrix only as
  compatibility evidence.
- Detect unknown dependencies and cyclic task graphs with stable cycle paths.

## Invariants

1. Mutable status, events, approvals, evidence, and projections are never part
   of `spec_hash`.
2. Unknown schema versions, states, capabilities, checks, operations, and
   transitions fail closed at their parsing boundary.
3. The same normalized Task Spec always produces the same hash.
4. Every immutable approval-relevant Task Spec field changes the hash subject.
5. Scope rejects absolute, parent, NUL, backslash, and `.git` write paths.
6. Routine pre-authorized local work is not hard-coded to require a human
   waiting state; Policy still decides whether a direct transition is allowed.
7. Task-domain functions perform no I/O and have no hidden clock.
8. V1 compatibility behavior is namespaced and cannot create a V2 hash.
9. `budget.accounting_currency` is a canonical uppercase three-letter code,
   is part of `spec_hash`, and denominates `max_external_cost`; this module
   performs no currency conversion. Canonical decimal budget strings are at
   most 256 ASCII bytes, with at most 127 integer digits and 128 fractional
   digits; those public bounds are shared with Policy.

## Allowed Dependencies

- Rust standard library.
- `lattice-contracts` immutable identifier types.
- `lattice-cjson` 1.0 canonical bytes and digest.
- Exact-version `time` 0.3.54 with parsing/formatting features only, for
  deterministic UTC RFC 3339 validation and normalization.

## Forbidden Dependencies

- Filesystem, subprocess, Git, database, network, OpenClaw, ports, Task Ledger,
  Policy Engine, Orchestrator, provider/runtime adapters, credentials, clocks,
  or randomness.

## Failure, Compatibility, And Migration

Validation returns stable errors and never partially accepts a spec. Unknown
schema versions require an explicit migration design and ADR. V1 and Task Spec
2.0 data remain read-only characterization evidence and are never silently
rewritten or hashed through the 2.1 path.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| V2 construction | focused valid/invalid Task Spec tests | Engineering | yes |
| V1/V2 transitions | complete frozen matrices and stable error tests | Engineering | yes |
| Hash stability | canonical-byte fixtures and immutable-field mutation matrix | Engineering | yes |
| Dependency graph | acyclic/unknown/cycle fixtures with stable evidence | Engineering | yes |
| No-I/O/dependencies | Cargo metadata and forbidden-reference scan | Architecture review | yes |
| Full verification | Rust workspace plus preserved Node suite | Engineering | yes |

## Change Policy

Mission, schema fields, hash subject, states, public contracts, dependency
direction, or transition rules require a versioned amendment, specification
update, architecture review, and explicit responsible-user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-001, ADR-001 | Initial offline task contract | Current user task |
| 2.0 | 2026-07-29 | SPEC-002 v4, ADR-004/005, approved V2 amendment, TASK-010 | Rust Task Spec V2, V1 transition compatibility, and separated canonical mechanism | User execution directive |
| 2.1 | 2026-07-29 | SPEC-002 v6, ADR-008/009, TASK-011 review RED | Immutable external-cost accounting currency added to Task Spec hash | User MVP-3 execution directive |
