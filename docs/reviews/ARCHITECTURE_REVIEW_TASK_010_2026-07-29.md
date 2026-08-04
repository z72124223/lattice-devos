# TASK-010 Architecture Review

## Triggers

- New reusable canonical-byte and hash-frame module.
- Material Task Domain V2 constitution and public-contract change.
- New dependency edges and approval-relevant hash subject.
- One Gateway, One Truth, and One Writer boundary sensitivity.

## Independent Result

`No blockers`. The current TASK-010 state passes the architecture gate.

Confirmed:

- `lattice-cjson` owns only deterministic canonical bytes and the generic
  `lattice-hash-1` frame.
- Task Domain privately selects the `lattice.task-spec/2.0` subject and
  immutable field set.
- Task Ledger and later approval, memory, and guardian modules retain their
  own subject semantics; sharing the byte mechanism transfers no semantic
  ownership.
- Dependency direction is
  `lattice-task-domain -> lattice-contracts + lattice-cjson + time`, while
  `lattice-cjson` has no LATTICE dependency. No cycle or concrete-adapter edge
  exists.
- Both crates are pure: no filesystem, process, network, database, provider,
  clock, or random input. They create no second gateway, truth, or writer.
- `lattice-cjson` 1.0, `task-domain` 2.0, and ADR-008 provide the required
  versioned decisions; no further constitution amendment is required.
- The stale approval paragraph in the V2 amendment record was corrected to
  reflect bounded local execution through MVP-3 while preserving protected
  gates.

## Gate Classification

| Gate | Classification | Current evidence |
|---|---|---|
| No raw canonical number | machine-enforced | Rust `CanonicalValue` variants and tests |
| Validated Task Spec encapsulation | machine-enforced | private `TaskSpec` fields and constructor |
| Exact dependency versions | machine-enforced | Cargo manifests and `Cargo.lock` |
| Formatting, lint, behavior | machine-enforced locally | format, Clippy, and focused/full tests |
| Semantic subject ownership | documented-only | ADR-008, constitutions, code inspection |
| One Gateway/Truth/Writer | documented-only | constitutions plus zero-I/O/dependency scan |
| Dedicated architecture lint | missing | no repository rule checker for semantic edges |
| Remote CI and branch protection | missing/unverified | no remote configured |
| PostgreSQL/live integration | unverified | explicitly outside TASK-010 |

## Residual Non-Blocking Risks

1. Canonical-value and DAG traversal are recursive and do not yet impose an
   explicit depth, node-count, or byte budget. There is no untrusted wire parser
   in TASK-010, so this does not block the pure internal API. A later parsing
   or Policy boundary must enforce resource limits before accepting untrusted
   input.
2. Semantic hash-subject ownership, forbidden I/O, and One Gateway/Truth/Writer
   are supported by constitutions, review, Cargo direction, and a static
   zero-match scan, but no dedicated architecture-lint rule currently prevents
   future drift.

Integration blocker result: no architecture blocker.
