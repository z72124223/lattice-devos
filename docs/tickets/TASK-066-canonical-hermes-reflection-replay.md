---
ticket_id: TASK-066
spec_id: SPEC-002
spec_version: 31
module_id: latticed
constitution_version: 1.7
status: completed
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

## Completion Evidence

- The canonical production path now delegates through one private seam while
  preserving the existing PostgreSQL load, graph-receipt load, Hermes
  research, persistence, and exact reload order.
- A deterministic fixture observes `Ready`, reflection miss, graph receipt,
  one research, one persist, and exact reload. The persisted receipt is then
  loaded by a fresh unsealed Status fixture with zero ready or research calls
  and no persistence surface.
- A substituted reload fails closed as `HermesReceiptRead`; a successfully
  bound production owner remains sealed until teardown.
- `cargo test -p lattice-runtime --all-targets --locked -- --test-threads=1`
  passed 96 library, 20 composition, 1 coordination, 5 dispatch, 31 MCP, and
  1 task-control tests. The parallel run exposed one pre-existing polling-count
  timing assertion; its exact rerun passed. Format and diff checks passed.
- Independent code and architecture reviews reported no P0, P1, or P2.
  No credential, database, Hermes/model, network, push, merge, deployment, or
  release action occurred.
