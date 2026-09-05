---
spec_id: SPEC-012
title: Desktop Control connection to the existing LATTICE Runtime
version: 1.0
status: approved
approved_by: user
approved_at_local: 2026-09-03
modules:
  - module_id: lattice-control-desktop
    constitution_version: unconstituted
  - module_id: lattice-control
    constitution_version: unconstituted
  - module_id: latticed
    constitution_version: 3.9
---

# SPEC-012 - Desktop Control connection to the existing LATTICE Runtime

## Problem

The installed desktop Control currently reports formal PostgreSQL as
`NOT_IMPLEMENTED` even when the existing `latticed` Runtime can verify the
PostgreSQL-backed foreman. Control therefore cannot distinguish a healthy
formal Runtime from an unavailable or incompatible one.

## Intended Behavior

The desktop-owned Control backend reads the already configured local `latticed`
command and environment in memory without exposing those values to the browser.
Control performs a bounded, read-only MCP handshake and calls only
`lattice_runtime_status`. The returned secret-free projection drives the formal
PostgreSQL capability shown in the UI.

The compatibility loader may derive the legacy
`LATTICE_DELIVERY_LAUNCHER` and `LATTICE_DELIVERY_SCHEMA_DIR` process values
from already configured equivalent values, in memory only. It must not modify
the global Codex configuration or create another database.

## User Scenarios

- With the existing compatible Runtime and PostgreSQL available, the user sees
  formal PostgreSQL as healthy.
- During a cold Runtime probe, the UI remains responsive and temporarily shows
  `NO_DATA`; a later poll promotes it to the verified health result.
- With missing configuration, a stopped database, a malformed response, an
  unexpected tool catalog, or an incompatible Runtime, the user sees a bounded
  degraded state while the desktop app and Control SQLite remain usable.
- Browser clients never receive Runtime credentials, command paths, raw stderr,
  or hidden model output.

## Goals

- Reuse the existing `latticed` / PostgreSQL authority.
- Make Runtime health observable from the installed desktop app.
- Fail closed on identity, protocol, catalog, schema, or response mismatch.
- Keep the probe read-only and bounded in time and bytes.

## Non-Goals

- No second PostgreSQL cluster, task ledger, scheduler, agent loop, or truth.
- No direct SQL from Control.
- No change to the current `+ New work` conversation semantics.
- No automatic task submission, merge, deployment, or release authority.
- No mutation of `%USERPROFILE%\.codex\config.toml`.

## Constraints And Module Impact

- `lattice-control-desktop`: accepts the existing bounded capability states for
  formal PostgreSQL; it receives no credentials or command paths.
- `lattice-control`: internal read-only MCP client and runtime-surface change;
  no owned durable data or public write contract is added.
- `latticed` 3.9: no source or public-contract change; the exact existing
  `lattice_runtime_status` tool is consumed.

No constitution amendment is required because `latticed` remains the sole
composition root and PostgreSQL remains the sole durable authority.

## Data, Privacy, And Security

- Configuration values are never logged, persisted by Control, or returned by
  HTTP.
- Only the exact configured local executable is started, with no shell.
- The MCP client accepts the exact protocol identity and seven-tool catalog.
- Only `lattice_runtime_status` with `{}` arguments is callable by this path.
- Child lifetime and output are bounded; timeout stops only the owned child.

## Compatibility And Migration

Existing Control SQLite data and schema remain byte-compatible. Missing or
older Codex configuration produces `STOPPED`, not a crash. The two legacy
environment aliases are derived only when absent and their sources are exact.

## Error Cases And Edge Cases

- Missing/non-absolute executable: `STOPPED`.
- Cold probe still running: `NO_DATA` without blocking the UI.
- Process start/timeout/no response: `UNREACHABLE`.
- Wrong MCP protocol/server/catalog, malformed or oversized result:
  `INCOMPATIBLE`.
- Runtime tool error or PostgreSQL failure: `UNREACHABLE` with no secret detail.
- Verified runtime response without retained delivery data still means the
  PostgreSQL capability is `HEALTHY`; delivery receipt state is separate.

## Acceptance Criteria

- [ ] A real configured Runtime returns a verified, secret-free status and the
  desktop `/api/runtime` surface reports formal PostgreSQL `HEALTHY`.
- [ ] The first `/api/runtime` response stays responsive while a cold Runtime
  probe completes in the background.
- [ ] Missing, hostile, malformed, timed-out, and incompatible fixtures map to
  bounded degraded states without starting another database or exiting Control.
- [ ] Browser-visible responses contain no configured secret or raw stderr.
- [ ] Existing Control tests, desktop lifecycle/policy tests, build, publish,
  candidate checks, and installed user-mode smoke remain green.
- [ ] Git contains one local implementation commit while the pre-existing dirty
  `HANDOFF.md` remains untouched.

## Verification Plan

| Criterion | Verification method | Expected evidence |
|---|---|---|
| Exact MCP compatibility | Node unit tests with protocol fixtures | exact catalog accepted; substitutions rejected |
| Safe degradation | timeout/start/error/malformed unit tests | status only; Control remains available |
| Live PostgreSQL connection | real zero-argument status call | foreman replay `VERIFIED`; UI capability `HEALTHY` |
| Packaging | desktop publish/candidate/installer tests | required source included; no credential file |
| User experience | installed desktop mouse/keyboard observation | app opens, capability is visible, conversation still works |

## Human Decisions

The user approved this local connection layer on 2026-09-03. Push, merge,
deployment, and release remain separate decisions.

## Open Questions

None for this bounded connection layer.
