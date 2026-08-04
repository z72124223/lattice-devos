# TASK-015 Workflow Audit

- Date: 2026-07-29
- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Mode: read-only audit before TASK-015 application-code modification

## Confirmed Slice

TASK-015 freezes Approval Verifier 1.0 as a pure Rust semantic owner with a
deterministic visibly non-durable fake. It replaces Policy's caller-owned
approval verification/currentness/self-approval verdicts with a fixed-owner
receipt plus an independently queried current head.

It also removes caller-owned R3 review Booleans. Approval Verifier does not own
independent-review meaning, so R3 remains fail-closed until a later Review
Runtime owner receipt/current-head ticket.

This ticket performs no OS authentication, real cryptography/key access,
PostgreSQL, filesystem, Git, process, network, credential, provider, payment,
publication, deployment, product-repository, or protected-release action.

## Baseline Evidence

- TASK-014 handoff, PLANS Step 5, SPEC-002 v10, ADR-007/008/009/013, the V2
  amendment proposal, module constitutions, and active Policy approval paths
  were inspected.
- The configured local project-router entry point remains absent; repository
  PLANS and HANDOFF provide the clear project match.
- `PLANS.md` has exactly one current marker for TASK-015.
- Baseline full Rust workspace passes 180 tests.
- Baseline Policy passes 81 tests.
- Preserved Node verification passes 38 tests; project check inspects 167 files
  and 14 constitutions before TASK-015 governance files.
- The shared V2 worktree remains dirty and uncommitted. No reset, clean, branch
  switch, commit, push, merge, deployment, or external action occurred.

## Security Gap

`ApprovalFact` is entirely caller constructible. Policy accepts:

- caller subject, authority, origin, actor/channel/session, nonce, and time
  strings;
- arbitrary unused `subject_digest`;
- `subject_verified`, `identity_verified`, `fresh`, `nonce_available`, and
  `self_approved` Booleans;
- `ReviewChecks.security` and `.architecture` Booleans.

Policy only checks timestamps for non-empty text. Current tests can use a
constant unrelated digest and flip Booleans to produce approval. This is a P1
before live approval, database, or Guardian integration.

## Capability Classification

| Capability | Status before TASK-015 | Evidence | Enforcement |
|---|---|---|---|
| Repository/rules/plan/handoff | valid | TASK-014 closure and current Git state | documented plus machine-observed |
| Complete typed approval subject | partial | Policy-local public structs | caller-owned |
| Subject canonical hash | missing | supplied digest is not checked | missing |
| Challenge/proof owner | missing | no Approval Verifier crate | missing |
| Nonce binding/currentness | unsafe | caller Boolean | missing |
| Expiry | unsafe | non-empty strings plus caller freshness Boolean | missing |
| Self-approval protection | unsafe | caller Boolean | missing |
| R3 independent review | unsafe | two caller Booleans | missing |
| Protected/normal trust separation | partial | Policy enum comparison only | caller-constructible |
| PostgreSQL atomic claim | deferred | Step 6/Guardian design only | missing |
| Remote CI/branch protection | missing/unverified | no remote configured | missing |
| Primary merge authorization | blocked | no committed candidate/authorization | absent |

## Ownership Decisions

1. Contracts 1.5 carries complete immutable approval subject and receipt/head
   representations; it owns no hashing, verification, lifecycle, or current
   state.
2. Approval Verifier owns subject canonicalization, challenge/proof, nonce
   binding, time/currentness, exact retry, aggregate replay, and pure claim
   preconditions.
3. Policy 2.6 owns only approval requirement and sufficiency. It has no normal
   Approval Verifier or cjson dependency.
4. `receipt.head()` is not currentness. Policy requires an independently
   obtained available head.
5. Normal claim mutation belongs inside the future approved transition/effect
   PostgreSQL transaction.
6. Protected release claim belongs only to Guardian `claim_activation`,
   atomically with nonce consumption, `ACTIVATION_CLAIMED`, and `DRAINING`.
7. Approval Verifier cannot turn review Booleans into independent review
   authority. R3 denies until Review Runtime has its own owner contract.

No unresolved user decision remains in this bounded pure/fake slice.

## Execution Order

1. Activate SPEC-002 v11, ADR-013, Approval Verifier 1.0, Contracts 1.5,
   Policy 2.6, and TASK-015 before code.
2. Move the complete neutral typed approval subject graph to Contracts and
   preserve Policy source compatibility through re-exports where safe.
3. Add stateful pure planner/verifier/fake RED/GREEN for challenge, verify,
   current head, normal claim, retry, raw replay, and checkpoint.
4. Replace Policy approval facts with receipt/current-head composition and
   remove all caller approval/review verdict Booleans.
5. Run focused/full tests, secret/dependency/no-I/O/governance checks.
6. Complete independent code/security and architecture reviews; reproduce
   every accepted finding RED before repair.
7. Write integration, workflow ledger, ticket closure, PLANS, and HANDOFF.

## Minimum Remaining Controls

- Full typed-subject and receipt/head substitution matrices.
- Canonical time, identity/trust lane, nonce binding, exact retry, denial-tail,
  raw replay, and rollback checkpoint regressions.
- Actual fake-owner Policy composition; no hand-built authority fact in
  positive tests.
- Explicit R3 fail-closed tests until Review Runtime owner exists.
- Live OS/crypto trust, PostgreSQL uniqueness/clock/transaction/restart,
  OpenClaw IPC, Review Runtime, and Guardian activation remain open.
