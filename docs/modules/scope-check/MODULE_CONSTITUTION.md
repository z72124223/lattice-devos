---
module_id: scope-check
name: Scope Check
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Compare normalized Git change evidence against the exact Task Spec scope and
produce a deterministic pass/violation manifest without modifying anything.

## Non-Goals

- Prevent a hostile process from writing.
- Act as an operating-system sandbox.
- Repair, revert, delete, stage, or ignore a violation.
- Change task status or approve work.

## Owned Data

- Allowed-path pattern semantics.
- Changed-operation normalization.
- Violation reason codes, stable ordering, rule hash, and evidence hash.

The Task Spec owns requested scope; Git adapter owns raw changed-path evidence;
Task Ledger owns persisted reports.

## Public Contracts

- Validate allowed/forbidden repository-relative patterns.
- Normalize add/modify/delete/rename/type-change records.
- Check both source and destination of a rename.
- Return a stable report with all violations and hashes.
- Reject absolute, traversal, empty, `.git`, link, junction, and escaped paths.

## Invariants

1. Scope Check is read-only.
2. Every evaluated path is repository-relative and slash-normalized.
3. Forbidden patterns override allowed patterns.
4. Unknown operations and link kinds deny.
5. Identical rules and changes produce identical sorted reports/hashes.
6. A pass is labeled detection evidence, never containment proof.

## Allowed Dependencies

- Node.js path and cryptographic standard-library APIs.
- Plain Task Spec scope data and Git change records.

## Forbidden Dependencies

- Filesystem mutation, Git commands, Policy Engine decisions, Task Ledger
  writes, Orchestrator state, Runtime, OpenClaw, or network.

## Failure, Compatibility, And Migration

Malformed patterns or change records fail closed. Pattern semantic changes
require versioned rule hashes and regression fixtures so prior reports remain
interpretable.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Canonical path tests | `node --test test/scope-check.test.js` | Engineering | yes |
| Rename/operation tests | table-driven fixtures | Engineering | yes |
| Escape/link tests | Windows/POSIX path fixtures | Security review | yes |
| Full verification | `npm run verify` | Engineering | yes |

## Change Policy

Path semantics, override order, operation coverage, hashes, or the read-only
contract require a versioned amendment, security review, and explicit
responsible-human approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-001, ADR-002 | Initial detection-only scope gate | Current user task |

