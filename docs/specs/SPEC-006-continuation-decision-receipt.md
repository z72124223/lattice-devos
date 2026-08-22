---
spec_id: SPEC-006
title: Bounded continuation decision receipt
version: 1
status: approved
module_id: engineering-status-dashboard
constitution_version: 1.3
---

# Bounded continuation decision receipt

## Problem

The dashboard already displays a free-text next step and dispatch eligibility,
but a new Codex window cannot safely distinguish continuation from dependency,
stale evidence, reconciliation, or duplicate work.

## Behavior

Each worktree item receives a closed `continuation` projection derived only from
the current ticket outcome, source completeness, Git identity/synchronization,
and duplicate identity counts. It has exactly one state:

- `CONTINUE`
- `WAITING_DEPENDENCY`
- `RECONCILIATION_REQUIRED`
- `STOP_DUPLICATE`

The projection includes a plain-language `nextAction` and `reasonZh`, but never
creates work, changes Git, contacts GitHub, invokes a model, or authorizes a
protected action. Missing, stale, conflicting, or incomplete evidence fails
closed to `RECONCILIATION_REQUIRED`.

## Acceptance

- Complete, unique, synchronized, non-duplicate current work yields
  `CONTINUE`.
- Explicit dependency or user-action outcomes yield `WAITING_DEPENDENCY`.
- Stale, unknown, partial, failed, or incomplete source evidence yields
  `RECONCILIATION_REQUIRED`.
- Duplicate canonical TASK/ISSUE identities yield `STOP_DUPLICATE` for every
  duplicate item.
- Snapshot schema remains `lattice.engineering-status/2.0`; the new field is
  additive and validated before publication.
