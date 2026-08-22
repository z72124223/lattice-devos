---
module_id: hermes-adapter
name: LATTICE Hermes Reflection Adapter
version: 1.3
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-22
---

## Mission

Obtain one bounded, read-only reflection candidate through the verified Codex
App Server in a whole-process OS containment boundary after an exact Graphify
receipt. The historical pinned Hermes Gateway is retained only as diagnostic
evidence; it is not a required production dependency.

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
- Production admission and its ephemeral receipt bind only inputs that the
  production process path actually executes or consumes. A non-executed
  helper path or caller-supplied matching digest cannot become a trust gate.
- The direct bridge starts exactly one Codex App Server turn with
  `approvalPolicy=never`, a read-only sandbox, no network access, an empty
  owned cwd, and a closed reflection output schema. Any server request is a
  terminal denial, not an approval or fallback path.

## Invariants

1. Isolated homes are state separation, not OS isolation.
2. The direct Codex bridge has no tool, approval, or write fallback; a server
   request terminates its owned Job Object.
3. Version probes receive no API key or provider credential.
4. Hermes output cannot authorize, mutate, persist itself, or become truth.
5. Fresh LATTICE status reads PostgreSQL only and performs zero Hermes calls.
6. The contained production proxy executes the exact verified Codex launcher;
   the retired Gateway and legacy helper have no production admission authority.

## Allowed Dependencies

- `lattice-contracts`, `lattice-ports`, canonical hashing, the Codex App
  Server protocol, and a verified containment/process launcher.

## Forbidden Dependencies

- Product repositories, Git mutation, PostgreSQL clients, writable Codex,
  policy/approval/Guardian authority, or memory promotion.

## Failure, Compatibility, And Migration

Unknown identity/capability/schema, containment drift, deadline expiry,
malformed/cross-bound output, a server request, mutation evidence, or ambiguous
teardown fails closed. The historical Hermes v2026.8.3 source package is kept
as diagnostic evidence only and must not be presented as a runtime requirement.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Identity and probe secrecy | exact Codex bundle/configuration digest; no probe API key | Engineering | yes |
| OS containment | empty cwd/no product mount, read-only policy, and Job teardown | Security review | yes |
| Closed context/output | substitution, secret, schema and provenance matrix | Engineering | yes |
| Deadline/recovery | one deadline and no post-submit resubmission | Engineering | yes |
| Durable replay | PostgreSQL survives restart; fresh status makes zero Hermes calls | Integration | yes |
| Direct Codex policy | closed thread/turn plan, tool-request denial, exact launcher plan | Engineering and security review | yes |

## Change Policy

Mission, identity, containment, schemas, dependencies, or durability ownership
changes require a versioned amendment and responsible-user approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.3 | 2026-08-22 | Four-core Runtime goal | Replace the retired Hermes Gateway production dependency with one LATTICE-owned, contained, read-only Codex App Server reflection bridge; retain the Gateway only for diagnostics | User goal-mode approval |
| 1.2 | 2026-08-22 | Runtime model policy | Replace the retired Spark broker identity with the fixed `gpt-5.6-terra` identity; non-5.6 substitutions remain rejected before launch | User goal-mode approval |
| 1.1 | 2026-08-13 | SPEC-002 v31, TASK-065 | Remove the non-executed broker helper from production admission and receipt identity; retain its legacy one-shot protocol separately | User goal-mode direction to complete Hermes |
| 1.0 | 2026-08-05 | SPEC-002 v27, V2 amendment, PLANS Step 9, TASK-033 | Contained reflection with PostgreSQL-owned replay | User full-chain directive |
