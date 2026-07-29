# LATTICE DevOS

**LATTICE DevOS v1.0 — Controlled Swarm** is an offline-first control core for
AI-assisted software development.

> One Gateway. One Truth. One Writer.

Phase 1 proves the workflow with a deterministic Fake Runtime. It does not call
a real model, network, account, Hostinger, OpenClaw runtime, or user project.

## Status

- Specification: `docs/specs/SPEC-001-controlled-swarm-core.md`
- Current plan: `PLANS.md`
- Current ticket: `docs/tickets/TASK-001-task-domain.md`
- Merge/deploy: not authorized

## Local Verification

Requirements:

- Node.js 24.15 or newer
- Git

Commands:

```text
npm run check
npm test
npm run verify
```

No third-party runtime package is required by the Phase 1 core.

## Safety Boundary

- Only the Implementer role may write product code.
- Execution and merge require separately bound approvals.
- The Task Ledger is the only durable control-plane truth.
- Git Scope Check is detection evidence, not a process sandbox.
- The OpenClaw plugin is an inert scaffold until a later live capability
  preflight passes.

