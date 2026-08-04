# ADR-004: Rust Core With Thin Polyglot Adapters

- Status: accepted; approved by user on 2026-07-29
- Date: 2026-07-29
- Decision owner: user
- Related: SPEC-002, ADR-003

## Context

The V1 Node.js core was chosen for a small offline scaffold. The user has now
explicitly requested a Rust and PostgreSQL platform that integrates OpenClaw,
Codex, Graphify, Hermes, and Codebase Memory.

Those upstream components are not all Rust. OpenClaw's native plugin boundary
is TypeScript/ESM, while Graphify and Hermes are Python applications. Rewriting
them would create forks and move LATTICE away from its control-plane mission.

## Proposed Decision

- Build the trusted LATTICE control plane as a Cargo workspace.
- Keep domain, policy, orchestration, store, workspace, memory, and supervision
  logic in Rust.
- Keep a minimal TypeScript/ESM OpenClaw plugin that translates authenticated
  user commands to versioned OS-local IPC.
- Treat Graphify and Hermes as separately pinned, least-privilege child
  processes behind Rust adapter traits.
- Treat Codex app-server as a separately versioned local protocol dependency.
- External adapters return versioned, schema-validated results. They do not
  receive direct control-plane database authority.
- Make fake adapters mandatory before any live binary preflight.
- Put all I/O traits in an explicit `lattice-ports` crate so domain ownership
  and concrete adapter dependencies cannot be confused.

Initial workspace proposal:

```text
crates/
  lattice-contracts
  lattice-ports
  lattice-policy
  lattice-orchestrator
  lattice-postgres
  lattice-workspace
  lattice-memory
  lattice-adapters
apps/
  latticed
  lattice-supervisor
plugins/
  openclaw-lattice
```

Provider adapters may begin as modules inside `lattice-adapters`; they should
be split into crates only when dependency or release isolation is demonstrated.

## Dependency Direction

In the diagram below, `A -> B` means **A depends on B**:

```text
lattice-ports -> lattice-contracts
lattice-policy -> lattice-contracts

lattice-orchestrator
  -> lattice-contracts
  -> lattice-policy
  -> lattice-ports

lattice-postgres/workspace/memory/artifact/provider-adapters
  -> lattice-ports
  -> lattice-contracts

latticed
  -> lattice-orchestrator
  -> concrete adapters
```

- Contracts and policy do not depend on ports or I/O adapters.
- `lattice-ports` depends only on contracts.
- Orchestration depends on contracts, policy, and ports, never concrete
  implementations.
- Concrete adapters implement ports and never depend on Orchestrator.
- `latticed` is the only normal composition root that selects concrete
  implementations.
- The OpenClaw plugin depends only on the public IPC schema.
- No external adapter depends on another external adapter.

## Compatibility

- ADR-003's Node-core decision is superseded for V2 implementation.
- ADR-003's evidence that OpenClaw requires its native plugin package shape is
  retained.
- V1 Node code remains a characterization oracle until Rust parity is verified.
- No dual-write or in-place language rewrite is permitted.

## Official Contract Evidence

- [OpenClaw Codex harness, v2026.7.1](https://github.com/openclaw/openclaw/blob/v2026.7.1/docs/plugins/codex-harness.md)
- [Codex app-server protocol, rust-v0.144.6](https://github.com/openai/codex/blob/rust-v0.144.6/codex-rs/app-server/README.md)
- [Graphify repository and runtime requirements, v0.9.24](https://github.com/Graphify-Labs/graphify/blob/v0.9.24/README.md)
- [Hermes programmatic integration, v2026.7.20](https://github.com/NousResearch/hermes-agent/blob/v2026.7.20/website/docs/developer-guide/programmatic-integration.md)
- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Dated component-contract evidence](../reviews/COMPONENT_CONTRACT_EVIDENCE_2026-07-29.md)

## Consequences

- Rust owns safety-critical behavior without pretending every upstream tool is
  Rust.
- Adapter crashes and version drift are localized.
- The build remains polyglot and will need explicit supply-chain/version
  evidence for each external component.
- Live compatibility remains unverified until separate authorized preflights.

## Approval Gate

Accepting this ADR authorizes constitution drafting and Rust ticket planning,
not installation, credentials, database creation, model calls, merge, release,
or deployment.
