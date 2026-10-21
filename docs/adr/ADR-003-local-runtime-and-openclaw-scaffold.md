# ADR-003: Dependency-Light Node Core and Inert OpenClaw Scaffold

- Status: superseded for V2 core; retained as V1/plugin evidence
- Date: 2026-07-29

> ADR-004 proposes the V2 Rust/polyglot boundary. The Node-core decision and
> inert-scaffold end state are not active for new work. The verified OpenClaw
> package-contract evidence remains useful until a live V2 preflight replaces
> it.

## Context

The workspace has Node.js 24.16.0 and no OpenClaw installation. The external
plugin contract changes over time, while Phase 1 explicitly forbids deployment,
authentication, and real model use.

## Decision

- Implement the control core as Node.js ESM using the standard library.
- Keep third-party runtime dependencies at zero for the offline core.
- Define injected ports for clock, approval verification, Git execution,
  runtime execution, review, and ledger storage.
- Ship an inert native OpenClaw plugin scaffold with:
  - `openclaw.plugin.json`;
  - `package.json` entry metadata;
  - a TypeScript ESM source entry and built JavaScript entry;
  - authenticated `api.registerCommand(...)` registration for `/lattice`;
  - a fail-closed response stating that the live orchestrator bridge is not
    enabled in Phase 1.
- Perform only static local consistency checks in Phase 1.
- Do not claim or pin an OpenClaw compatibility floor until the exact target
  host/plugin API version is installed and validated in Phase 3.

## Official Contract Evidence

Verified on 2026-07-29:

- [Building plugins](https://docs.openclaw.ai/plugins/building-plugins)
- [Plugin manifest](https://docs.openclaw.ai/plugins/manifest)
- [Plugin entry points](https://docs.openclaw.ai/plugins/sdk-entrypoints)
- [Plugin SDK overview](https://docs.openclaw.ai/plugins/sdk-overview)
- [Plugin hooks and command example](https://docs.openclaw.ai/plugins/hooks)
- [OpenAI/Codex provider route](https://docs.openclaw.ai/providers/openai)

The OpenClaw source snapshot independently inspected for these contracts was
commit
[`5578d01777ef354bd459bc6ee0c04716ab6b0eaa`](https://github.com/openclaw/openclaw/tree/5578d01777ef354bd459bc6ee0c04716ab6b0eaa).
For V1, LATTICE planned to use the official `@openclaw/codex` harness rather
than implement a second Codex app-server harness. Proposed ADR-006 supersedes
that ownership choice for V2 by recommending that the Rust core own the single
writable Codex process while OpenClaw remains the gateway. Neither live path
has been verified in this repository.

## Consequences

- Core tests run without network access or package installation.
- Static checks cannot prove that a particular Hostinger/OpenClaw image loads
  the plugin.
- Phase 3 must pin the target compatibility range, run
  `openclaw plugins inspect ... --runtime --json`, and run the
  applicable plugin validation command in the actual target environment before
  activation.
