# ADR-025: Writer Lease v3 and global-schema-v6 compatibility

- Status: accepted for TASK-087 by fixed-foreman delegation
- Date: 2026-08-21
- Related: ADR-005, ADR-011, ADR-012, ADR-019, ADR-023, TASK-076, TASK-079, TASK-087

## Decision

Global schema changes remain fail-closed with Writer Lease. Writer v2 stays
frozen to global schema 3/5. The Writer owner first creates a runtime-closed v3
bridge from exact schema-v5/Memory-v3/v2-current; Store may recognize only that
bridge while applying the approved schema-v6 successor; Writer finally
re-verifies and rebinds v3 before runtime resumes.

The sole reserved schema-v6 successor is exact v5 plus ordinal 7, ID
`0007_foreman_coordination`, path
`db/migrations/0007_foreman_coordination.sql`, stream
`FOREMAN_COORDINATION`, and event `FOREMAN_SNAPSHOT_RECORDED`. TASK-087 freezes
compatibility and catalog/ACL assertions but does not implement the event or
migration. Missing migration bytes, objects, ACL closure, stream/event identity,
or manifest evidence cannot be current.

The unchanged 15-scalar `writer_lease_assert_current_v1` stays the sole fencing
predicate inside a Task Ledger persistence transaction. A future foreman append
must invoke it before mutation and atomically commit its event, projection/
checkpoint and Store receipt. No compatibility profile, dashboard, cache,
diagnostic, or extension ledger becomes another truth.

## Consequences

- Accepted v1/v2 SQL and migrations `0001`-`0006` remain immutable.
- Bridge and pending states have zero runtime Writer authority.
- Unknown, skipped, duplicated or cross-generation histories fail closed.
- TASK-079 owns later event/Port/Postgres implementation and must supply exact
  `0007` bytes and measured catalog signatures before schema-v6 activation.
