# TASK-012 Workflow Audit

- Date: 2026-07-29
- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Mode: read-only audit before TASK-012 application-code modification

## Confirmed Scope

TASK-012 is the pure/fake Project Registry owner slice. It may change shared
identity values, Registry lifecycle/receipts, and Policy receipt consumption.
It performs no real filesystem, Git, PostgreSQL, process, network, credential,
provider, publication, deployment, or protected action.

The V2 worktree is already dirty with the shared uncommitted MVP-0/TASK-009
through TASK-011 baseline. The separate V1 worktree remains present and was not
modified. No reset, clean, branch switch, commit, push, merge, or worktree
mutation occurred during this audit.

## Audit Evidence

- Global audit script: exit 0; Git repository confirmed; 3,150 scanned files;
  62 dirty paths at audit time.
- Current branch/HEAD: exact values above.
- Remote inventory: no remote/upstream.
- Worktrees: preserved V1 plus active V2 sibling.
- Baseline Rust:
  `cargo test --workspace --all-targets --all-features --locked`, exit 0,
  94 tests.
- Baseline Node: `npm.cmd run verify`, exit 0, 38 tests;
  `check=ok files=136 constitutions=12`.
- Router:
  `C:\Users\f7212\OneDrive\文件\codex 個人化\scripts\codex-memory-router.mjs`
  is absent; no project match was inferred from that missing entry point.
- Local memory quick search found no LATTICE/Registry entry and was not used as
  project evidence.

## Capability Classification

| Capability | Status | Evidence | Enforcement |
|---|---|---|---|
| Repository/rules/plan/handoff | present | AGENTS, PLANS, HANDOFF, Git checks | documented plus machine-observed |
| Requirements | present | TASK-012 handoff plus SPEC/proposal/ADR evidence | documented-only |
| Specification | partial before update | SPEC-002 v6 has AC-06 but no exact receipt/currentness contract | documented-only |
| Module constitution | missing before update | proposed Registry 1.0 only; no active file | missing |
| Ticket | missing before update | PLANS marker only | missing |
| TDD entry point | present | Cargo workspace and established focused/full commands | machine-executed locally |
| Project Registry crate | missing | no crate/source/tests | missing |
| Shared Registry receipt | missing | Policy-local booleans and types only | missing |
| Code/architecture review | missing for TASK-012 | prior TASK-011 reviews do not cover new change | missing |
| Local integration | available after implementation | full Rust/Node commands | unverified for TASK-012 |
| Remote CI/branch protection | missing/unverified | no remote/service evidence | missing |
| Merge authorization | blocked | no committed candidate or primary-merge authorization | documented and absent |

## Ownership And Dependency Finding

`ProjectClass`, `GitRefIdentity`, and `ProjectAuthorityFact` currently live in
Policy, while ADR-009 assigns project identity production to Registry and the
Policy constitution forbids direct Registry dependencies. Creating either
`policy -> registry` or `registry -> policy` would violate the approved graph.

The accepted correction, including the later independent-review amendment, is:

1. contracts 1.2 owns minimal neutral Project ID/class/lifecycle/ref, fixed
   producer/version, and task-agnostic receipt/full-head representations;
2. Registry 1.1 owns the full mutable identity/lifecycle aggregate,
   accepted/pending reservations, defensive blocking, and receipt issuance;
3. Policy 2.3 owns only the task-bound receipt/full-head sufficiency
   projection and requires an independent current owner lookup;
4. future Orchestrator/PostgreSQL authenticates and serializes the Registry
   current-head lookup while composing it with Task Spec;
5. no Registry inspection port is added in this ticket.

The original pre-code governance used contracts 1.1, Registry 1.0, and Policy
2.2. Review RED/GREEN evidence required the versioned 1.2/1.1/2.3 hardening;
the historical audit observations above remain baseline evidence, not the
final active contract.

## Actual Execution Order

1. Audit rules, plan/handoff, Git/worktrees, active specs/ADRs/constitutions,
   code/tests, and baseline commands.
2. Amend SPEC/ADR/constitutions and create TASK-012.
3. Add shared-contract RED, then GREEN.
4. Add Registry RED/GREEN cycles.
5. Add Policy composition RED/GREEN.
6. Run focused/full verification.
7. Run independent code/security/architecture review.
8. Verify local integration, update ledger/handoff, and keep merge blocked.

## Minimum Remaining Controls

- Machine-enforce the Registry public contract and lifecycle with tests.
- Add dependency and forbidden-I/O checks for the new crate.
- Preserve Scope Check, real Windows/Git identity, PostgreSQL durability, and
  restart behavior as explicit future gates.
- Remote CI, required reviews, branch protection, and a committed integration
  candidate remain missing; TASK-012 cannot be called merge-ready.
