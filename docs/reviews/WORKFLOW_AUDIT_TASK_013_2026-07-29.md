# TASK-013 Workflow Audit

- Date: 2026-07-29
- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Mode: read-only audit before TASK-013 application-code modification

## Confirmed Slice

TASK-013 freezes the pure Rust Task Ledger 2.0 semantic owner and a
deterministic, visibly non-durable fake. It may amend shared resource-receipt
representation and Policy consumption because the current Policy-local
`ResourceUsageFact` is caller-constructible and does not prove Task Ledger
ownership or currentness.

This ticket performs no PostgreSQL, filesystem, Git, process, network,
credential, provider, payment, publication, deployment, product-repository, or
protected-release I/O. PostgreSQL remains the only future durable truth.

The worktree already contains the shared uncommitted MVP-0 and TASK-009 through
TASK-012 result. The separate V1 worktree remains present at the same base and
was not modified. No reset, clean, branch switch, commit, push, merge, worktree
mutation, publication, or deployment occurred during the audit.

## Audit Evidence

- Git repository, branch, base, two-worktree inventory, and no remote/upstream
  were re-observed.
- The active worktree had 69 dirty paths at the TASK-013 baseline.
- `PLANS.md` had exactly one current marker:
  `CURRENT TASK-013 PLANNING`.
- Baseline Rust:
  `cargo test --workspace --all-targets --all-features --locked`, exit 0,
  118 tests.
- Baseline Node: `npm.cmd run verify`, exit 0, 38 tests;
  project check reported 146 files and 13 constitutions.
- The configured local project router entry point was absent, so routing used
  the current repository, PLANS, and HANDOFF evidence directly.
- Local memory contained no TASK-013 or LATTICE Task Ledger project evidence
  and was not used.

## Capability Classification

| Capability | Status before update | Evidence | Enforcement |
|---|---|---|---|
| Repository/rules/plan/handoff | valid | Git, AGENTS, PLANS, HANDOFF, TASK-012 evidence | documented plus machine-observed |
| Requirements | valid | TASK-012 next-slice contract and approved V2 proposal | documented-only |
| Specification | partial | SPEC-002 v8 names Task Ledger 2.0 but does not freeze its pure fake contract | documented-only |
| Task Ledger constitution | stale | active file is V1 Node/filesystem 1.0 while V2 proposal requires 2.0/PostgreSQL ports | documented conflict |
| Contracts resource receipt | missing | no fixed Ledger producer/runtime/full receipt/head | missing |
| Policy resource consumption | partial | caller strings and `fresh` Boolean are accepted | machine-tested behavior with an owner-authenticity gap |
| Ticket | missing | PLANS marker only | missing |
| TDD entry point | valid | Cargo workspace, cjson/contracts/policy patterns, focused/full commands | machine-executable locally |
| PostgreSQL event transaction | blocked for this ticket | ADR-005 and AC-03 require a later durable-store ticket | deliberately deferred |
| Remote CI/branch protection | missing/unverified | no remote or service evidence | missing |
| Primary merge authorization | blocked | no committed candidate or explicit primary-merge authorization | absent |

## V1 Characterization Findings

Retained semantics:

- sequence starts at one with a zero predecessor;
- exact command retry is checked before stale-sequence denial;
- same command and sanitized request returns the original receipt;
- changed command content rejects;
- new stale sequence rejects;
- stream/head hash verification, deterministic replay, and secret-before-write
  intent remain required.

Rejected as active V2 design:

- Node canonical JSON, raw numbers/floats, file JSONL/head persistence,
  in-process queues, split append/head writes, Task Ledger importing Task
  Domain, heuristic JavaScript sanitization, unknown-event no-op behavior, and
  task-ID-only stream identity.

Adversarial characterization found that two V1 instances can both append
sequence one and leave an invalid stream; a writer able to rewrite payload,
hash, and head can make tampering self-consistent; secret redaction can collapse
distinct diagnostics into one retry subject; and JavaScript prototype/sensitive
value cases escape or distort the sanitizer. These are vulnerability evidence,
not compatibility behavior to port.

## Ownership Decision

1. Task Ledger 2.0 owns stream/event/request/receipt hash subjects, exact
   command replay, verified replay, corruption semantics, and its resource
   projection.
2. Task Domain remains the sole owner of legal task-state transitions. Task
   Ledger has no Task Domain dependency; future Orchestrator composition feeds
   verified events into a versioned Task Domain reducer.
3. `lattice-contracts` 1.3 owns only neutral immutable Task Ledger head and
   resource observation receipt/head representation with fixed producer and
   runtime identity.
4. Policy 2.4 consumes the owner receipt plus a head obtained from an
   independent current Ledger-owner lookup. It removes the caller `fresh`
   Boolean and caller-selected owner/producer strings.
5. Real effect authorization still requires PostgreSQL to re-check and claim
   the resource projection in the same epoch/admission-bound transaction as the
   effect/outbox claim. A fake observation cannot authorize a live effect.

## Execution Order

1. Update SPEC-002, ADR-009/011, constitutions, routing, and TASK-013.
2. Add shared receipt/head RED, then GREEN.
3. Add pure Task Ledger crate/API RED, then one behavior at a time.
4. Add Policy resource receipt/current-head composition RED/GREEN.
5. Run focused and full verification plus forbidden dependency/I/O scans.
6. Run independent code, security, architecture, and integration review.
7. Update the workflow ledger, ticket, PLANS, and HANDOFF.

## Minimum Remaining Controls

- Machine-test exact request/event/head/receipt domains, retries, stale-head
  denial, replay/tamper rejection, bounded diagnostics, resource projection,
  and owner receipt/current-head composition.
- Keep AC-03 PostgreSQL atomicity and full AC-04 restart/durable projection
  evidence open.
- Keep authenticated current-owner lookup, daemon epoch/admission, effect
  claim/outbox atomicity, remote CI, branch protection, and merge authorization
  explicitly deferred or blocked.
