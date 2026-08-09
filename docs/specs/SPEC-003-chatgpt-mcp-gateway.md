---
spec_id: SPEC-003
status: ready
version: 1
modules:
  - module_id: latticed
    constitution_version: 1.1
---

# ChatGPT MCP Gateway

## Problem

ChatGPT cannot call the private newline-delimited `latticed` stdio process
directly. The current Rust MCP implementation also exposes five caller-supplied
binding fields even though the approved `latticed` contract requires two
closed, zero-argument tools with a composition-owned immutable binding.

## Intended Behavior

The official OpenAI Secure MCP Tunnel launches the private `latticed` stdio
server. ChatGPT discovers exactly `lattice_delivery_run` and
`lattice_delivery_status`; each tool accepts only an omitted argument object or
an empty object. The MCP adapter injects one server-owned immutable task binding
into the existing typed service boundary before dispatch.

A local operator entrypoint may create, inspect, or run a named
`sample_mcp_stdio_local` tunnel profile. It accepts only explicit executable,
profile, tunnel, and `latticed` paths; it never accepts a generic MCP command or
credential value. Runtime authentication comes only from the inherited
`CONTROL_PLANE_API_KEY` environment variable.
The launcher gives the tunnel-client child a closed environment: only the
explicit runtime key, required `LATTICE_*` process configuration, and a bounded
set of Windows process variables survive. Ambient tunnel/profile/MCP/control-
plane/log/health/UI/Harpoon/proxy/certificate/cloudflared overrides, including
`OPENAI_ADMIN_KEY` and the `OPENAI_API_KEY` fallback, cannot reach the child.
The operator process environment is restored after the child exits.

## User Stories Or System Scenarios

1. ChatGPT lists the two bounded LATTICE tools through a private tunnel.
2. An empty tool call reaches the existing typed delivery/status service with
   the exact server-owned binding.
3. A caller-supplied task, shell, SQL, filesystem, Git, provider, credential, or
   other field is rejected before service dispatch.
4. An operator can prepare and diagnose a tunnel profile without embedding a
   key in arguments or configuration.

## Goals

- Restore the approved closed zero-argument public MCP schema.
- Preserve the internal typed binding check and single composition root.
- Provide a deterministic Windows tunnel profile entrypoint.
- Keep all production truth and workflow authority inside LATTICE.

## Non-Goals

- A second orchestrator, truth source, writer, or HTTP MCP server.
- Account, workspace, developer-mode, tunnel, credential, deployment, or
  public-listener creation.
- Production `Hermes -> Memory -> Status` completion before TASK-037 passes.
- Actor-level rate limiting or a claim that ChatGPT end-to-end access is live.

## Constraints

- Preserve One Gateway, One Truth, One Writer.
- Preserve exactly two tool names and the existing bounded result/error shape.
- The tunnel is transport only and must launch `latticed` over stdio.
- No secret may appear in command arguments, checked-in configuration, MCP
  schemas, normal responses, or ordinary evidence.

## Module Impact

`latticed` 1.1: internal implementation correction only. The public schema is
restored to its approved zero-argument contract; no owned data, dependency, or
constitution amendment changes.

## Data, Privacy, And Security

The adapter owns no durable data. PostgreSQL remains authoritative. The public
schema cannot carry identity, task binding, paths, commands, SQL, credentials,
or provider settings. `CONTROL_PLANE_API_KEY` is inherited by the official
tunnel client and is never accepted as a script parameter.

The direct stdio binding has no HTTP MCP session ID and cannot receive a
connector bearer header. This checkpoint therefore binds authority only at the
server-owned tunnel/profile and fixed LATTICE subject boundary; it does not
claim per-human ChatGPT actor/session authorization or audit correlation.

## Compatibility And Migration

The current five-field public input is rejected after this correction. That
surface contradicted the active constitution and duplicated a fixed binding
already owned by composition. Internal services retain their typed
`DeliveryToolArguments` validation. Existing local MCP verifier requests must
send `{}`.

## Error Cases And Edge Cases

- Non-empty arguments fail with MCP `-32602` before dispatch.
- Unknown tools and extra call fields fail closed.
- The existing per-process invocation and frame-size bounds remain enforced.
- Tunnel initialization rejects malformed tunnel IDs, unsafe executable paths,
  unknown modes, and implicit profile locations.
- Tunnel `run` refuses to start without `CONTROL_PLANE_API_KEY`.
- `doctor` is local preflight only and cannot be reported as live readiness.

## Acceptance Criteria

- [x] Exact tool discovery reports two closed empty-object schemas.
- [x] Omitted and empty arguments dispatch with the server-owned binding.
- [x] Every caller-supplied property is rejected before dispatch.
- [x] Direct stdio MCP tests and the real-binary composition contract pass.
- [x] TASK-037 verifier uses empty argument objects without weakening its gates.
- [x] The tunnel entrypoint emits exact `init`, `doctor`, and `run` commands and
      never accepts or prints a credential.
- [x] Focused runtime tests, formatting, repository checks, and independent
      reviews pass. Scoped strict Clippy passes with the exact unchanged
      baseline exception; baseline-wide lints are reproduced and recorded.

## Verification Plan

| Criterion | Verification method | Expected evidence |
|---|---|---|
| Public schema and binding | Rust MCP contract tests | RED then passing exact-schema/dispatch tests |
| Real binary compatibility | Runtime composition tests | `latticed` discovery and calls pass |
| Verifier compatibility | PowerShell parser/static assertions | empty arguments and no duplicate binding |
| Tunnel entrypoint | isolated fake executable harness | exact argument arrays and fail-closed cases |
| Repository regression | format, scoped strict Clippy, runtime tests, Node checks | zero exit status plus exact baseline-wide lint record |

## Human Decisions

The user authorized local TASK-038 Phase 1-5 implementation on 2026-08-09.
Account/workspace permissions, tunnel creation, runtime credentials, public
exposure, production E2E, push, merge, and deployment remain separate actions.

## Open Questions

None for this local implementation slice. Broader Issue #4 still requires an
explicit fixed-profile-actor versus authenticated-loopback-HTTP decision before
per-human identity work.
