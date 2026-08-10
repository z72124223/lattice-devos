---
module_id: task-domain
name: Task Domain
version: 2.3
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-11
---

## Mission

Define and validate the immutable Rust Task Spec V2, deterministic
specification-hash subject, task states, dependency graph, legal core
transitions, and the independent semantic vocabulary for Reflection work so
every LATTICE component uses the same workflow language without allowing
Reflection to rewrite core Task truth.

## Non-Goals

- Persist events, receipts, current status, queues, claims, or projections.
- Decide actor authorization, policy, approval sufficiency, provider
  capability, claim ownership, retry budgets, or scheduling.
- Own the generic canonical-byte algorithm or event/receipt hash semantics.
- Run Git, databases, filesystems, processes, models, network calls, MCP, or
  OpenClaw.
- Maintain a second mutable task record or a second Reflection truth.
- Define cross-Task discovery, `claim_next`, heartbeat, timeout, orphan
  recovery, retention, or unattended execution.

## Owned Data

- Task Spec schema/version `lattice.task-spec/2.1`; V2.0 remains read-only
  characterization.
- V2 Task Spec field validation and normalized immutable field ownership.
- V2 core Task state/transition rules plus explicit read-only V1 transition
  compatibility.
- Task dependency DAG semantics and stable cycle evidence.
- Selection of fields and schema domain used for Task Spec `spec_hash`.
- The complete normalized canonical subject and canonical Task Spec 2.1
  document bytes used by Gateway/persistence consumers. Reduced or alternate
  documents are not domain-owned representations.
- The independent first-slice Reflection vocabulary:
  - `REFLECTION_PENDING`
  - `REFLECTION_FAILED`
  - `RETRY_PENDING`
  - `DEGRADED`
- Closed Reflection failure categories:
  - `TASK_FAILURE`
  - `OUTPUT_REJECTED`
  - `HERMES_FAILURE`
- Closed non-authoritative candidate categories:
  - `OBSERVATION`
  - `INFERENCE`
  - `ROOT_CAUSE_CANDIDATE`
  - `IMPROVEMENT_CANDIDATE`
- Legal semantic edges in the independent Reflection state graph.

`lattice-cjson` owns only the mechanical canonical-byte and generic
domain-framing algorithm. Task Ledger owns persisted task events, event
canonical subjects, replay, and projections. Ports own transport values and
repository traits. This module mutates no external data.

## Public Contracts

- Validate and own a proposed Task Spec V2 with exact fields, normalized
  collection semantics, repository-relative scope, typed capabilities, string
  numeric budgets, one canonical accounting currency, and explicit approval
  requirements.
- Derive `spec_hash` from its immutable fields using the
  `lattice.task-spec/2.1` `lattice-cjson-1` domain.
- Export the complete normalized canonical subject and byte-stable canonical
  document from the validated Task Spec itself so callers never rebuild an
  incomplete hash carrier.
- Validate V2 core Task transitions and expose the frozen V1 transition matrix
  only as compatibility evidence.
- Parse and expose the closed Reflection state set without granting persistence,
  claim, provider, or scheduling authority.
- Validate legal first-slice Reflection transitions independently from core Task
  transitions.
- Expose closed Reflection failure and candidate categories so untrusted
  providers cannot invent new authority-bearing meanings.
- Detect unknown dependencies and cyclic task graphs with stable evidence.

## Core / Reflection Separation

Core Task state and Reflection state are separate semantic graphs:

```text
core Task: authoritative execution lifecycle
Reflection: independent post-event learning / failure evidence lifecycle
```

Rules:

1. Reflection transitions never transition a core Task.
2. Core `COMPLETED`, its result, and its verified core head remain immutable
   under later Reflection events.
3. `DEGRADED` means core output remains usable without successful Reflection;
   it does not downgrade core completion.
4. Reflection candidates are non-authoritative by type and cannot become Task
   truth without a separately governed promotion path.
5. The first slice permits repository policy to be narrower than the semantic
   graph. In particular, the current known-Task queue may allow only completed
   core Tasks to claim/retry/append candidates while failed/rejected core
   records remain read-only evidence.
6. The final product must later add a separate versioned failure-learning
   contract for failed/rejected Tasks; it must not silently expand the 2.3
   first-slice semantics.

## Invariants

1. Mutable status, events, approvals, evidence, Reflection projections, and
   candidates are never part of `spec_hash`.
2. Unknown schema versions, states, capabilities, checks, operations,
   candidate kinds, failure kinds, and transitions fail closed at their parsing
   boundary.
3. The same normalized Task Spec always produces the same hash.
4. Every immutable approval-relevant Task Spec field changes the hash subject.
5. Scope rejects absolute, parent, NUL, backslash, and `.git` write paths.
6. Routine pre-authorized local work is not hard-coded to require a human
   waiting state; Policy still decides whether a direct transition is allowed.
7. Task-domain functions perform no I/O and have no hidden clock, process,
   randomness, provider, queue, or database dependency.
8. V1 compatibility behavior is namespaced and cannot create a V2 hash.
9. `budget.accounting_currency` is a canonical uppercase three-letter code,
   is part of `spec_hash`, and denominates `max_external_cost`; this module
   performs no currency conversion. Canonical decimal budget strings are at
   most 256 ASCII bytes, with at most 127 integer digits and 128 fractional
   digits; those public bounds are shared with Policy.
10. `canonical_subject`, `canonical_document`, and `spec_hash` cover the exact
    same complete normalized immutable fields. Re-encoding is byte-stable and
    recomputing the frozen hash domain yields the owned `spec_hash`.
11. A Gateway, template, adapter, or persistence layer cannot supply a reduced
    canonical document and call it Task Spec 2.1 domain evidence.
12. Reflection state is never projected as core `TaskState`, and core state is
    never parsed as `ReflectionState`.
13. `ReflectionCandidateKind` values remain non-authoritative; adding a new
    candidate kind or promotion meaning requires a versioned amendment.
14. Domain transition legality does not imply caller authorization, durable
    append success, claim ownership, provider success, or acceptance.

## Allowed Dependencies

- Rust standard library.
- `lattice-contracts` immutable identifier types.
- `lattice-cjson` 1.0 canonical bytes and digest.
- Exact-version `time` 0.3.54 with parsing/formatting features only, for
  deterministic UTC RFC 3339 validation and normalization.

## Forbidden Dependencies

- Filesystem, subprocess, Git, database, network, MCP, OpenClaw, ports, Task
  Ledger, Policy Engine, Orchestrator, provider/runtime adapters, credentials,
  clocks, queues, schedulers, or randomness.

## Failure, Compatibility, And Migration

Validation returns stable errors and never partially accepts a spec or
Reflection value. Unknown schema/state/category versions require an explicit
migration design and ADR. V1 and Task Spec 2.0 data remain read-only
characterization evidence and are never silently rewritten or hashed through
the 2.1 path.

Version 2.2 added only the public complete canonical subject/document carrier
for an already validated Task Spec 2.1. It changed no schema field,
normalization, hash domain, transition, dependency, or I/O boundary.

Version 2.3 adds the independent first-slice Reflection state graph and closed
failure/candidate vocabulary. It does not add persistence, queue ownership,
provider execution, cross-Task discovery, scheduling, or authority. Existing
core Task Spec and transition semantics remain byte- and behavior-compatible.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| V2 construction | focused valid/invalid Task Spec tests | Engineering | yes |
| V1/V2 transitions | complete frozen matrices and stable error tests | Engineering | yes |
| Reflection state set | parse/wire round-trip for the exact four states and unknown-state rejection | Engineering | yes |
| Reflection transitions | complete legal/illegal transition matrix independent from core Task transitions | Engineering | yes |
| Reflection categories | exact failure/candidate wire values and closed unknown-value boundary | Engineering | yes |
| Core separation | tests prove Reflection tails cannot rewrite core completion/result/core head | Architecture review | yes |
| Hash stability | canonical-byte fixtures and immutable-field mutation matrix | Engineering | yes |
| Canonical document | byte-stable document equals the complete subject and recomputes the owned spec hash; reduced/alternate carriers fail comparison | Security review | yes |
| Dependency graph | acyclic/unknown/cycle fixtures with stable evidence | Engineering | yes |
| No-I/O/dependencies | Cargo metadata and forbidden-reference scan | Architecture review | yes |
| Full verification | Rust workspace plus preserved Node suite | Engineering | yes |

## Change Policy

Mission, Task schema fields, hash subject, core or Reflection states, public
contracts, candidate/failure categories, dependency direction, or transition
rules require a versioned amendment, specification/ADR update, architecture
review, and explicit responsible-user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-001, ADR-001 | Initial offline task contract | Current user task |
| 2.0 | 2026-07-29 | SPEC-002 v4, ADR-004/005, approved V2 amendment, TASK-010 | Rust Task Spec V2, V1 transition compatibility, and separated canonical mechanism | User execution directive |
| 2.1 | 2026-07-29 | SPEC-002 v6, ADR-008/009, TASK-011 review RED | Immutable external-cost accounting currency added to Task Spec hash | User MVP-3 execution directive |
| 2.2 | 2026-08-09 | SPEC-003 v3, ADR-023, TASK-038 | Export the complete normalized canonical subject/document from the validated Task Spec so every boundary uses one hash carrier | User TASK-038-first direction |
| 2.3 | 2026-08-11 | Issue #9 decision freeze, ADR-024, GH-9 | Add an independent, closed Reflection state/failure/candidate vocabulary while preserving immutable core Task completion and keeping persistence/provider authority outside Task Domain | Responsible user architecture amendment |