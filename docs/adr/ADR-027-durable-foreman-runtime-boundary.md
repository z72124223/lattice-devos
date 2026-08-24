# ADR-027: Durable foreman runtime boundary

- Status: accepted for TASK-105
- Date: 2026-08-25
- Related: ADR-021, ADR-024, ADR-025, ADR-026, SPEC-009, TASK-079, TASK-094, TASK-105

## Decision

Add one canonical MCP command, `lattice_foreman_checkpoint`, above the existing
Foreman State → Task Ledger → PostgreSQL schema-v6 path. It is a narrow product
entrypoint, not a new event or store. The legacy observer does not expose it.
Runtime Status remains zero-argument and projects only replay-verified state.

Exact retry is resolved from durable command/event/child replay before a new
Git or Writer observation. New writes follow acquire → append → known-success
release. Unknown append outcome retains the lease for reconciliation; unknown
release outcome returns reconciliation without repeating the append.

Migration is administrative only. `latticed --postgres-initialize` provisions
only roles/database/foundation; the official launcher must then invoke
`latticed --postgres-bootstrap`, which owns the Writer-v3-before-Store-v6 state
machine. Normal serving and tool calls verify current schema and never install
or migrate.

## Constitutional narrowing

ADR-024 and Task Ledger invariant 25 meant that the internal foreman event could
not silently widen MCP. TASK-105 intentionally adds one separately versioned,
closed MCP adapter after SPEC-009 approval; it does not change the Ledger event,
historical bytes, generic append API or legacy surface.

## Consequences

PostgreSQL remains the sole durable truth. Git observation is persisted evidence
inside the existing snapshot. Dashboard/status caches gain no authority. A
corrupt or unsupported replay prevents Runtime Status success and MCP serving.
