# TASK-089 Architecture Review — 2026-08-21

## Trigger

The change adds a ticket-frontmatter contract consumed by the delivery finisher
and dashboard exporter.

## Result

**PASS — no architecture blocker.**

Before this change, reconciliation provenance was overloaded onto
`depends_on`, making an annotation look like a delivery edge. After this
change, `evidence_subjects` is a distinct captured-tree provenance relation.
The finisher remains the only bounded Git mutation owner; the dashboard remains
read-only and only projects the relation. No new writer, durable truth source,
runtime service, database, MCP capability, dependency, migration, or external
authority is introduced.

The contract is backward compatible: absent `evidence_subjects` means no
provenance relation. Once present, the field is fail-closed and cannot overlap
delivery dependencies or form a provenance cycle. No ADR amendment is needed.
