# TASK-013 Independent Code And Security Review

## Target

- Worktree: `lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Specification: SPEC-002 version 9
- Ticket: TASK-013
- Active contracts: Task Ledger 2.0, Contracts 1.3, Policy 2.4
- Reviewers: independent read-only code, security, and Ledger-gap subagents

## Review RED Findings

Independent review rejected earlier implementations until every accepted
finding had a failing regression:

- the first implementation lacked one public untrusted persistence/replay
  boundary and could not completely verify denied command records;
- request/receipt command substitution, forged cross-identity heads, corrupt
  exact retries, uncreated-stream stale retries, raw diagnostic limits,
  secret-shaped keys/values, and embedded token prefixes had incomplete
  adversarial coverage;
- Task Ledger accepted `TASK-_ABC` and `TASK--ABC`, although Task Domain
  requires the suffix to begin with an uppercase letter or digit;
- an uncreated stream with a retained terminal stale denial could retry
  internally but could not export its complete public snapshot;
- Policy's resource evidence tests did not substitute every receipt/head and
  decision-subject field required by TASK-013;
- the original Policy composition test supplied `receipt.head()` directly
  instead of obtaining a current head from the fake Ledger owner.

## Resolutions

- Added public raw snapshot DTOs and `verify_untrusted_snapshot`; it validates
  complete raw event, request, terminal receipt, command key, stream head, and
  resource projection rows and returns only a typed `VerifiedStream`.
- `FakeTaskLedger` exports and replays through that same boundary. Exact retry
  first verifies the complete stored snapshot before returning a receipt.
- Retained command records store the full sanitized request, including denied
  requests with no event. Cross-stream/identity poisoning, duplicate/orphan
  rows, unknown versions/outcomes, and every hash/head/projection mismatch
  fail closed.
- Diagnostics are raw-bounded before sanitization, NFC/NUL checked, reject
  secret-shaped keys, redact recognized secret values, and use redacted
  `Debug` implementations for untrusted DTOs.
- Task Ledger now enforces the same first suffix-character rule as Task
  Domain.
- Public snapshot export synthesizes a validated zero-event snapshot from the
  retained command identity when an uncreated stream has only terminal denied
  commands.
- Policy now has three explicit matrices:
  receipt identity/runtime substitution, independent current-head
  security-field substitution, and decision-subject substitution.
- A cross-crate composition test obtains
  `FakeTaskLedger::current_resource_head`, proves the current receipt passes,
  advances the stream, and proves the historical receipt fails stale.

## Final Results

Code review: `PASS`, zero remaining P0, P1, P2, or P3 findings.

Security review: `PASS`, zero remaining P0, P1, P2, or P3 findings.

Governance rescan: `PASS`; Task Ledger 2.0, Contracts 1.3, Policy 2.4,
ADR-011, SPEC-002 v9, and TASK-013 describe the same owner/currentness and
test-only composition boundaries.

Final focused evidence:

- Contracts: 13 tests pass;
- Task Ledger: 12 unit plus 8 integration tests pass;
- Policy: 3 unit plus 7 contract plus 56 matrix plus 1 receipt plus 8 V1
  compatibility tests, 75 total;
- format and workspace Clippy with `-D warnings`: pass;
- normal Cargo dependency trees: pass;
- test-only Policy-to-Ledger edge: explicit, one-way, and non-production;
- forbidden Task Ledger I/O scan: zero matches;
- `git diff --check`: pass.

## Documented Residuals

- A fully self-consistent rewrite of events, receipts, and the claimed head
  cannot be distinguished from authentic history by an unauthenticated
  SHA-256 replay alone. PostgreSQL permissions, transaction ownership, and an
  independently trusted current head remain required.
- TASK-013 exposes live snapshot verification but not a public live append
  planning API. The PostgreSQL ticket must first reuse a pure Ledger-owned
  append plan; an adapter may not duplicate hash/event semantics.
- Resource-observation revision and latest-receipt persistence are not restart
  evidence in this in-memory fake. Durable schema and recovery remain outside
  TASK-013.

These are explicit future owner gates, not remaining defects in the bounded
pure/fake TASK-013 contract.
