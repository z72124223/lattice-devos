---
module_id: lattice-core-bootstrap
name: LATTICE Core Bootstrap
version: 2.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-21
---

## Mission

Provide the smallest buildable Rust contract that identifies retained LATTICE
components, their authority mode, and whether each component is core,
optional, deferred, or retired without starting processes or mutating data.

## Non-Goals

- Implement orchestration, policy, persistence, adapters, or self-upgrade.
- Connect to PostgreSQL, Codex, OpenClaw, Graphify, or Hermes.
- Freeze the final crate split proposed by ADR-004.
- Make an optional, deferred, or retired component a core release dependency.

## Owned Data

- Compile-time platform name, component identifiers, and bootstrap modes.
- No durable or user-project data.

## Public Contracts

- Return the canonical platform name.
- Return every retained component exactly once with a bootstrap mode and
  lifecycle tier.
- Keep Codex, the Rust core, and minimal PostgreSQL truth in the core tier.
- Keep Graphify and Codebase Memory optional, Hermes and Guardian deferred,
  and OpenClaw retired.

## Invariants

1. The platform is general-purpose and contains no website-specific behavior.
2. Bootstrap inspection performs no network, process, database, Git, or file
   mutation.
3. Lifecycle tier does not grant authority and does not delete source history.
4. Component identifiers remain stable across the 1.x to 2.0 migration.

## Allowed Dependencies

- Rust standard library.

## Forbidden Dependencies

- Database drivers, network clients, model SDKs, shell execution, credentials,
  and concrete external adapters.

## Failure, Compatibility, And Migration

Unknown component identifiers and lifecycle tiers fail at compile time. The
2.0 status output adds a stable `tier` field while retaining every component ID
and authority mode.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Contract test | `cargo test -p lattice-core` | Engineering | yes |
| Formatting/lint | workspace Cargo checks | Engineering | yes |
| No external dependencies | manifest inspection | Architecture review | yes |

## Change Policy

Mission, public identifiers, modes, dependency policy, or invariants require a
versioned amendment, SPEC-002 trace, architecture review, and user approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-002, ADR-004/006 | Minimal inert Rust platform contract | User |
| 2.0 | 2026-08-21 | Product simplification decision | Separate the small MVP from optional, deferred, and retired retained modules | User |
