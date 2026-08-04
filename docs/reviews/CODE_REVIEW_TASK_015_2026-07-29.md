# TASK-015 Independent Code And Security Review

## Target

- Worktree: `lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Specification: SPEC-002 version 11
- Ticket: TASK-015
- Active contracts: Approval Verifier 1.0, Contracts 1.5, Policy 2.6
- Reviewers: independent read-only Approval, Policy, security, dependency, and
  architecture reviewers

## Review RED Findings

Independent review rejected earlier implementations until every accepted
finding had a failing regression:

- Policy returned before its Review Runtime fail-closed check when the
  ordinary approval floor was `NotRequired` or `Policy`. A task requiring both
  security and architecture review could therefore reach an allow result
  without Review Runtime authority.
- Fact-memory promotion hard-coded the independent-review flag to false and
  could bypass the same Review Runtime boundary.
- `ApprovalChallenge` exposed mutable public fields and fake signers copied the
  retained digest without recomputing the envelope. A substituted benign
  subject with the original dangerous digest could confuse the signer.
- Required security evidence was incomplete: no fixed golden challenge/proof/
  receipt hashes, incomplete five-family typed-subject hash matrices, missing
  normal/protected and Guardian substitution matrices, no denied exact-retry
  proof, and no denial-tail checkpoint rollback proof.
- ADR-013 required typed revocation and unrevoked currentness, but the first
  state machine had no revoke command, revoked phase, public revocation record,
  or raw replay path.
- Integration initially contained two `[dev-dependencies]` sections in the
  Policy manifest and strict Clippy exposed oversized test functions and
  similar-name bindings.

## Resolutions

- `approval_reason` now returns `ReviewAuthorityUnavailable` for every
  independent-review-required path even when the ordinary approval floor is
  `NotRequired` or `Policy`.
- Fact and preference memory promotion now preserve the same fail-closed
  Review Runtime rule. Two regressions first reproduced both unexpected allow
  results and then passed after repair.
- `ApprovalChallenge` fields are private and exposed only through read-only
  getters. Normal and protected signers recompute subject and challenge
  digests and validate runtime, canonical time, identifiers, non-zero
  commitments, exact actor/authenticator/key/evidence, trust lane, and
  Guardian runtime/trust-root binding before producing proof.
- An internal preserved-digest tamper regression proves the signer rejects a
  cloned envelope whose subject changed while its old digest was retained.
- Added fixed golden challenge, proof, and authority-receipt digests and full
  field-substitution matrices for execution/external-cost, merge, preference,
  protected-change, protected-release, project/snapshot/task/spec binding,
  trust lane, and Guardian identity/runtime/trust root.
- Applied and denied terminal commands retain exact retry, changed-content
  command-ID rejection, predecessor chaining, high-water/tail commitments,
  strict raw replay, and trusted-checkpoint rollback comparison.
- Added `RevokeApprovalCommand`, `ApprovalPhase::Revoked`, and a public
  immutable evidence-bound `ApprovalRevocation`. Normal and protected
  available authority can be revoked only by the exact approver while
  time-current. Revocation advances the owner revision, removes current-head
  availability, and is bound into state, terminal receipts, snapshots, raw
  replay, and checkpoints. Claimed, challenged, already revoked, wrong-
  revoker, stale, and expired cases deny without partial mutation.
- Policy has one consolidated dev-dependency section. Test helpers were split
  without lint suppressions, and all strict Clippy findings were repaired.

## Final Results

Code review: `PASS`, zero remaining P0, P1, P2, or P3 findings.

Security review: `PASS`, zero remaining P0, P1, P2, or P3 findings.

Final focused evidence:

- Approval Verifier: 1 unit plus 27 integration tests pass;
- Contracts: 25 tests pass;
- Policy: 84 tests pass;
- full Rust workspace: 218 tests pass;
- strict workspace Clippy with `-D warnings`: pass;
- Rust format and `git diff --check`: pass;
- normal dependency tree and forbidden-I/O/legacy-Boolean scans: pass;
- preserved Node verification: 38 tests pass;
- final closure project check reports 174 files and 15 constitutions before
  the three TASK-015 final review artifacts are added.

Reviewed source hashes:

- `crates/lattice-approval-verifier/src/lib.rs`:
  `41aaa3f38bdc429213dd34ac7c1644f4a8b2e6f94dbf1e003a3107591f2f3eab`
- `crates/lattice-approval-verifier/tests/approval_verifier.rs`:
  `f5ee30ea1c048d5c844fa630b3eab6bd5006112512f09681ca62cf71ef45e566`

## Documented Residuals

- The fake signer is deterministic test evidence, not live cryptographic or OS
  authentication.
- Context-free replay proves internal consistency. Rollback-sensitive restore
  requires an independently retained validated checkpoint.
- PostgreSQL must serialize nonce uniqueness, database time, aggregate/
  receipt/checkpoint persistence, normal effect claim, restart, and atomic
  current-head revalidation.
- Protected release remains pending. Only the future Guardian/PostgreSQL
  `claim_activation` transaction may atomically claim its nonce, append the
  activation event, and change runtime admission.
- Review Runtime remains absent. R3 and every independent-review-required path
  therefore intentionally deny.
- OpenClaw approval IPC, live trust roots, product effects, remote CI, branch
  protection, and primary-branch merge remain outside TASK-015.

These are explicit future durable/live-owner gates, not remaining defects in
the bounded pure/fake AC-29 contract.
