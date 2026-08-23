# ADR-024: Versioned ingress receipt handoff

- Status: superseded by ADR-025 before deployment
- Date: 2026-08-24
- Decision owner: user
- Related: ADR-023, Task Ledger 2.3

## Decision

An existing task record remains immutable when the canonical `latticed`
binary changes.  A successor binary may replay that record only after it
appends one closed handoff receipt that commits to the previous verified
ingress-profile commitment, the previous terminal receipt digest, the
successor ingress-profile commitment, and the fixed compatibility verifier
version.

The handoff is an internal Ledger event.  It adds no MCP tool, argument, or
caller authority.  It cannot create a task, change a result, authorize an
effect, or replace the original TaskCreated evidence.  Missing, duplicate,
out-of-order, substituted, or malformed handoffs fail closed.

## Consequences

Historical events, receipts, heads, and hash domains remain byte-identical.
New binaries cannot impersonate old binaries.  A future replay verifies the
historical chain followed by one explicitly recorded compatible successor.

This decision was not deployable against schema v5 and did not address the
live non-success terminal. ADR-025 records the replacement decision without
erasing this decision history.
