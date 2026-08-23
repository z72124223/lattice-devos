# ADR-025: Separate historical status from ingress authority

- Status: accepted; supersedes ADR-024 before deployment
- Date: 2026-08-24
- Decision owner: user
- Related: ADR-024, SPEC-008 v2, Task Ledger 2.5, latticed 2.6

## Context

ADR-024 selected an append-only successor-ingress handoff. The implementation
introduced `INGRESS_RECEIPT_HANDOFF` in source, but schema v5 could not persist
that event and the production record needing recovery was `FAILED`, while the
handoff admitted only `COMPLETED`. It therefore could not be installed as a
working repair.

## Decision

Runtime status observation and ingress mutation authority are separate.
`latticed` may use a dedicated read-only replay to report a fully verified
historical non-success terminal after binary commitment drift. The replay
validates the stored ingress audit and complete Task Ledger history but grants
no current-profile authority and writes nothing.

The unpersistable source-only handoff event is withdrawn before deployment.
Normal lifecycle replay and every mutation remain bound to the current ingress
commitment. A future writable successor handoff requires a separately approved,
deployable schema decision and is not implied by this read-only projection.

## Consequences

The existing database, events, receipts, heads, task references, MCP schemas,
Memory v3, and Writer Lease v2 remain unchanged. Historical non-success truth
becomes visible without allowing a successor binary to continue or rewrite it.
Historical `COMPLETED` and nonterminal streams remain current-profile-bound.
