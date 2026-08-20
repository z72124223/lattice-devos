# TASK-079 architecture review — 2026-08-21

Trigger: new snapshot module and proposed durable control-plane schema/event.

The implemented `foreman-state` crate remains pure and has no I/O, dashboard,
Git, database, process, scheduler, MCP, or Writer Lease dependency. It is
compatible with TASK-048 as a read-only identity input and TASK-049 as a
read-only archive consumer. TASK-078 exporter files are unchanged.

Integration is **BLOCKED**. The existing Task Ledger only accepts a `TASK`
stream and closed event kinds; `Diagnostic` is non-authoritative and cannot be
repurposed for the snapshot. The next slice must version and test a fixed
control-stream/event, a typed Port, Postgres Store physical row/function and
same-transaction Writer Lease/fencing assertion. A standalone table, dashboard
JSON, chat record, or cache would violate One Gateway. One Truth. One Writer.
