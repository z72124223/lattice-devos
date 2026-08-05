# ADR-021: `latticed` Delivery Composition And Bounded MCP

- Status: accepted; explicitly approved by the user before the 2026-08-05
  implementation window
- Date: 2026-08-05
- Decision owner: user
- Related: SPEC-002 v25, ADR-004, ADR-005, ADR-006, ADR-015, TASK-032

## Context

TASK-032 proved a deterministic PostgreSQL/Git delivery checkpoint, but the
existing `apps/lattice-runtime` package also selected concrete Codex,
PostgreSQL, workspace, test, and Git effects. That dependency shape conflicts
with Orchestrator Runtime 2.0's injected-port boundary and cannot be the final
architecture.

The Codex App also needs a narrow MCP stdio entry for the same executable path.
Making that surface accept arbitrary task, path, shell, SQL, or credential
arguments would create a second general gateway and bypass the approved
OpenClaw boundary.

## Decision

1. `lattice-contracts` 1.10 owns immutable typed delivery request, stage
   evidence, terminal outcome/status, and receipt representations. These values
   grant no policy, persistence, workspace, Git, test, or provider authority.
2. `lattice-ports` 1.5 owns abstract `DeliveryLedgerPort`,
   `WorkspaceGitPort`, and `TestRunnerPort` traits in addition to the existing
   `CodexPort`. No port exposes a concrete driver, process, repository, command
   line, SQL statement, path supplied by an MCP caller, or credential.
3. `orchestrator-runtime` 2.1 is pure Rust effect coordination. With all
   dependencies injected, it may only order:
   - durable intent;
   - bounded workspace preparation;
   - one Codex effect;
   - bounded workspace changed-path inspection;
   - one fixed test;
   - Git commit only after the test and scope evidence pass;
   - durable terminal outcome and receipt.
4. A failed gate stops the sequence. Timeout, malformed protocol, failed test,
   foreign change, ambiguous Git state, unknown database commit, or any
   uncertain external effect cannot become success and requires typed failure
   or reconciliation.
5. `latticed` 1.0 is the sole normal composition root. The existing
   `apps/lattice-runtime` package implements it, constructs the pure
   orchestrator and concrete adapters, and owns the bounded MCP stdio
   transport. Concrete adapters never determine workflow order or call one
   another.
6. The existing `lattice-runtime` command is retained only as a compatibility
   wrapper over the identical `latticed` composition, ports, PostgreSQL truth,
   and status projection. It is not a second orchestrator or gateway.

## MCP Surface

The MCP server exposes exactly these tools:

- `lattice_delivery_run`;
- `lattice_delivery_status`.

Each tool has a zero-parameter object input schema with no properties and
`additionalProperties: false`. A call cannot provide shell text, SQL, a path,
a credential, provider configuration, arbitrary task content, or another tool
name. The bounded delivery binding is selected from process-start composition
configuration, not from tool arguments, and secrets never enter tool output.

The MCP surface is a Codex App entry to one preconfigured delivery operation;
it does not replace the six-action OpenClaw gateway defined by ADR-015 and does
not expose submit, plan, approval, rejection, arbitrary status targets, stop,
Guardian, release, deployment, or general administrative operations.

## Dependency Direction

```text
lattice-ports -> lattice-contracts

lattice-orchestrator
  -> lattice-contracts
  -> lattice-ports

concrete delivery adapters
  -> lattice-contracts
  -> lattice-ports

apps/lattice-runtime (`latticed` composition)
  -> lattice-orchestrator
  -> concrete delivery adapters
  -> bounded MCP stdio transport

`lattice-runtime` compatibility wrapper
  -> identical `latticed` composition
```

The orchestrator has no dependency on concrete adapters, MCP, PostgreSQL,
process, filesystem, test runner, or Git implementation. The compatibility
wrapper introduces no alternate call order or durable state.

## Compatibility And Evidence

- Existing scripted app-server evidence remains a test harness and must remain
  labeled `SCRIPTED_ACCEPTANCE`.
- Official-Codex completion requires a real bounded app-server turn plus exact
  changed-path, fixed-test, local-commit, durable-result, and restart/status
  replay evidence.
- Existing PostgreSQL Store 1.4 and Codex Adapter 1.0 contracts remain
  unchanged; composition wraps their public behavior through the new ports.
- OpenClaw remains the normal human gateway. No deployment, publication,
  payment, primary-branch merge, public listener, or protected release is
  authorized by this decision.

## Approval Record

The responsible user explicitly approved this versioned architecture revision
in the preceding task window, including the contract/port expansion, pure
orchestrator, `latticed` 1.0 composition root, fixed zero-parameter MCP tools,
and TASK-032 allowlist update. No additional routine approval gate remains for
this bounded local implementation.
