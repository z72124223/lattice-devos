---
module_id: openclaw-adapter
name: OpenClaw Adapter
version: 2.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-01
---

## Mission

Own the authenticated native OpenClaw `/lattice` package boundary and translate
its six normal user actions to the versioned local LATTICE IPC client without
duplicating Orchestrator, Codex ownership, policy, or task truth.

## Non-Goals

- Reimplement or own the Codex app-server harness/thread.
- Directly access PostgreSQL, Git, product paths, providers, credentials, or
  protected Guardian authority.
- Accept arbitrary SQL, shell, path, provider, or process commands.
- Claim live OpenClaw compatibility from TASK-017 pure Rust fake evidence.

## Owned Data

- Native plugin manifest/package identity and one authenticated `/lattice`
  registration.
- Thin parsing and display mapping for submit/plan/status/approve/reject/stop.
- Client-side schema/version and binary identity used by later live preflight.
- No task state, command authority receipt, approval proof, credential, or
  product data.

## Public Contracts

- Native plugin ID remains `lattice-devos`; `/lattice` has one authenticated
  owner and accepts arguments.
- Translate only the closed `gateway-ipc` schema and display typed replies.
- Never synthesize peer authentication context or approval authority from
  request fields.
- Normal approval/rejection only routes a bound challenge/presentation; a
  protected release requires the separate Guardian surface.
- Stop requests one exact task attempt and never claims the task is stopped.

## Invariants

1. One command registration and one normal gateway owner exist.
2. The plugin depends only on the generated public IPC schema/client artifact,
   not Orchestrator internals or concrete adapters.
3. It owns no writable Codex process/thread and no task/database truth.
4. It has no direct database, Git, product, provider, credential, payment,
   deployment, publication, or protected-release capability.
5. Static or fake checks never report live compatibility or authentication.

## Allowed Dependencies

- Focused official OpenClaw plugin entry API after exact-version preflight.
- Versioned generated `gateway-ipc` schema/client artifact.
- Minimal encoding/display dependencies approved by a later live ticket.

## Forbidden Dependencies

- Internal OpenClaw paths, third-party Codex harnesses, Orchestrator internals,
  database/Git/process/provider/model clients, credential stores, product
  repositories, Guardian keys, and mutable Task Ledger access.

## Failure, Compatibility, And Migration

Unknown IPC/plugin versions, unauthenticated commands, malformed replies,
daemon unavailability, disconnect, or ambiguous outcome fail closed. Live
compatibility requires exact plugin/host/schema/binary capability evidence in
MVP-2. The preserved V1 inert package evidence is characterization only.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Pure protocol fake | TASK-017 Rust loopback/contract suite | Engineering | yes for fake boundary |
| Package contract | exact manifest and one authenticated registration | Engineering | yes before live use |
| Schema identity | plugin and Rust schema/digest comparison | Security review | yes before live use |
| Live validation | pinned OpenClaw inspect/load/action/stop tests | MVP-2 preflight | yes before live claim |

## Change Policy

Plugin ID, command ownership, SDK imports, capabilities, live transport,
authentication mapping, compatibility floor, or protected-surface behavior
requires a versioned amendment, current official source verification,
security/architecture review, and responsible-user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | ADR-003 | Initial inert native plugin boundary | Current user task |
| 2.0 | 2026-08-01 | SPEC-002 v13, ADR-004/006/015, TASK-017 | Thin typed local IPC client; live package remains MVP-2 | User MVP-3 execution directive |

