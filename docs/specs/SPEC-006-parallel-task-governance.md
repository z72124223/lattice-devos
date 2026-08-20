---
spec_id: SPEC-006
title: Parallel TASK governance check
version: 2
status: approved
approved_by: delegated_user_instruction
approved_at_local: 2026-08-21
---

# SPEC-006 — Parallel TASK governance check

## Problem

The project check treated the single `PLANS.md` CURRENT marker as the active
worktree identity. That rejects an otherwise authorized parallel TASK branch
and pressures workers to rewrite the shared planning index.

## Required behavior

1. `PLANS.md` retains exactly one CURRENT TASK marker and continues to validate
   its unique ticket, module, branch guide, and delivery metadata.
2. On `feature/task-nnn-*`, when that branch differs from the PLANS CURRENT
   TASK, the checker treats the captured ticket matching `TASK-nnn` as the
   parallel delivery identity.
3. That parallel ticket must be unique, terminal (`complete`, `completed`, or
   `verified`), name the exact current branch, resolve to an existing module,
   and declare valid credential-free delivery metadata and an allowed delivery
   push policy. Its exactly-once, non-empty Traditional-Chinese
   `display_name_zh_tw` and `display_purpose_zh_tw` fields are the preferred
   human-readable evidence; the shared branch guide is legacy fallback only.
4. Missing tickets, duplicate identity, branch mismatch, non-terminal status,
   malformed or unauthorized delivery metadata, and the configured default
   branch fail closed. No rule permits a parallel worker to create another
   CURRENT marker or edit PLANS.
5. Detached read-only verification keeps its prior behavior; this change adds
   no exporter, dashboard-parser, runtime, database, MCP, Hermes, or Git
   mutation capability.
6. A governed worktree must not resolve through a reparse point or be nested
   beneath another Git worktree. The checker has no approved-workspace-root
   registry input, so enforcement of a specific workspace-root allowlist and
   direct-child morphology remains a worktree-manager follow-up rather than an
   inferred local path policy.

## Acceptance criteria

- Focused tests prove a legal non-CURRENT parallel TASK passes while PLANS has
  one unchanged CURRENT marker.
- Focused tests prove every listed failure case remains denied.
- Focused tests prove ticket-local metadata for TASK-081/082/083-shaped
  branches passes without shared-file ownership, while legacy guide fallback
  remains valid and missing, duplicate, blank, or non-Chinese local values fail.
- Focused tests characterize and deny the TASK-083 nested-worktree shape.
- `npm.cmd run check`, `npm.cmd run verify`, and `git diff --check` pass.

## Amendment history

- v2 moves the parallel-safe branch presentation evidence from shared JSON to
  closed ticket-local fields, while retaining the shared guide as legacy
  fallback for existing tickets.
