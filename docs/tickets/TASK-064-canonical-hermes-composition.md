---
ticket_id: TASK-064
spec_id: SPEC-002
spec_version: 30
module_id: latticed
constitution_version: 1.6
status: completed
parallel_safe: false
depends_on:
  - TASK-060
allowed_paths:
  - docs/tickets/TASK-064-canonical-hermes-composition.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/modules/latticed/MODULE_CONSTITUTION.md
  - docs/adr/ADR-021-latticed-delivery-composition-and-bounded-mcp.md
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/tests/composition.rs
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-064 Canonical Hermes composition

## Objective

Connect the existing production Hermes owner to canonical four-tool `latticed`
without changing MCP schemas or making task/status-only work depend on Hermes.
The integrated broker-root base is local commit `8bc5e977c05973d331001ee1144c54551cb89c7c`;
no repository ticket named TASK-063 is inferred.

## Acceptance Criteria

1. Exact process mode defaults to `TASK_ONLY`, accepts only `TASK_ONLY` or
   `PRODUCTION`, and redacts rejected values.
2. `PRODUCTION` is lazy. Only Delivery Run activates one production Hermes
   owner, and activation precedes writer effects.
3. Delivery Status and both Task tools never activate Hermes.
4. MCP termination explicitly reaps an activated owner. Teardown ambiguity
   overrides successful or failed stdio completion.
5. Existing four-tool discovery/schema, standalone lifecycle, and task-only
   tests remain unchanged.

## Non-Goals

No credential acquisition, live model request, public listener, new MCP tool,
PostgreSQL migration, push, merge, deployment, or release.

## Completion Evidence

- Canonical no-argument `latticed` accepts exact process-owned
  `LATTICE_HERMES_MODE=TASK_ONLY|PRODUCTION`; the default remains `TASK_ONLY`,
  while unknown values fail closed without being echoed.
- `PRODUCTION` holds an inactive owner until Delivery Run. Recording tests
  prove Delivery Run is the only activating tool, one launch attempt is reused,
  a failed launch is not retried, and Delivery Status plus both Task tools have
  zero activation. `TASK_ONLY` Delivery Run preserves its previous path.
- Canonical MCP shutdown explicitly invokes Hermes teardown once. A teardown
  ambiguity overrides stdio success or transport failure. The legacy observer
  entry retains its prior exit semantics.
- `cargo test --workspace --all-targets --locked` passed on the candidate tree;
  the final affected `cargo test -p lattice-runtime --all-targets --locked`
  passed 92 library, 20 composition, 1 coordination, 5 dispatch, 31 MCP, and
  1 task-control tests.
- `npm.cmd run verify` passed 48/48; `cargo fmt --all -- --check`,
  `git diff --check`, and `npm.cmd run check` passed.
- Strict runtime Clippy remains blocked only by 17 pre-existing diagnostics in
  unchanged `lattice-hermes-adapter` files. No suppression or unrelated repair
  was added for this task.
- Independent code, test, and architecture reviews reported no P0/P1/P2 after
  the recording lifecycle tests, TASK_ONLY regression, and contract alignment
  were completed. No credential was read, no Hermes or model request was
  launched, and no push, merge, deploy, or release ran.
