# TASK-011 Independent Code And Security Review

## Target

- Worktree: `lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Specification: SPEC-002 version 6
- Ticket: TASK-011
- Reviewers: independent read-only code and security subagents

## Initial Blocked Findings And RED Evidence

The first implementation was `BLOCKED`. Every accepted behavioral finding was
reproduced before repair:

- protected-release approval did not bind the exact Guardian runtime identity;
- merge conflict/readiness was not a fresh Workspace-Git-owned fact bound to
  target head, reviewed subject, analysis, and scope evidence;
- resource usage lacked an independent expected Ledger
  stream/head/revision/effect-claim subject and could be replayed inside the
  same Task Spec;
- recovery could use a generic action or move normal runtime directly from
  `RECONCILIATION_REQUIRED` to `ACTIVE` without a resolved outcome;
- Guardian recovery did not bind its producer or durable saga/database/boot
  reconciliation;
- release-writer authority did not bind the requesting actor;
- rollback reused an activation-shaped subject, had no exact typed failed
  activation receipt, could point in the wrong slot direction, and did not
  require a strictly newer epoch;
- worker/merge external-cost denial did not have fixed precedence;
- Task Domain external-cost budgets had no accounting currency and later had
  decimal precision/scale bounds that differed from Policy;
- `HEAD`, `AUTO_MERGE`, bisect pseudo-refs, revision DWIM, tags, remotes, and
  shorthand refs could masquerade as local feature branches;
- on Windows case-insensitive Git storage, `refs/heads/Main` could resolve to
  the same physical ref as `refs/heads/main` while string comparison treated it
  as non-primary.

The review cycle also identified governance routing that still named Task
Domain/Policy 2.0 and SPEC v5. Active routing was synchronized to Task Domain
2.1, Policy 2.1, and SPEC v6 without rewriting historical 1.0-to-2.0 records.

## Resolutions

- Protected release binds `ProtectedReleaseSubject` to one exact
  `GuardianRuntimeSubject`.
- Merge consumes an exact, fresh `MergeReadinessFact`. Registry and Workspace
  Git facts carry owner-produced `GitRefIdentity` physical storage digests;
  Policy uses those identities for primary classification without guessing
  platform case rules.
- Resource-consuming gates carry an independent `ResourceObservationSubject`
  and compare all stream, head, revision, claim, owner, freshness, Task Spec,
  and currency fields.
- Normal recovery is stop-only and carries a typed effect, holder-death, or
  replaced-leadership resolution plus immutable evidence. Guardian restoration
  to `ACTIVE` binds the exact producer, protected activation receipt, and
  consistent durable saga/database/boot release state.
- `ProtectedActivationReceipt` replaces the unused rollback activation digest;
  rollback reverses exact slots and advances the epoch.
- Task Domain and Policy share a 256-byte, 127-integer-digit,
  128-fractional-digit decimal contract.
- Merge accepts only fully qualified `refs/heads/*` text and rejects
  pseudo-ref/DWIM namespaces before owner-identity comparison.
- Stable precedence rejects external cost before approval/resource failures in
  Worker Admission and Merge.

## Final Results

Code review: `PASS`, no actionable P0 through P3 finding.

Security review: `PASS`, zero P1 and zero P2 finding.

Independent evidence:

- Policy: 3 unit + 3 contract + 52 matrix + 8 V1 compatibility = 66 tests;
- Task Domain: 6 tests;
- full Rust workspace: 94 tests;
- preserved Node suite: 38 tests;
- locked all-target/all-feature Clippy with `-D warnings`: pass;
- `cargo fmt --check`: pass;
- `cargo metadata` and `cargo tree -p lattice-policy --locked`: approved pure
  dependency direction only;
- forbidden Policy I/O scan: zero matches;
- `git diff --check`: pass.

## Residual Owner-Module Gates

Policy evaluates typed facts but does not authenticate or persist them.
TASK-012 and later owner modules must produce physical Git ref identities,
Task-Ledger resource observations and atomic claims, Approval/Guardian receipts,
freshness, and nonce/epoch transitions. Scope Check must later emit its own
exact receipt for Workspace Git composition. These are explicit future
implementation gates, not unresolved TASK-011 code findings.
