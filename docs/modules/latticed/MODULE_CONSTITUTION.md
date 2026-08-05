---
module_id: latticed
name: LATTICE Normal Composition Root
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-05
---

## Mission

Provide the sole normal local composition root that constructs the pure Rust
orchestrator with concrete delivery adapters and exposes the bounded Codex App
MCP stdio surface. The existing `apps/lattice-runtime` package implements this
module.

## Non-Goals

- Own domain transitions, policy, workflow order, durable task truth, provider
  semantics, or product-code mutation logic.
- Become a second general human gateway, database authority, product writer,
  approval surface, Guardian, release controller, or deployment service.
- Accept arbitrary task, shell, SQL, path, credential, provider, process, Git,
  or test-command input through MCP.
- Duplicate orchestration or durable state for the `lattice-runtime`
  compatibility command.

## Owned Data

- Process-lifetime composition configuration, constructed adapter instances,
  the bounded MCP stdio session, and compatibility-entry routing.
- No Task Ledger, project, workspace, Git, test, Codex-thread, approval,
  credential, memory, or release truth. PostgreSQL and their domain owners
  retain durable and semantic authority.

## Public Contracts

- Construct one Orchestrator 2.1 instance with typed Contracts 1.10 / Ports 1.6
  implementations for the bounded TASK-032 delivery path.
- Expose exactly two MCP tools: `lattice_delivery_run` and
  `lattice_delivery_status`.
- Give each tool a zero-parameter object input schema with no properties and
  `additionalProperties: false`; reject any supplied property before dispatch.
- Select the bounded delivery binding from process-start composition
  configuration, never from MCP arguments. Credentials remain adapter-private
  process input and never enter tool schemas, diagnostics, or results.
- Map `lattice_delivery_run` to the one typed delivery coordinator and map
  `lattice_delivery_status` to the same durable ledger/status projection.
- Retain `lattice-runtime` as a compatibility wrapper that invokes the
  identical composition and returns equivalent typed evidence. It cannot
  select a different adapter, order, workspace, test, or truth source.
- Keep OpenClaw as the normal human gateway. This MCP surface provides no
  general submit/plan/approve/reject/stop or protected-release operation.

## Invariants

1. Exactly one normal composition root selects concrete implementations.
2. Orchestrator owns effect order; `latticed` and its adapters do not reorder,
   skip, or synthesize delivery stages.
3. Concrete adapters are constructed at the edge, implement typed ports, and
   never call one another.
4. MCP tool enumeration contains exactly the two approved names and both input
   schemas remain zero-parameter and closed to extra properties.
5. No MCP request can carry shell, SQL, path, credential, provider, arbitrary
   task, process, Git-command, or test-command data.
6. The MCP surface is not a second general gateway and cannot represent normal
   OpenClaw or protected Guardian actions.
7. `lattice-runtime` compatibility uses the same coordinator, adapter set,
   PostgreSQL truth, and status projection; it introduces no second runtime
   owner.
8. Startup, adapter, protocol, test, Git, database, or evidence ambiguity fails
   closed and never reports success.
9. Scripted protocol evidence remains visibly distinct from an official Codex
   app-server live turn.

## Allowed Dependencies

- `lattice-contracts` 1.10, `lattice-ports` 1.6, and
  `orchestrator-runtime` 2.1 public APIs.
- Concrete Codex, PostgreSQL Task Ledger, bounded workspace/Git, and fixed-test
  adapters required by TASK-032, only for construction and port
  implementation.
- Bounded stdio/JSON/MCP framing, process configuration, hashing, timeout, and
  diagnostics libraries required at the application edge.

## Forbidden Dependencies

- Orchestrator internals, direct policy/domain mutation, OpenClaw SDK as a
  second gateway, Graphify/Hermes/Memory mutation, Guardian trust roots,
  deployment/publication/payment code, or companion/playmate website code.
- Arbitrary shell/SQL execution, caller-selected filesystem/product paths,
  credential payloads, dynamic provider/tool registration, or an alternate
  durable ledger.
- Adapter-to-adapter calls or reverse dependencies from an adapter into the
  composition root.

## Failure, Compatibility, And Migration

Unknown startup configuration, extra MCP arguments, duplicate/unknown tools,
adapter construction failure, malformed framing, interrupted output, or an
uncertain downstream effect returns a bounded typed failure or reconciliation
result. No permissive fallback or generic command surface is allowed.

Version 1.0 is implemented in the existing `apps/lattice-runtime` package. The
`latticed` executable is the canonical normal entry; the existing
`lattice-runtime` name remains a compatibility wrapper over the same
composition. Removing or behaviorally diverging that wrapper requires a
versioned migration decision and compatibility evidence.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Composition direction | Cargo metadata proves orchestrator has no concrete dependency and only `latticed` selects adapters | Architecture review | yes |
| MCP tool closure | exact list and zero-property/additional-property rejection tests for both approved tools | Engineering | yes |
| Restricted input | shell/SQL/path/credential/provider/arbitrary-task property matrix is rejected before dispatch | Security review | yes |
| Compatibility parity | `latticed` and `lattice-runtime` wrapper use identical composition and status evidence | Compatibility review | yes |
| Delivery acceptance | official Codex turn, isolated scope/test/commit, durable outcome and separate restart/status replay | Engineering | yes |
| Failure closure | startup/framing/adapter/timeout/unknown-effect cases never report success | Engineering | yes |

## Change Policy

Composition ownership, tool names or schemas, compatibility-entry behavior,
adapter dependency direction, credential boundary, gateway separation, or
success/failure semantics require a versioned constitution amendment,
SPEC/ADR trace, architecture review, and responsible-user approval. This
constitution cannot be weakened merely to excuse implementation drift.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-05 | SPEC-002 v25, ADR-021, TASK-032 | Sole normal composition root in `apps/lattice-runtime`, two fixed zero-parameter MCP tools, and one shared `lattice-runtime` compatibility wrapper | User approval in preceding implementation window |
