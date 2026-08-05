---
module_id: codebase-memory
name: LATTICE Codebase Memory
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-05
---

## Mission

Own pure, deterministic validation, canonical record construction, candidate
state, exact-snapshot retrieval ranking, and persistence-plan semantics for
provenance-first structural code memory.

## Non-Goals

- Read Git/filesystem/Graphify output, connect to PostgreSQL, execute SQL,
  select a process, or own transport/composition.
- Store raw source, credentials, untracked files, model prose, user
  preferences, policy, approvals, leases, scope, task truth, or release state.
- Promote candidates to trusted context or infer authority from graph edges.

## Owned Data

- Closed `NODE/EDGE`, `EXTRACTED/INFERRED/AMBIGUOUS`,
  `OBSERVATION/CANDIDATE`, retrieval-disposition, record, record-set,
  query-plan/result, and digest-validation semantics.
- Deterministic canonical ordering, bounded relevance scoring, no-answer
  behavior, and exact project/snapshot/commit binding.
- No durable rows; Postgres Store owns persistence mechanics.

## Public Contracts

- Accept only a complete typed graph analysis whose manifest/tool/config/
  graph digests validate and whose source references remain in the exact
  tracked manifest.
- Construct bounded records with strict ordinals, stable record/content
  digests, normalized labels/relations/paths, and no raw source body.
- Keep all TASK-033 graph records `OBSERVATION/CANDIDATE` and
  `trusted_context=false`.
- Rank a prevalidated fixed query deterministically: exact identifier/path/
  token matches outrank partial matches; stable record digest breaks ties.
- Bind retrieval to exact project/snapshot/commit/analysis, algorithm version,
  query digest, limit, ordered results, scores, disposition, and receipt digest.
- Return `NO_ANSWER` rather than cross-snapshot or irrelevant results.

## Invariants

1. Same valid input produces byte-identical record order and digests.
2. A record cannot cite a source outside its exact manifest.
3. Inferred/ambiguous evidence never becomes authority or trusted context.
4. Rejected input produces no persistence plan.
5. Retrieval never crosses project, snapshot, commit, or analysis.
6. Raw query text/source/secret data is not part of durable records.

## Allowed Dependencies

- `lattice-contracts` 1.11 and `lattice-cjson` canonical-byte utilities.

## Forbidden Dependencies

- Ports, orchestrator, adapters, database/Git/filesystem/process/JSON drivers,
  provider/model SDKs, Codex/OpenClaw/Graphify/Hermes implementations,
  policy/approval/Guardian/release/deployment/payment modules.

## Acceptance Gates

- Canonical record/digest golden vectors; duplicate/overflow/provenance denial;
  deterministic sorting and relevance/no-answer tests.
- Exact binding, changed-snapshot isolation, rejection-zero-plan, and
  malicious label/path/redaction tests.
- Independent code/security and architecture review.

## Change Policy

Version any record schema, canonical bytes, digest subject, state/review
meaning, ranking/scoring/tie-break, accepted provenance, trust boundary,
dependency direction, or persistence-plan contract.

## Failure, Compatibility, And Migration

- Invalid or cross-bound input returns a typed error and produces no
  persistence plan or external effect.
- Version 1.0 accepts only its exact typed contract and fails closed on future
  record/ranking versions. It owns no durable schema or migration.
- PostgreSQL durability remains blocked until an explicitly approved
  versioned owning-module amendment supplies database/extension identity.

## Amendment History

| Version | Date | Change | Authority |
|---|---|---|---|
| 1.0 | 2026-08-05 | Pure structural candidate normalization and deterministic exact-snapshot retrieval | TASK-033 user direction |
