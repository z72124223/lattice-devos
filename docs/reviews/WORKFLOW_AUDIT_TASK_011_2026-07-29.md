# TASK-011 Workflow Audit

## Scope And Identity

- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Requested slice: pure Rust Policy Engine V2
- Overall goal: MVP-1 through MVP-3 without any unrelated website scope
- Audit mode: read-only

The automatic project/memory router could not run because
`C:\Users\f7212\OneDrive\文件\codex 個人化\scripts\codex-memory-router.mjs`
does not exist. Repository-local PLANS, HANDOFF, specifications, ADRs,
constitutions, tickets, source, tests, and current command output were used as
the authoritative continuation evidence.

## Current Repository Evidence

- The global workflow audit script reports a Git repository on the expected
  branch with 55 folded dirty paths and 2,354 scanned files after bulk
  build/dependency exclusions.
- The dirty state is the preserved shared TASK-004 through TASK-010 baseline.
  No reset, clean, branch switch, bulk move, or destructive operation is safe.
- HANDOFF records TASK-010 as complete and TASK-011 as the exact next bounded
  slice.
- `cargo test --workspace --locked` passes 28 Rust tests.
- `npm.cmd run verify` passes 38 preserved Node tests and reports
  `check=ok files=118 constitutions=12`.
- No Git remote or upstream is configured; remote CI, required checks, branch
  protection, and merge readiness remain unavailable.

## Capability Classification

| Stage | Status | Evidence | Gate strength |
|---|---|---|---|
| Repository inspection | valid | audit script, Git/worktree state, HANDOFF, PLANS | machine-observed plus documented baseline |
| Requirements clarification | valid | SPEC-002, ADR-004/005/006/007, V2 amendment, user execution directive | documented-only |
| Specification | partial | SPEC-002 v4 requires Policy V2 but lacks its exact observable contract | documented-only |
| Module constitution | stale | policy-engine 1.0 is the V1/Phase-1 contract; approved proposal requires 2.0 | documented-only |
| Ticket plan | missing | TASK-011 does not yet exist | missing |
| Branch/worktree plan | valid | dedicated V2 worktree; V1 worktree preserved | machine-observed |
| TDD implementation | missing | no `lattice-policy` crate or Rust policy tests | missing |
| Focused verification | missing | no Rust policy test command | missing |
| Full verification | valid baseline | Rust 28 and Node 38 pass before TASK-011 | machine-executed locally |
| Code review | missing | no TASK-011 implementation | missing |
| Architecture review | required/missing | public security contract and dependency edge will change | documented requirement |
| Integration verification | missing | no TASK-011 result | missing |
| CI and merge authorization | blocked | no remote, policy evidence, or committed candidate | missing/unverified |

## Requirements And Design Findings

- No material product question requires a user pause. The current spec and ADRs
  already require pure deterministic default deny, registered-project
  isolation, one Implementer, capability drift denial, routine-policy versus
  protected authority separation, and no I/O.
- `grill-me` is skipped because repository evidence resolves behavior and
  protected boundaries; the skip is not a claim that live authority exists.
- V1 Policy has useful characterization evidence but unsafe behavior:
  - risk and requested capabilities do not affect authorization;
  - missing `spec` can match missing lease/approval subject fields and allow;
  - merge approval does not bind project identity or Task Spec hash;
  - Policy checks but does not consume one-use nonces.
- Those behaviors are vulnerability evidence, not V2 compatibility
  requirements.
- ADR-004 and the V2 dependency diagram say
  `lattice-policy -> lattice-contracts`, while Task Domain owns the V2 risk,
  capability, state, network, deployment, approval-requirement, and Task Spec
  types. The V1 policy constitution already permits Task Domain.
- TASK-011 must resolve that drift explicitly rather than duplicate enums or
  pass untyped strings. ADR-009 will make the acyclic read-only edge
  `lattice-policy -> lattice-task-domain + lattice-contracts` authoritative.

## Actual Execution Order

1. Global/repository instructions and current HANDOFF/PLANS.
2. Current SPEC/ADRs/constitutions and V1 policy oracle.
3. Versioned ADR-009, Policy 2.0 constitution, SPEC-002 update, and TASK-011.
4. One public-contract RED/GREEN behavior at a time.
5. Focused Policy tests, then locked full Rust and preserved Node gates.
6. Independent code review.
7. Independent architecture/security-boundary review.
8. Local integration assessment; remote CI and merge remain separate.
9. Workflow ledger and HANDOFF.

## Enforcement Truth And Skip Risks

- Machine-enforced locally: Rust type system, exact Cargo lock, focused/full
  tests, format, Clippy, project checker, and diff hygiene once connected.
- Documented-only: module semantic ownership, forbidden I/O classes,
  One Gateway/Truth/Writer, review sequence, and protected-surface provenance.
- Missing: dedicated architecture/dependency-policy linter, remote CI evidence,
  branch protection, and a committed integration candidate.
- Unverified/out of TASK-011: Project Registry, Approval Verifier, Writer
  Lease, PostgreSQL runtime admission, real capability observations, and every
  live provider.
- Main bypass risk: treating caller-supplied authority facts as real authority.
  TASK-011 may decide their sufficiency but must never claim to authenticate,
  persist, consume, or make them current.

## Minimum Controls For This Ticket

- Activate Policy Engine 2.0 before code.
- Require a constructed `TaskSpec`; missing subjects deny.
- Use Task Domain enums directly rather than duplicated wire values.
- Keep external authority facts typed, exact-subject-bound, and explicitly
  unverified outside their future owner modules.
- Test risk/capability/project/snapshot/runtime/approval/writer/resource and
  protected-action matrices.
- Preserve V1 oracle evidence only under a named compatibility namespace.
- Run independent code and architecture review before integration assessment.
