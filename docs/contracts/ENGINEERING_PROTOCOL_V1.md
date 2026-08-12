---
protocol_id: LATTICE_ENGINEERING_PROTOCOL
version: 1.0.0
status: active
canonical_path: docs/contracts/ENGINEERING_PROTOCOL_V1.md
---

# LATTICE Engineering Protocol V1

This file is the version-controlled engineering entry contract for work in this
repository. It supplements higher-priority platform and user instructions; it
does not grant authority or weaken any safety boundary.

## Start Gate

Before editing, read `AGENTS.md`, this protocol, `PLANS.md`, `HANDOFF.md`, the
active task material, and every applicable module constitution. Confirm the
current checkout, scope, and unrelated working-tree changes before modifying
files.

## Execution Rule

Keep changes within the authorized task and owned paths. Preserve unrelated
work. Use the repository's existing contracts, policy boundaries, and bounded
interfaces instead of adding a second truth, writer, authority source, or
unrequested dependency.

## Repairable Failure Rule

A reproducible ordinary compile, test, formatting, configuration, or
development-PostgreSQL failure is repairable evidence, not a reason to close
the task. Diagnose it, repair it within the authorized scope, and rerun the
same failed check. Stop only at a new authority decision, protected or
irreversible boundary, unknown unrelated change that cannot be preserved, or a
failure that remains genuinely blocked after bounded repair attempts.

## Completion Gate

Before claiming completion, reread this protocol, inspect the final diff,
preserve unrelated changes, and run focused verification proportional to the
changed behavior. Report exact commands and outcomes, distinguish local checks
from live machine acceptance, and keep unrun or blocked gates explicit.

## Preserved Boundaries

This protocol does not change One Gateway, One Truth, One Writer, PostgreSQL
authority, Writer Lease or fencing, credentials, rollback, machine acceptance,
MCP tool closure, push, merge, deployment, or release boundaries.

## Current MCP Availability

The active TASK-038 P0 contract fixes canonical `latticed` discovery at exactly
four bounded tools. Until that contract is versioned forward, this protocol is
read through its canonical repository path and validated by
`scripts/check-project.mjs`; no additional public MCP tool or resource is
advertised by this slice.
