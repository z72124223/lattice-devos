---
spec_id: SPEC-003
status: ready
version: 2
modules:
  - module_id: latticed
    constitution_version: 1.1
---

# ChatGPT MCP Gateway

## Problem

ChatGPT cannot call the private newline-delimited `latticed` stdio process
directly. The approved `latticed` contract requires two closed, zero-argument
tools with a composition-owned immutable binding. ChatGPT's current refresh
path also uses stateless MCP `2026-07-28` discovery, while the original server
implemented only the stateful `2025-11-25` lifecycle.

## Intended Behavior

The official OpenAI Secure MCP Tunnel launches the private `latticed` stdio
server. ChatGPT discovers exactly `lattice_delivery_run` and
`lattice_delivery_status`; each tool accepts only an omitted argument object or
an empty object. The MCP adapter injects one server-owned immutable task binding
into the existing typed service boundary before dispatch.

The server supports two wire generations without changing tool authority:
legacy clients retain `initialize -> notifications/initialized`, while
`2026-07-28` clients use `server/discover` and per-request protocol metadata
without initialization. Modern discovery/list results use private, immediately
stale cache hints; every modern success result declares `resultType`. Reserved
modern metadata cannot silently downgrade into the legacy lifecycle.

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
5. A restricted Tunnels Read+Use runtime key can start the existing private
   profile, reach `/readyz`, and refresh the existing ChatGPT app without
   persisting or exposing the key.

## Goals

- Restore the approved closed zero-argument public MCP schema.
- Preserve the internal typed binding check and single composition root.
- Provide a deterministic Windows tunnel profile entrypoint.
- Preserve legacy MCP clients while supporting ChatGPT's stateless
  `2026-07-28` discovery and tool-call path.
- Prove live Phase 1 readiness and ChatGPT tool discovery through the existing
  private app and tunnel profile.
- Keep all production truth and workflow authority inside LATTICE.

## Non-Goals

- A second orchestrator, truth source, writer, or HTTP MCP server.
- A successful production delivery run or status replay through ChatGPT.
- Public-listener creation, deployment, release, or broad workspace/account
  administration.
- Production `Hermes -> Memory -> Status` completion before TASK-037 passes.
- Per-human actor/session authorization or actor-level rate limiting.

## Constraints

- Preserve One Gateway, One Truth, One Writer.
- Preserve exactly two tool names and the existing bounded result/error shape.
- The tunnel is transport only and must launch `latticed` over stdio.
- No secret may appear in command arguments, checked-in configuration, MCP
  schemas, normal responses, or ordinary evidence.

## Module Impact

`latticed` 1.1: an internal schema correction plus additive wire-protocol
compatibility. The approved zero-argument tool contract, owned data,
dependencies, and authority remain unchanged, so no constitution amendment is
required.

## Data, Privacy, And Security

The adapter owns no durable data. PostgreSQL remains authoritative. The public
schema cannot carry identity, task binding, paths, commands, SQL, credentials,
or provider settings. `CONTROL_PLANE_API_KEY` is inherited by the official
tunnel client and is never accepted as a script parameter.
The live operator flow uses only a restricted Tunnels Read+Use key, keeps it in
process memory, clears the clipboard immediately after launch, and revokes any
superseded key. Key text is never inspected, logged, or written to the profile.

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

The stateful `2025-11-25` lifecycle remains compatible. Stateless
`2026-07-28` requests require the reserved protocol-version and
client-capabilities metadata; `clientInfo` remains optional and informational.

## Error Cases And Edge Cases

- Non-empty arguments fail with MCP `-32602` before dispatch.
- Unknown tools and extra call fields fail closed.
- Missing or malformed stateless request metadata fails with `-32602` before
  dispatch; a well-formed unsupported version fails with bounded `-32022`.
- `server/discover` does not mutate the legacy lifecycle, and removed modern
  `initialize`/`ping` methods fail with `-32601`.
- The legacy per-session invocation bound and all frame-size bounds remain
  enforced. Stateless calls do not consume a process-lifetime pseudo-session;
  per-actor rate limiting remains outside this Phase 1 transport slice.
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
- [x] Legacy `2025-11-25` and stateless `2026-07-28` real-binary paths both
      preserve the same two-tool, server-owned binding contract.
- [x] A restricted live runtime key reaches tunnel readiness and the existing
      ChatGPT app refresh discovers exactly the two LATTICE tools.

## Verification Plan

| Criterion | Verification method | Expected evidence |
|---|---|---|
| Public schema and binding | Rust MCP contract tests | RED then passing exact-schema/dispatch tests |
| Real binary compatibility | Runtime composition tests | `latticed` discovery and calls pass |
| Verifier compatibility | PowerShell parser/static assertions | empty arguments and no duplicate binding |
| Tunnel entrypoint | isolated fake executable harness | exact argument arrays and fail-closed cases |
| Dual MCP generations | Rust unit and real-binary tests | legacy lifecycle plus stateless discovery/list/call pass |
| Live ChatGPT discovery | `/readyz`, tunnel admin log/metrics, app refresh | ready 200 and exact two-tool discovery without negotiation errors |
| Repository regression | format, scoped strict Clippy, runtime tests, Node checks | zero exit status plus exact baseline-wide lint record |

## Human Decisions

The user authorized local TASK-038 implementation and the bounded live
credential/tunnel/app-refresh acceptance flow on 2026-08-09. Public exposure,
production E2E, push, primary-branch merge, deployment, and release remain
separate actions.

## Open Questions

None for this local implementation slice. Broader Issue #4 still requires an
explicit fixed-profile-actor versus authenticated-loopback-HTTP decision before
per-human identity work.
