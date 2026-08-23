---
ticket_id: TASK-091
title: Native Codex process fault fixture
spec_id: SPEC-007
spec_version: 1
module_id: lattice-codex-adapter
constitution_version: 1.2
status: in_progress
parallel_safe: false
depends_on:
  - TASK-090
evidence_subjects:
  - TASK-051
execution_authorized_by: direct_user_goal_mode_authorization
execution_authorized_at_local: 2026-08-23
allowed_paths:
  - docs/tickets/TASK-091-native-codex-process-fault-fixture.md
  - crates/lattice-codex-adapter/Cargo.toml
  - crates/lattice-codex-adapter/src/**
  - crates/lattice-codex-adapter/tests/process.rs
branch: product/lattice-control-mvp
---

# TASK-091 — Native Codex process fault fixture

## Objective

Replace the Windows-only PowerShell fake app-server used by the Codex process
tests with a Rust-native, test-only helper.  The replacement must exercise the
same `run_codex_app_server` public boundary and prove all of these conditions:

1. an owned app-server child that exceeds its deadline returns `Timeout`;
2. the owned Windows Job Object reaps its writable descendant before the caller
   returns;
3. a completed child whose launcher exits cannot leave a descendant effect;
4. duplicate, foreign, and late callback frames remain rejected by the existing
   session verifier;
5. the fixture needs no PowerShell, profile, execution-policy change, account,
   credential, network, or external Codex installation.

## Current blocker evidence

On 2026-08-23, focused test
`timeout_immediately_terminates_and_reaps_the_owned_tree` returned
`SpawnFailed` instead of `Timeout`.  Inspection shows the fixture launches a
PowerShell `.ps1` server.  This conflicts with the user-authorized Rust-only
live-fault path, so it is not evidence of a timeout or cleanup guarantee.

## Boundaries

- No production app-server behavior, public protocol, timeout meaning, or Job
  Object implementation change unless a focused test proves a product defect.
- No relaxed assertion that accepts `SpawnFailed` as a timeout result.
- The helper accepts only fixture-selected modes; no arbitrary command, path,
  script, environment, or child executable surface is introduced.

## Acceptance

1. Existing success, malformed, EOF, wrong-home, timeout, and orphan tests
   pass using the native helper.
2. The focused timeout test returns exactly `Timeout`, records a descendant PID,
   and proves no descendant effect after return.
3. The existing duplicate/late callback test continues to pass unchanged.
4. Focused adapter tests, formatting, diff check, and an independent code
   review have no unresolved P0/P1 issue.

## Verification commands

```text
cargo fmt --check
cargo test -p lattice-codex-adapter timeout_immediately_terminates_and_reaps_the_owned_tree
cargo test -p lattice-codex-adapter job_object_kills_a_descendant_after_the_launcher_parent_exits
cargo test -p lattice-codex-adapter rejects_foreign_duplicate_and_late_completed_items
git diff --check
```
