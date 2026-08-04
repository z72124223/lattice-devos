# TASK-010 Workflow Audit — 2026-07-29

## Target

- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base/HEAD before TASK-010: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Worktree state: intentionally dirty shared MVP-0 baseline; 47 folded status
  entries at audit time.
- Remote: none configured.

## Detected Controls

| Control | Status | Evidence |
|---|---|---|
| Repository rules | valid | root `AGENTS.md` |
| Plan | stale, then updated | TASK-009 was still marked CURRENT; PLANS now defines MVP-0 through MVP-3 and TASK-010 |
| Specification | partial, then updated | SPEC-002 v3 lacked MVP definitions; v4 defines them and records AC-23/24 evidence |
| Ticket | missing, then created | TASK-010 exact scope and allowed paths |
| Module governance | partial, then activated | legacy task-domain 1.0 replaced by approved V2 2.0; new lattice-cjson 1.0 |
| Architecture decisions | valid with clarification | ADR-004/005 plus ADR-008 mechanism/semantic ownership |
| Tests/build | available | Cargo workspace and preserved Node test suite |
| CI | documented only | local workflow exists; no remote run or branch enforcement |
| Reviews | required | independent code and architecture review after implementation |
| Merge controls | missing/unverified | no remote, branch protection, required review, or merge queue evidence |

## Current Evidence

- Workflow audit script detected Git, 1 instruction file, 3 plan files, 2
  specifications, 9 pre-TASK-010 tickets, 13 module-constitution files, 7 ADRs,
  12 test files, 6 build/test configs, 1 local CI config, and 11 review files.
- Active V2 and preserved V1 are separate worktrees on separate feature
  branches sharing base `06c3954`; neither may be reset, cleaned, or switched.
- TASK-008 and TASK-009 are completed and locally verified but uncommitted.
- No PostgreSQL or live external component is required by TASK-010.
- The mandatory automatic project router is currently missing at
  `C:\Users\f7212\OneDrive\文件\codex 個人化\scripts\codex-memory-router.mjs`;
  routing used direct repository evidence instead.

## Stage Classification

| Stage | Status before implementation |
|---|---|
| Repository/Git inspection | valid |
| Requirements | valid; direct user execution through MVP-3 |
| Material clarification | resolved from ADR-005, V2 proposal, current user preference, and bounded ADR-008 |
| Specification | valid as SPEC-002 v4 |
| Module constitutions | valid for lattice-cjson 1.0 and task-domain 2.0 |
| Ticket/worktree plan | valid; one current non-parallel TASK-010 |
| TDD | ready |
| Verification/review/integration | pending |

## Enforcement Truth

- Cargo/Node tests and local scripts are machine-enforced only when run.
- Ticket allowed paths, review order, and one-current-ticket policy are
  documented-only; no hook blocks violations.
- Remote CI, required reviews, branch protection, and merge queue are missing
  or unverified.
- Passing TASK-010 local tests cannot prove PostgreSQL durability, live
  providers, containment, release safety, or merge readiness.
