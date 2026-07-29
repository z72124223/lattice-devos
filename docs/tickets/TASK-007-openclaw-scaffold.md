---
ticket_id: TASK-007
spec_id: SPEC-001
module_id: openclaw-adapter
constitution_version: 1.0
status: blocked
parallel_safe: false
depends_on:
  - TASK-006
allowed_paths:
  - plugins/lattice-devos/**
  - scripts/check-project.mjs
  - test/openclaw-scaffold.test.js
  - PLANS.md
  - docs/tickets/TASK-007-openclaw-scaffold.md
  - docs/workflow/WORKFLOW_LEDGER.md
likely_files:
  - plugins/lattice-devos/openclaw.plugin.json
  - plugins/lattice-devos/package.json
  - plugins/lattice-devos/src/index.ts
  - plugins/lattice-devos/dist/index.js
  - plugins/lattice-devos/skills/lattice-workflow/SKILL.md
  - test/openclaw-scaffold.test.js
branch: feature/phase1-controlled-swarm
---

## Objective

Deliver the current-format native OpenClaw plugin/skill scaffold with exactly
one authenticated `/lattice` command that remains inert in Phase 1.

## Acceptance Criteria

- [ ] SPEC-001 AC-11.
- [ ] Manifest/package/source/runtime entry IDs and paths agree.
- [ ] No `.codex-plugin` dual-format marker exists in the native plugin.
- [ ] Fake registration proves command name, auth, args, and inert result.
- [ ] Live/runtime status remains explicitly unverified.

## Non-Goals

- Install OpenClaw, load the plugin, invoke Codex, connect a model, authenticate,
  deploy, publish, or declare a host compatibility floor.

## Module And Constitution Constraints

Use `openclaw-adapter` v1.0. The plugin cannot access Task Ledger/Git/Runtime
directly and cannot reimplement the official Codex harness.

## Dependencies And Overlap

Blocked on the stable Orchestrator boundary even though Phase 1 remains inert.
Not parallel-safe because root project checks gain plugin consistency rules.

## TDD Behaviors

1. Reject missing/mismatched manifest/package entries.
2. Register exactly one `/lattice` command via fake plugin API.
3. Require auth and accept args.
4. Return an explicit no-action Phase 1 result.
5. Verify non-user-invocable `lattice-workflow` skill metadata.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Focused scaffold test | `node --test test/openclaw-scaffold.test.js` | exit 0 |
| Project consistency | `npm run check` | exit 0 |
| Live plugin validation | target OpenClaw Phase 3 | unverified and not run |

## Human Gate

Target OpenClaw/Hostinger live validation is required before deployment.

