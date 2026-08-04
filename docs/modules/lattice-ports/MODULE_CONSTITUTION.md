---
module_id: lattice-ports
name: LATTICE I/O Ports
version: 1.4
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-02
---

## Mission

Define the abstract Rust traits through which orchestration reaches the gateway,
sole product-code writer, read-only knowledge lane, untrusted research lane,
and typed physical control store.

## Non-Goals

- Select or start OpenClaw, Codex, Graphify, Hermes, or PostgreSQL.
- Perform I/O, decide policy, own workflow order, or define domain transitions.
- Manufacture PostgreSQL durability or external component compatibility; a
  concrete Store may return a structurally classified physical receipt whose
  durability still requires its own implementation evidence.

## Owned Data

- Port traits, external-port errors, and a component-free inbound
  `GatewayServiceError`.
- No runtime, durable, product, credential, or provider-session data.

## Public Contracts

- `GatewayService` accepts server-derived peer context plus one complete typed
  request and returns a typed bound reply; codec errors remain outside the
  service call.
- `GatewayService` returns `GatewayServiceResult`: Rust-core routing or
  reply-binding failures cannot be attributed to an external component.
- `CodexPort` is the only product-code mutation lane contract.
- `GraphifyPort` returns derived read-only evidence.
- `HermesPort` returns untrusted candidate evidence.
- `ControlStore::transact` accepts one complete typed physical transaction and
  returns a typed terminal receipt or Store-specific error without defining
  domain legality.
- `ControlStore::current_head` takes mutable Store access and returns the
  independently retained physical head for one exact scope; this is not a
  domain-owner current head. The mutability is explicit because a synchronous
  driver query mutates connection state.
- Each trait returns its own evidence type so a provider cannot cross-label
  another component or authority boundary.

## Invariants

1. This crate depends only on `lattice-contracts`.
2. OpenClaw is an inbound gateway client, never a second control core or an
   outbound provider selected by orchestration.
3. Traits expose no concrete database, filesystem, or process type.
4. A port cannot return another lane's evidence type.
5. Port errors are explicit and unknown outcomes never imply success.
6. No adapter calls another adapter through these traits.
7. `GatewayService` never returns OpenClaw-produced evidence as a substitute
   for a Rust-core routing reply.
8. A `GatewayServiceError` has a stable kind/code but no `Component`; generic
   outbound adapter `PortError` values carry an external component, while the
   Store uses its complete Store-specific error type.
9. Store outcomes distinguish invalid/substituted, unauthorized/admission,
   capacity/overflow, unavailable/serialization, corruption, unknown outcome,
   and stale physical head without representing any as success. Stale head is
   a terminal denial receipt, not an error or applied mutation.
10. Store traits expose no SQL, table/schema/path, arbitrary row, driver,
    connection, migration, or domain-transition type.
11. A terminal physical receipt may classify its own fake or PostgreSQL
    durability, but the port never upgrades that evidence into domain legality,
    freshness, effect delivery, Guardian authority, or release authority.

## Allowed Dependencies

- `lattice-contracts`.
- Rust standard library.

## Forbidden Dependencies

- Concrete adapters, Orchestrator, policy, database drivers, network/process
  clients, model SDKs, credentials, and product repositories.

## Failure, Compatibility, And Migration

Rejected calls and exhausted/unknown outcomes return typed errors. Version 1.4
makes `current_head` explicitly mutable for synchronous adapters and permits
the unchanged typed receipt to carry its Contracts-owned live/durability
classification; no driver or concrete connection enters the trait. Version 1.3
replaces nominal `append(AppendCommand) -> ControlStoreEvidence` with typed
`transact(StoreTransactionRequest) -> StoreTransactionReceipt` and an exact
physical-head query. Store failures use a Store-specific typed error; no trait
claims durability or domain authority. Version 1.2
separates the inbound Rust-core `GatewayServiceError` from component-attributed
external `PortError` values and returns `GatewayServiceResult<GatewayReply>`.
Version 1.1 replaced the nominal `GatewayCommand -> GatewayEvidence` signature
with `(GatewayPeerContext, GatewayRequest) -> GatewayReply`; physical codec and
authentication remain outside this crate. Later signature or semantic changes
require a versioned amendment and coordinated consumer migration.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Port contract tests | `cargo test -p lattice-ports` | Engineering | yes |
| Store error/trait shape | complete transaction/current-head compile and failure matrix | Security review | yes |
| Dependency direction | Cargo metadata inspection | Architecture review | yes |
| Full Rust verification | workspace format, lint, and tests | Engineering | yes |

## Change Policy

Mission, trait signatures, error meaning, or dependency direction changes
require a versioned amendment, SPEC-002 trace, architecture review, and user
approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | SPEC-002 v3, ADR-004/006 | Inbound gateway plus four abstract outbound ports | User |
| 1.1 | 2026-08-01 | SPEC-002 v13, ADR-015, TASK-017 | Complete typed gateway peer/request/reply signature; contracts-only dependency retained | User MVP-3 execution directive |
| 1.2 | 2026-08-01 | SPEC-002 v14, ADR-015 review amendment, TASK-017 | Component-free Rust-core Gateway service error; external port attribution retained only for adapters/store | User MVP-3 execution directive |
| 1.3 | 2026-08-01 | SPEC-002 v15, ADR-016, TASK-018 | Complete typed Store transaction/current-head boundary and Store-specific failure semantics | User MVP-3 execution directive |
| 1.4 | 2026-08-02 | SPEC-002 v22, ADR-018, TASK-020 | Explicit mutable current-head query and live physical receipt semantics without exposing a driver | User MVP-3 execution directive |
