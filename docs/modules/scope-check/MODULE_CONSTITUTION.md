---
module_id: scope-check
name: Scope Check
version: 1.1
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-10
---

## Mission

Compare language-neutral normalized Git change evidence with an exact Task
Spec path/operation scope and produce a deterministic detection-only
pass/violation manifest without performing I/O or mutation.

## Non-Goals

- Prevent a hostile process from writing or act as an operating-system sandbox.
- Inspect the filesystem, invoke Git, or trust a caller containment Boolean.
- Repair, revert, delete, stage, ignore, approve, merge, or change task state.
- Own Project Registry identity, Writer Lease/fencing, Git truth, or persisted
  reports.

## Owned Data

- Versioned exact/prefix/glob path-pattern semantics and allowed operations.
- Path/operation/link classification, forbidden precedence, violation reason
  codes, stable ordering, rule/evidence/report hashes, and the explicit
  detection-only label.

Task Spec owns requested scope. Workspace Git owns normalized raw change and
path-kind evidence. Project Registry owns identity. Task Ledger owns durable
report persistence.

## Public Contracts

- Validate repository-relative exact, segment-prefix, and language-neutral
  glob rules.
- Classify every normalized create/modify/delete/rename/type-change/unmerged
  record; both source and destination of a rename are checked.
- Reject malformed/absolute/traversal/empty/`.git`/escaped paths, Windows
  trailing-dot/space and reserved-device aliases, and any symlink, Git link,
  junction/reparse point, directory, or unknown path kind.
- Apply forbidden patterns before allowed patterns and return all stable-sorted
  violations.
- Return stable rule, evidence, and report hashes plus `DETECTION_ONLY` evidence
  level; a pass makes no containment or authorization claim.

## Invariants

1. Scope Check is pure and read-only: no filesystem, Git, process, clock,
   environment, network, database, or task-state access.
2. Every evaluated path is repository-relative, slash-normalized, and checked
   independently; invalid evidence fails closed.
3. Forbidden patterns override allowed patterns and operation permission.
4. Both sides of rename/copy-like evidence are checked; unknown operations or
   path kinds deny.
5. Identical versioned rules and evidence produce identical sorted reports and
   hashes regardless of input order.
6. `PASS` is always labeled detection evidence and never proves containment,
   lease currentness, policy authorization, or merge readiness.

## Allowed Dependencies

- `lattice-workspace-git` for its language-neutral normalized change manifest.
- `lattice-cjson` for canonical bytes and domain-separated SHA-256 evidence
  hashing.
- Rust standard-library collections only.

## Forbidden Dependencies

- Filesystem/path inspection APIs, Git commands, process execution, Policy,
  Project Registry, Writer Lease, Task Ledger writes, Orchestrator state,
  Runtime, OpenClaw, providers, credentials, network, or mutation APIs.

## Failure, Compatibility, And Migration

Version 1.1 preserves the V1 detection-only contract and removes Node.js as a
constitutional dependency. Malformed rules or evidence fail closed. Rule or
hash semantic changes require a new version/domain so prior manifests remain
interpretable. V1 Node artifacts remain historical characterization only.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Rule/path tests | exact/prefix/glob plus absolute/traversal/`.git` matrices | Engineering | yes |
| Rename/precedence tests | table-driven both-side and forbidden-first fixtures | Engineering | yes |
| Link/escape tests | symlink/Git-link/reparse/unknown path-kind denial | Security review | yes |
| Stable evidence tests | input-order permutation and frozen hash fixtures | Engineering | yes |
| Read-only/dependency gate | source/dependency inspection plus strict Clippy | Architecture review | yes |
| Full verification | focused/full Rust and preserved Node suites | Engineering | yes |

## Change Policy

Path/pattern semantics, operation coverage, precedence, hash subjects,
evidence level, dependency direction, or the read-only contract require a
versioned amendment, security/architecture review, and explicit
responsible-human approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-001; ADR-002 | Initial Node detection-only scope gate | Current user task |
| 1.1 | 2026-08-09 | SPEC-002 v27; approved V2 amendment; TASK-027 | Consume Rust normalized evidence while preserving pure detection-only semantics | Responsible user |
