---
module_id: hermes-adapter
name: LATTICE Hermes Reflection Adapter
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-05
---

## Mission

Obtain one bounded reflection candidate from the exact pinned Hermes source in
a verified whole-process OS containment boundary after an exact graph receipt.

## Non-Goals

- Write product code, authorize work, accept or promote memory, own durable
  state, or recover truth from Hermes process memory.

## Owned Data

- Ephemeral child/run/session binding and strict canonical reflection parsing.
- No project truth, product source, Git/database credential, durable receipt,
  `MEMORY.md`, SQLite, skill, or provider state.

## Public Contracts

- Accept only typed redacted task/project/commit/tree/graph-receipt context with
  fixed bounds and no raw source, secret, SQL, path selection, or credential.
- Verify exact source/executable/schema identity, OS containment, empty isolated
  cwd/no product mount, isolated homes, and memory/hooks/updates off before run.
- Use one absolute deadline across capability, submit, event, and status work.
- Return only a closed schema-valid `INFERENCE/CANDIDATE` envelope with exact
  provenance and digest; post-submit ambiguity never causes resubmission.

## Invariants

1. Isolated homes are state separation, not OS isolation.
2. Server-side tools require verified containment with no product mount,
   Git/database credential, or normal home.
3. Version probes receive no API key or provider credential.
4. Hermes output cannot authorize, mutate, persist itself, or become truth.
5. Fresh LATTICE status reads PostgreSQL only and performs zero Hermes calls.

## Allowed Dependencies

- `lattice-contracts`, `lattice-ports`, canonical hashing, bounded local API
  transport, and a verified containment/process launcher.

## Forbidden Dependencies

- Product repositories, Git mutation, PostgreSQL clients, writable Codex,
  policy/approval/Guardian authority, or memory promotion.

## Failure, Compatibility, And Migration

Unknown identity/capability/schema, containment drift, deadline expiry,
malformed/cross-bound output, mutation evidence, or ambiguous teardown fails
closed. Hermes v2026.8.3 source package 0.20.0 is source-pinned; a PyPI install
must not be claimed because that release is not published there.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Identity and probe secrecy | source/executable/schema digests; no probe API key | Engineering | yes |
| OS containment | empty cwd/no product mount and credential/home/memory denial | Security review | yes |
| Closed context/output | substitution, secret, schema and provenance matrix | Engineering | yes |
| Deadline/recovery | one deadline and no post-submit resubmission | Engineering | yes |
| Durable replay | PostgreSQL survives restart; fresh status makes zero Hermes calls | Integration | yes |

## Change Policy

Mission, identity, containment, schemas, dependencies, or durability ownership
changes require a versioned amendment and responsible-user approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-05 | SPEC-002 v27, V2 amendment, PLANS Step 9, TASK-033 | Contained reflection with PostgreSQL-owned replay | User full-chain directive |
