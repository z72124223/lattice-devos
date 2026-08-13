---
ticket_id: TASK-066
spec_id: SPEC-002
spec_version: 31
module_id: latticed
constitution_version: 1.7
status: ready
parallel_safe: false
depends_on:
  - TASK-065
allowed_paths:
  - docs/tickets/TASK-066-canonical-hermes-reflection-replay.md
  - apps/lattice-runtime/src/composition.rs
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-066 Canonical Hermes reflection replay

## Objective

Prove the canonical production composition stitches one lazy-ready Hermes
reflection to exact graph-receipt persistence and replay, while a fresh Status
read performs zero Hermes activation, research, or persistence.

## Acceptance Criteria

1. A deterministic in-memory fixture observes exact Run order: ready, load
   miss, one canonical research call, persist, and byte-equal reload.
2. A fresh Status fixture loads the same candidate and performs zero ready,
   research, and persist calls.
3. A successfully bound Hermes owner remains production-sealed.
4. Production PostgreSQL and Hermes adapters keep their existing call paths;
   the seam adds no public API, process, network, credential, MCP, or schema.

## Non-Goals

No real database, Hermes, provider/model, credential, external network, MCP
change, push, merge, deployment, or release.
