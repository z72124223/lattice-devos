---
ticket_id: TASK-072
spec_id: SPEC-002
spec_version: 31
module_id: hermes-adapter
constitution_version: 1.1
status: ready
parallel_safe: false
depends_on:
  - TASK-071
allowed_paths:
  - docs/tickets/TASK-072-production-hermes-recovery-acceptance.md
  - crates/lattice-hermes-adapter/src/production.rs
  - crates/lattice-hermes-adapter/src/wsl_outer_runner.py
  - crates/lattice-hermes-adapter/tests/wsl_outer_runner_fixture.py
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-072 Prove production Hermes recovery without a model

## Objective

Exercise the exact frozen official Hermes runtime through its production
WSL/bubblewrap owner and a deterministic in-process Codex provider: retain one
known submitted run across a bounded observation timeout, then reconcile it on
the same `ProductionHermesPort` without another submission.

## Acceptance Criteria

1. The ignored Windows acceptance launches the exact pinned official-mode
   Hermes runtime inside WSL/bubblewrap, exposes only a loopback endpoint, and
   uses a synthetic local bearer plus an in-process fake Codex provider. It
   reads no external or provider credential and makes no real provider, model,
   or external-network request.
2. The fake provider acknowledges exactly one `turn/start` but withholds its
   terminal event. The first and only `run_reflection_evidence` returns one
   exact eligible observation timeout, while the active recovery receipt has a
   known run ID and exact request, session, input, and model binding.
3. After the test releases the terminal event, exactly one
   `reconcile_reflection_evidence` on the same bound port and receipt returns
   the strict canonical reflection with `RuntimeKind::Live`, exact invocation,
   input, and output-digest evidence, then clears the active receipt.
4. If the first timed-out Windows client has already closed, the outer WSL
   relay may ignore only a broken-pipe or connection-reset failure while
   writing the already-received inner response. Inner exchange, endpoint, and
   process failures still terminate fail closed, and the relay accepts the
   reconciliation request on a new loopback connection.
5. The observed fake Codex lifecycle contains exactly one `turn/start`, proving
   no resubmission. Explicit teardown invokes the underlying proxy control once
   and removes the owned isolation root.
6. The default adapter suite and the new exact ignored acceptance pass. No
   public API, frozen third-party runtime or inner-runner asset, dependency,
   schema, persistence, MCP, FullChain, database, credential, model,
   external-network, or ownership contract changes.

## Non-Goals

No canonical MCP or PostgreSQL end-to-end claim, real provider/model request,
external credential read, public listener, durable recovery, retry loop,
cross-process recovery, push, merge, deployment, payment, account change, or
release.
