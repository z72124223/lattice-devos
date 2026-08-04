---
module_id: gateway-ipc
name: LATTICE Gateway IPC
version: 1.1
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-01
---

## Mission

Own the bounded, versioned, canonical local gateway protocol and its pure
in-memory fake loopback without becoming task truth, orchestration, approval,
transport authentication, or an external effect owner.

## Non-Goals

- Start or connect to OpenClaw, a socket, named pipe, TCP listener, or daemon.
- Authenticate an operating-system peer or create live session authority.
- Validate Task Spec domain semantics or maintain task state.
- Access PostgreSQL, Git, files, product worktrees, providers, or credentials.
- Prove approval, interruption, lease release, completion, or protected release.

## Owned Data

- Canonical wire-frame layout, parser limits, hash subjects, codec enforcement,
  retry semantics, and the mapping of shared protocol values onto wire fields.
- Frame, depth, node, array, canonical-NFC, and encoded-response limits.
- `lattice-contracts` separately owns neutral in-process representations,
  shared protocol identifiers, and identifier/cursor/page constructor bounds;
  changing a shared value requires a coordinated versioned amendment.
- Mechanical request/reply and Task Spec document digest verification.
- Fake-only in-memory exact-command replay records (maximum 1,024) and fault
  scripts.
- No durable, product, credential, provider-session, or approval data.

## Public Contracts

- Encode and decode the six closed action-specific requests and typed replies.
- Reject non-canonical, duplicate-key, numeric, unknown-field/version/action,
  malformed, oversized, over-deep, and over-node frames before a port call.
- Reject non-NFC encoder/request/reply inputs before canonical hashing or
  normalized allocation, so a typed round trip cannot silently change identity.
- Keep server-derived peer context outside untrusted request bytes.
- Recompute Task Spec 2.1 canonical digest and binding fields mechanically;
  leave Task Domain validation to Task Domain.
- Exact fake replay returns the original terminal reply; changed content under
  one scoped command key returns stable substitution denial.
- Peer-role authorization precedes replay lookup. A narrower recovery peer
  cannot read a cached normal reply, and deterministic role denials do not
  poison an authorized peer's replay key.
- Recovery clients can use bounded status/task-stop only; normal submit/plan/
  approval remains OpenClaw-only.

## Invariants

1. No arbitrary action, SQL, shell, path, provider, daemon, or Guardian escape
   hatch exists in the schema.
2. A normal gateway request cannot represent protected-release authority.
3. `STOP_REQUESTED` is never reported as interruption, lease release, or
   terminal stop completion.
4. Raw Task Spec, proof, nonce, token, credential, path, or provider output is
   absent from errors, receipts, and `Debug`.
5. Fake context and replies cannot be labeled live or durable.
6. The fake performs no filesystem, database, process, network, Git, provider,
   credential, product, payment, publication, deployment, or release I/O.
7. IPC retains no mutable task truth; Orchestrator routes and PostgreSQL later
   persists authoritative receipts/events.
8. Reused task/snapshot/attempt identifiers are at most 256 bytes; freshness,
   authority, receipt, observation, and terminal evidence digests reject the
   all-zero sentinel.
9. A project-status reply cannot exceed either 100 items or the request's
   smaller page size. Reply structure is validated before canonical hashing.
10. Gateway service failures use component-free `GatewayServiceError`; a
    Rust-core routing failure is never labeled as OpenClaw-produced.

## Allowed Dependencies

- `lattice-contracts` for neutral immutable values.
- `lattice-ports` for the injected `GatewayService` boundary.
- `lattice-cjson` for canonical bytes and domain-separated SHA-256.
- Exact `serde`/`serde_json` versions for bounded parsing only.
- Exact `unicode-normalization` version for allocation-free NFC preflight.
- Rust standard library.

## Forbidden Dependencies

- Orchestrator implementation, Task Domain semantics, Policy, PostgreSQL,
  database/network/process/Git clients, OpenClaw SDK, provider/model SDKs,
  credentials, product repositories, and operating-system authentication.

## Failure, Compatibility, And Migration

Malformed or non-NFC transport values return a typed codec error and never call
the service. A legal request may return a typed denial or unknown outcome.
Unknown versions fail closed. Wire field, digest, parser-limit, action, or
retry-semantic changes require a new protocol version and compatibility
fixtures; neutral representation bounds require a coordinated Contracts
amendment. Live transports must reuse the schema without weakening it.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Canonical codec | request/reply golden and adversarial tests | Engineering | yes |
| Closed actions | six variant and action/payload tests | Engineering | yes |
| Binding | Task Spec, session, project/task/approval/stop substitution tests | Security review | yes |
| Replay/faults | exact retry, reuse denial, timeout/cancel/unavailable/ambiguous tests | Engineering | yes |
| No I/O | dependency and forbidden-source inspection | Architecture review | yes |
| Full verification | workspace format, Clippy, Rust and preserved Node tests | Engineering | yes |

## Change Policy

Protocol fields, encoding, hash domains, limits, peer trust, actions, reply
meaning, retry keys, dependency direction, or live/fake classification require
a versioned constitution amendment, SPEC/ADR trace, architecture review, and
responsible-user authorization.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-01 | SPEC-002 v13, ADR-015, TASK-017 | Pure bounded canonical gateway protocol and fake loopback | User MVP-3 execution directive |
| 1.1 | 2026-08-01 | SPEC-002 v14, ADR-015 review amendment, TASK-017 | NFC-preserving bounded encoding, truthful core-service errors, and explicit Contracts/wire ownership split | User MVP-3 execution directive |
