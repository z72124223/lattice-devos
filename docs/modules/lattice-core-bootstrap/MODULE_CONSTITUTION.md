---
module_id: lattice-core-bootstrap
name: LATTICE Core Bootstrap
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Provide the smallest buildable Rust contract that identifies the approved
LATTICE DevOS platform components without starting processes or mutating data.

## Non-Goals

- Implement orchestration, policy, persistence, adapters, or self-upgrade.
- Connect to PostgreSQL, Codex, OpenClaw, Graphify, or Hermes.
- Freeze the final crate split proposed by ADR-004.

## Owned Data

- Compile-time platform name, component identifiers, and bootstrap modes.
- No durable or user-project data.

## Public Contracts

- Return the canonical platform name.
- Return every approved component exactly once with a bootstrap mode.
- Keep Graphify and Hermes read-only and keep the guardian approval-gated.

## Invariants

1. The platform is general-purpose and contains no website-specific behavior.
2. Bootstrap inspection performs no network, process, database, Git, or file
   mutation.
3. Component identifiers are stable within constitution version 1.0.

## Allowed Dependencies

- Rust standard library.

## Forbidden Dependencies

- Database drivers, network clients, model SDKs, shell execution, credentials,
  and concrete external adapters.

## Failure, Compatibility, And Migration

Unknown component identifiers fail at compile time. A public identifier or mode
change requires a versioned constitution amendment and compatibility note.

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
