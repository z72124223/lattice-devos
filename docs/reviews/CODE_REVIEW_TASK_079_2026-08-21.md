# TASK-079 code review — 2026-08-21

Review target: current TASK-079 diff through the pure `lattice-foreman-state`
foundation. Reviewer independence: not proven; this is a separate read-only
self-review because no independent reviewer is available.

## Findings

### P1 — durable Ledger/Port/Postgres path is not implemented

`crates/lattice-foreman-state/src/lib.rs` provides only pure validation,
reconstruction, and watchdog classification. It does not append a Task Ledger
event, require/recheck Writer Lease fencing, expose a typed Port, or persist
through Postgres Store. SPEC-006 acceptance criterion 2 therefore remains
unmet; a fresh operating-system process cannot load durable foreman state.

Resolution: intentionally not papered over. TASK-079 remains `blocked` until a
versioned fixed control-stream/event plus Postgres transaction/migration slice
is implemented and tested.

### P1 — the required global migration is not safe under the current Writer Lease v2 profile

The durable-binding audit proved that the current global Store manifest ends at
schema-v5, while `db/extensions/writer-lease/v2.sql` accepts only global schema
3 or 5 in its extension identity and runtime bind/assert profile. A new global
schema-v6 migration would therefore invalidate the exact current Writer Lease
profile before a fenced append could be accepted. TASK-050 also has uncommitted
changes in the same Ledger/Store governance paths.

Resolution: no speculative `0007`, Store adapter, fake durable result, or
diagnostic/table bypass was added. A separately owned Writer Lease successor
bridge plus Store profile/migration implementation is required before this
ticket can write a valid durable RED test.

## No additional findings

The completed pure foundation has focused coverage for generation rollback,
identity collision, dependency-blocked retention, dashboard/Git drift, missed
heartbeats, and secret/transcript rejection. It accepts normal `task-` branch
and worktree references while rejecting exact secret-token prefixes and a
non-ASCII confusable input fail-closed. The added `lattice.foreman-epistemic/1.0`
references are bounded digest pointers, expose no hypothesis text, and cannot
modify lifecycle state; learning/promotion remains TASK-084 scope.
