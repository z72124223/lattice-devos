---
protocol_id: LATTICE_ENGINEERING_PROTOCOL
version: 1.0.2
status: active
canonical_path: docs/contracts/ENGINEERING_PROTOCOL_V1.md
---

# LATTICE Engineering Protocol V1

This version-controlled contract keeps only the mandatory engineering entry and
delivery guards.

## Mandatory Entry

Before editing, read `AGENTS.md`, this protocol, `PLANS.md`, `HANDOFF.md`, and
the active task and module contracts. Confirm scope and preserve unrelated work.

## Mandatory Delivery

Before claiming completion, reread this protocol, inspect the final diff, run
`npm.cmd run check` and focused verification, and report only current evidence.
If an ordinary reproducible check fails, repair it within the authorized scope and rerun the same failed check.

After the durable handoff is current, and again after any authorized push, run
`npm.cmd run status:refresh`. This updates the local engineering-status
projection for the user. A refresh failure or stale/unknown source must remain
visible and be reported; the projection never replaces ticket, Git, test, CI,
review, or LATTICE acceptance evidence.

## Knowledge Routing

Personal preferences, historical cases, and detailed decision logic belong in LATTICE, Hermes, and the knowledge graph.
Retrieve them when relevant instead of copying them into this contract.

## Authority Boundary

These guards do not change P0 MCP closure, One Gateway, One Truth, One Writer,
PostgreSQL authority, lease or fencing, credentials, rollback, machine
acceptance, push, merge, deployment, or release boundaries.
