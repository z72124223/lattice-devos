---
ticket_id: TASK-054
title: Latticed safe startup diagnostics
spec_id: SPEC-003
spec_version: 4
module_id: latticed
constitution_version: 1.4
status: completed
parallel_safe: false
depends_on:
  - TASK-050
allowed_paths:
  - docs/tickets/TASK-054-latticed-startup-diagnostics.md
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/src/mcp.rs
  - apps/lattice-runtime/tests/mcp.rs
  - apps/lattice-runtime/tests/composition.rs
  - target/task054-latticed-startup-diagnostics/**
likely_files:
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/src/mcp.rs
  - apps/lattice-runtime/tests/mcp.rs
  - apps/lattice-runtime/tests/composition.rs
  - target/task054-latticed-startup-diagnostics/**
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-054 — Latticed safe startup diagnostics

## Objective

Give canonical `latticed` a built-in, daily-use startup diagnostic stream that
lets an operator distinguish configuration validation, service assembly, entry
into the MCP stdio loop, waiting for initialization/input, recognized
initialize/initialized/tools-list milestones, EOF, and a fixed startup failure
classification. It addresses the current TASK-051 pre-MCP observation gap
without using an external debugger for ordinary startup diagnosis.

## Scope

- Emit a newline-delimited, fixed-vocabulary record to stderr only.
- Each record contains a schema version, current stage, last completed stage,
  waiting reason, configuration health, dependency health, and fixed failure
  classification.
- Configuration and dependency health are classifications only. They never
  contain configuration/environment values, endpoint values, paths, DSNs,
  credentials, request bytes, child output, raw errors, or stack data.
- A normal startup records configuration validation, service assembly, stdio
  loop entry, and input waiting. A recognized MCP lifecycle input records only
  its fixed milestone. EOF is recorded before clean server return.
- A startup/configuration, assembly, or transport failure records the last
  completed stage and the existing fixed `LatticedErrorKind` code.

## Compatibility and non-goals

- MCP JSON-RPC stdout stays byte-for-byte unchanged; the exact four tool names,
  schemas, and responses stay closed and unchanged.
- The diagnostic stream is non-authoritative operational output, not an MCP
  result, Task Ledger event, database record, public status field, or a new
  tool.
- Do not add a network probe, database connection/mutation/repair, configuration
  mutation, automatic repair, stack capture, arbitrary process control,
  credential-reading API, or diagnostic MCP input.
- Rare OS/runtime faults below these fixed stages still require an external
  debugger. They are intentionally not converted into a product capability.

## Acceptance criteria

1. Focused tests prove the lifecycle observer records the fixed
   initialize/initialized/tools-list/EOF milestones while the resulting stdout
   remains valid, unchanged MCP JSON-RPC only.
2. Tests prove diagnostics use fixed values and a fixed failure code rather
   than caller or configuration text.
3. The full available project check and affected Rust tests pass, or any
   pre-existing unrelated failure is recorded separately.
4. A direct local startup probe can preserve stderr as the normal diagnostic
   artifact without sending an MCP semantic request.

## Completion evidence

- `cargo test -p lattice-runtime --all-targets` passed on 2026-08-13.
- `cargo fmt --all -- --check` and `npm.cmd run check` passed.
- The focused stream test proves stdout stays JSON-RPC-only while lifecycle
  milestones are observed separately; canonical legacy and stateless binary
  tests prove the four-tool surface remains unchanged while accepting the
  fixed stderr records.
- The current-machine EOF-only probe is preserved at
  `target/task054-latticed-startup-diagnostics/startup-eof-final-20260813T143600-714f3b9`.
  It sent neither an MCP protocol request nor a semantic request, left no
  run-owned candidate process, and recorded `CONFIGURATION_VALIDATION_STARTED`
  with `last_completed_stage=NONE`.
- Strict workspace Clippy remains separately blocked by pre-existing
  `lattice-hermes-adapter` diagnostics; TASK-054 adds no dependency or lint
  suppression.
