---
ticket_id: TASK-067
spec_id: SPEC-002
spec_version: 31
module_id: latticed
constitution_version: 1.7
status: completed
parallel_safe: false
depends_on:
  - TASK-066
allowed_paths:
  - docs/tickets/TASK-067-canonical-hermes-no-model-integration.md
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/tests/composition.rs
  - crates/lattice-hermes-adapter/src/production.rs
  - crates/lattice-hermes-adapter/tests/reflection_api.rs
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-067 Canonical Hermes no-model integration

## Objective

Close a two-leg compositional acceptance at the existing concrete production
owner boundary. TASK-064 and TASK-066 prove that canonical Delivery Run alone
lazily readies and seals `FullChainHermes` before writer or persistence effects.
The adapter leg directly exercises the exact pinned local Hermes v2026.8.3
`ProductionHermesRunner` -> `ProductionHermesPort` lifecycle against the
deterministic in-process no-model Codex provider. This ticket does not claim one
live canonical MCP/PostgreSQL execution.

## Acceptance Criteria

1. The canonical leg keeps production startup lazy: only Delivery Run activates
   Hermes, while Status and Task tools launch no Hermes process.
2. The adapter leg starts the exact pinned official-mode runtime on a loopback
   endpoint and reaches the in-process fake provider without reading an
   environment, file, or external-provider credential and without calling an
   external model.
3. The adapter leg validates schema plus input/output digest and
   `RuntimeKind::Live`; the canonical leg validates exact graph/provenance
   binding through `ProductionHermesOutput` before persistence authority.
4. Success and failure each call the underlying provider teardown exactly once
   and remove the owned run root. Repeated teardown replays the same ambiguity,
   and canonical teardown ambiguity remains fail-closed.
5. No public test-support feature, Cargo or MCP/schema change, database
   ownership change, non-loopback/external network, real provider/model call,
   external credential read, push, merge, deployment, or release occurs.

## Non-Goals

No single-process live canonical Delivery Run with PostgreSQL, Codex, or
Graphify is claimed. No real provider/model, external credential, public
network, PostgreSQL restart, push, merge, deployment, payment, account change,
or release.

## Completion Evidence

- Canonical leg: TASK-066 commit `41a78cf` proves Delivery Run lazy activation,
  graph/provenance validation before persistence, exact receipt replay, fresh
  Status with zero Hermes activation, and fail-closed teardown precedence.
- Adapter success leg:
  `production::proxy_host_tests::official_hermes_gateway_reaches_interactive_fake_codex_without_model`
  passed with `--ignored --exact`. It exercised the exact pinned WSL2,
  bubblewrap, and Hermes v2026.8.3 runtime, a loopback-only endpoint, the
  in-process fake Codex protocol, `RuntimeKind::Live`, schema/digest binding,
  exactly one provider teardown, and removal of the owned root.
- Adapter failure leg:
  `production::proxy_host_tests::official_hermes_gateway_reports_failed_fake_codex_turn_without_model`
  passed with `--ignored --exact`. It retained the fixed failed-turn
  classification, emitted no reflection, performed exactly one provider
  teardown, and removed the owned root.
- `production::proxy_host_tests::codex_proxy_teardown_invokes_the_owned_control_once_and_replays_ambiguity`
  proves success and ambiguity invoke the owned control once and replay the
  same result. The startup-timeout cleanup test also passed.
- Adapter all-targets passed 81 tests plus 4 preparation tests with 9 explicit
  live-only ignores. Runtime canonical focused tests passed 9/9. Format and
  diff checks passed. No external credential, real provider/model, public
  network, push, merge, deployment, payment, account, or release action ran.
