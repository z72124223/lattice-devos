# ADR-026: Writer-owned schema-v6 rebind

- Status: accepted for TASK-094 bounded repair
- Date: 2026-08-24
- Related: ADR-005, ADR-012, ADR-023, ADR-025, TASK-079, TASK-087, TASK-094

## Decision

The Writer Lease adapter owns v3 bridge application and rebind. It exposes
typed administrative operations and one fixed zero-argument PostgreSQL
procedure, `writer_lease.writer_lease_rebind_v3()`. The Store runner may call
only that procedure after it has applied the exact ordinal-7 migration and
staged schema-v6 compatibility, while both remain in its existing transaction.
Exact-v6 retry calls the same idempotent procedure before it verifies the exact
v6 catalog/ACL closure; that retry does not add a Writer ledger row.

The Store must distinguish six-row `ExactV5Prefix` from seven-row
`ExactV6Full`. It cannot treat either as the other, install a Writer extension,
interpret Writer identity, ledgers, active heads, commands, transitions, create
Writer receipts, or reconstruct a Writer authority head. It may recognize only
pg_catalog procedure signature/owner/body/ACL/grant closure. V1/v2 SQL and
migrations 0001 through 0006 remain frozen.

## Consequences

- Bridge and pending profiles have zero runtime Writer authority.
- Exact retry is idempotent only for the same verified profile; altered history,
  identity, lease, fence, ACL, catalog or unknown commit outcome fails closed.
- Migration 0007, Task Ledger event meaning and foreman semantics remain
  TASK-079-owned. This decision repairs their blocked persistence boundary only.
- Live evidence must use a TASK-094 marker-owned disposable PostgreSQL cluster
  on a dynamic loopback port other than 5432, and prove its teardown.
