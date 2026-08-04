---
module_id: lattice-cli
name: LATTICE Recovery CLI
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Expose a local inspection and recovery command that renders the inert bootstrap
manifest through the same public core contract.

## Non-Goals

- Become a second normal human gateway; OpenClaw remains the planned gateway.
- Execute tasks, approve work, mutate PostgreSQL, or call external components.
- Install, authenticate, publish, or deploy anything.

## Owned Data

- Command-line argument parsing and text rendering.
- No durable state, credentials, approvals, or project data.

## Public Contracts

- `lattice status` renders the platform name and component bootstrap modes.
- Unknown commands fail with a non-zero exit and a short usage message.

## Invariants

1. Status is read-only and deterministic.
2. The CLI cannot grant authority or bypass OpenClaw/guardian gates.
3. Output is derived only from the public core bootstrap contract.
4. Direct core linkage is limited to the inert bootstrap manifest; any future
   operational command must use the approved local IPC boundary.

## Allowed Dependencies

- `lattice-core`.
- Rust standard library.

## Forbidden Dependencies

- Direct PostgreSQL, Codex, OpenClaw, Graphify, Hermes, Git, network, process,
  credential, or release dependencies.

## Failure, Compatibility, And Migration

Unsupported arguments return a stable failure code. Output changes must preserve
machine-readable component identifiers or introduce a versioned format. Any
operational command requires a versioned constitution amendment and must use
the approved local IPC instead of direct runtime authority.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Rendering tests | `cargo test -p lattice-cli` | Engineering | yes |
| Executable smoke check | `cargo run -p lattice-cli -- status` | Engineering | yes |
| Dependency review | Cargo metadata and architecture review | Architecture review | yes |

## Change Policy

New commands, mutation authority, gateway behavior, dependencies, or output
contracts require a versioned amendment, specification update, and user
approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-002, PLANS Step 4 | Read-only bootstrap status CLI | User |
