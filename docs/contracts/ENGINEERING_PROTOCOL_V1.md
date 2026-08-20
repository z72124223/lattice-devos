---
protocol_id: LATTICE_ENGINEERING_PROTOCOL
version: 1.1.0
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

After the durable handoff is current and the logical commit is clean, do not use
an ordinary manual push. Run `npm.cmd run delivery:finish`. The current TASK
ticket must explicitly declare its named remote, canonical
`delivery_repository` identity, `delivery_push` policy, and `delivery_archive`
policy. The finisher may perform only the authorized non-force
current-feature-branch push, verifies exact remote and upstream equality, and
then runs the engineering-status refresh.

`npm.cmd run status:refresh` remains the manual diagnostic and launcher
fallback. A refresh failure or stale/unknown source must remain visible and be
reported; the projection never replaces ticket, Git, test, CI, review, or
LATTICE acceptance evidence.

Only the exact successful marker `LATTICE_DELIVERY_READY_TO_ARCHIVE=1` permits
Codex to use the Codex App native archive-task action as its final operation.
Missing markers, `keep_open`, interruption, or every failure keeps the task open
for diagnosis. Repository code never archives or edits Codex App task storage.

Every new branch must add a plain Traditional-Chinese name and purpose to
`tools/engineering-status-dashboard/branch-guide.zh-TW.json` and include that
path in the active ticket `allowed_paths`. Complete this human-readable entry
before the delivery refresh; an unmapped branch intentionally remains visible
but cannot be selected as a new-work starting point.

## Knowledge Routing

Personal preferences, historical cases, and detailed decision logic belong in LATTICE, Hermes, and the knowledge graph.
Retrieve them when relevant instead of copying them into this contract.

## Authority Boundary

These guards do not change P0 MCP closure, One Gateway, One Truth, One Writer,
PostgreSQL authority, lease or fencing, credentials, rollback, machine
acceptance, push, merge, deployment, or release boundaries.
