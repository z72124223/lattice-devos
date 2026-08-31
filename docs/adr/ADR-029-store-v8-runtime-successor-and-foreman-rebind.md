# ADR-029: Store-v8 runtime successor and Foreman rebind

- Status: accepted for the managed-foreman deployment repair
- Date: 2026-08-31
- Related: ADR-023, ADR-027, ADR-028, SPEC-009, SPEC-011

## Context

Store migration `0009` advanced the global schema to v8 for external verified
result adoption, but retained Store runtime functions still asserted the
eight-entry Store-v7 profile. The same retained functions were the only
ordinary-role path used to load database identity and retained history.
Consequently an exact nine-entry Store-v8 database could not construct a fresh
Runtime client. The existing Foreman extension was also immutably installed
against Store v7 and had no owner-controlled successor binding for Store v8.

## Decision

Append migration `0010_store_v8_runtime_successor`; never edit migrations
`0001` through `0009`. The exact current Store profile is schema v8 with ten
manifest entries. The exact nine-entry schema-v8 profile remains a supported
migration predecessor named `V8LegacyPrefix`, not a current Runtime profile.

Store verification classifies only exact retained histories and exact
principal/companion catalog profiles. For the optional Foreman companion, the
migrator may inspect owner tables directly; Runtime, Guardian, and ReadOnly use
the Foreman-owned extension-identity security-definer reader only after the
Store verifier has pinned the complete companion catalog digest. Unknown,
cross-paired, partial, future, or ACL-drifted profiles fail closed.

Writer v5 identity and its append-only ledger remain bound to their immutable
Store-v7 provenance. A Writer-owned Store-v8 successor asset replaces only the
fixed v5 runtime bind/load procedures after exact predecessor verification.
The read-only Writer bootstrap classifier distinguishes rebind-pending from
exact Store-v8-current catalog state; no Writer semantic row, fence, authority,
identity, or historical ledger row is rewritten.

Foreman keeps extension schema version 1 and its original Store-v7 installation
ledger row. Its owner-controlled Store-v8 rebind changes the singleton global
binding to the exact ten-entry Store-v8 manifest and appends exactly ledger
ordinal 2 with `REBOUND`. Runtime may use only that exact rebound identity.

Explicit `--postgres-bootstrap` is the sole product mutation path. One
session-level global advisory gate serializes the complete Store/Memory/Writer/
Foreman transition. The coordinator admits only the enumerated exact profile
matrix, changes admission to exact `STOPPED`/no-leader before owner mutation,
and leaves it stopped on any owner failure. After every owner gate succeeds it
restores the exact configured authority, releases migrator credentials, and
uses fresh Runtime-role clients for Task Ledger replay and Foreman verification.
A failure in that final fresh-runtime proof reports no readiness; because the
configured authority has already been restored, it is not described as a
post-stop owner failure.

Normal MCP startup and tool calls remain migration-free and verify-only.

## Consequences

- Existing Store, Writer, and Foreman receipts remain append-only and
  independently attributable to their owning generation.
- Exact legacy Store-v8 databases converge through one retryable product path;
  concurrent bootstrap processes serialize and terminal-current retries make
  zero durable changes.
- A Foreman rebind failure after Store/Writer successor mutation remains exact
  stopped state and is repairable without replaying or fabricating task truth.
- Store v8 does not authorize managed-task adoption, merge, push, deployment,
  release, or any new MCP tool.

## Required evidence

- Contract tests for immutable manifest bytes, role-aware Store closure, exact
  Writer and Foreman successor assets, and the closed composition matrix.
- Disposable PostgreSQL 17 migration from exact nine-entry Store v8, Foreman
  failure to exact stopped state, concurrent retry convergence, third-run
  idempotency, ACL/tamper checks, and fresh-process Runtime replay.
- Deployed direct-stdio `initialize`, exact seven-tool `tools/list`, and
  zero-argument `lattice_runtime_status` smoke from the configured environment.
