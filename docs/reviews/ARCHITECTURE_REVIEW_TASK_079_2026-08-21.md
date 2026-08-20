# TASK-079 architecture review — 2026-08-21

Trigger: new snapshot module and proposed durable control-plane schema/event.

The implemented `foreman-state` crate remains pure and has no I/O, dashboard,
Git, database, process, scheduler, MCP, or Writer Lease dependency. It is
compatible with TASK-048 as a read-only identity input and TASK-049 as a
read-only archive consumer. TASK-078 exporter files are unchanged.

The `lattice.foreman-epistemic/1.0` reference schema is separately typed and
expiring: it carries opaque digest pointers plus confidence and refresh metadata,
not lifecycle transitions. This preserves the authority boundary; any learning
or promotion remains an explicit TASK-084 dependency.

Integration is **BLOCKED**. The existing Task Ledger only accepts a `TASK`
stream and closed event kinds; `Diagnostic` is non-authoritative and cannot be
repurposed for the snapshot. The next slice must version and test a fixed
control-stream/event, a typed Port, Postgres Store physical row/function and
same-transaction Writer Lease/fencing assertion. A standalone table, dashboard
JSON, chat record, or cache would violate One Gateway. One Truth. One Writer.

## Durable-binding re-audit — 2026-08-21

**P1 — global schema-v6 would invalidate the verified Writer Lease v2 profile.**
`crates/lattice-postgres-store/src/migrations.rs` ends at global schema-v5, but
`db/extensions/writer-lease/v2.sql` constrains its extension identity and
runtime bind/assert profile to global schema 3 or 5. Adding the required
append-only global migration for `FOREMAN_SNAPSHOT_RECORDED` therefore needs a
new Writer Lease successor bridge, Store catalog/ACL profile, and combined
migration ordering/revalidation. Those are not TASK-079-owned paths and cannot
be implied by the existing v1 assertion function.

The TASK-050 worktree currently has uncommitted Task Ledger, Store, and module
constitution edits, so modifying the same contracts here would overwrite an
active owner. No code, migration, or live PostgreSQL test was run. The safe
next slice must own the Writer Lease successor and combined schema profile;
until then TASK-079 remains blocked rather than substituting a diagnostic,
dashboard, cache, or independent table.
