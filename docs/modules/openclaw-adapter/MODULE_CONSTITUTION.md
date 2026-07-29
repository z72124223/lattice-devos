---
module_id: openclaw-adapter
name: OpenClaw Adapter
version: 1.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-07-29
---

## Mission

Own the native OpenClaw plugin package boundary and authenticated `/lattice`
command without duplicating Orchestrator, Codex harness, policy, or task state.

## Non-Goals

- Reimplement the official Codex app-server harness.
- Directly mutate Task Ledger, Git, project files, Runtime, or credentials.
- Claim Hostinger/Telegram/live compatibility from static files.
- Execute a task in the inert Phase 1 scaffold.

## Owned Data

- Plugin manifest, package entry metadata, source/built entry, and bundled
  `lattice-workflow` skill metadata.
- `/lattice` command parsing and Phase 1 fail-closed response.

The Orchestrator owns task behavior. Official OpenClaw/Codex plugins own their
runtime protocol. Live config and credentials remain operator-owned.

## Public Contracts

- Native plugin ID `lattice-devos` agrees across manifest and entry.
- Register `/lattice` with `acceptsArgs: true` and `requireAuth: true`.
- Keep the bundled skill non-user-invocable so `/lattice` has one owner.
- Phase 1 handler reports that no live bridge/action occurred.
- Future live adapter may call only the Orchestrator public command contract.

## Invariants

1. No native/Codex dual-format manifest exists in the same plugin directory.
2. Source and runtime entry arrays are paired and stay inside the package.
3. `/lattice` is authenticated and has exactly one registration.
4. Phase 1 registration performs no network, model, credential, Git, or file
   side effect.
5. A static check never reports live runtime acceptance.

## Allowed Dependencies

- Focused official `openclaw/plugin-sdk/plugin-entry` import in plugin entry.
- Future Orchestrator client/public contract after a separate approved ticket.

## Forbidden Dependencies

- Internal OpenClaw paths, third-party Codex harnesses, direct app-server
  protocol copies, Task Ledger files, Git adapters, credential stores, or model
  APIs.

## Failure, Compatibility, And Migration

No OpenClaw compatibility floor is claimed until the target host version is
known and tested. Missing runtime entry, ID mismatch, or unauthenticated command
fails validation. Phase 3 must pin the tested host/plugin API versions and run
live inspect/validate commands.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Static manifest/package test | `node --test test/openclaw-scaffold.test.js` | Engineering | yes |
| Command registration fake | authenticated/inert handler assertions | Engineering | yes |
| Official-contract comparison | pinned source note | Architecture review | yes |
| Live OpenClaw validation | Phase 3 target runtime | User/preflight | no for Phase 1; yes before deploy |

## Change Policy

Plugin ID, command ownership, SDK imports, capabilities, config, live bridge, or
compatibility floor changes require a versioned amendment, current official
source verification, security/architecture review, and responsible-human
approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | ADR-003 | Initial inert native plugin boundary | Current user task |

