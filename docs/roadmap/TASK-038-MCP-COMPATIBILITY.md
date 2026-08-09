# TASK-038 Phase 1 — ChatGPT MCP Compatibility

Status on 2026-08-09: the local contract correction and bounded tunnel
entrypoint are implemented on `feature/task-038-chatgpt-mcp`. Live ChatGPT
connection remains unclaimed until a real tunnel ID, runtime key, workspace
permissions, successful control-plane readiness, and ChatGPT discovery/call
evidence exist.

## Decision

Use the OpenAI Secure MCP Tunnel as the thinnest supported bridge between
ChatGPT and the existing private `latticed` stdio server:

```text
ChatGPT developer-mode app
  -> OpenAI-hosted tunnel endpoint
  -> tunnel-client (outbound HTTPS only)
  -> latticed (private newline-delimited stdio MCP)
```

This is transport-only. `latticed` remains the sole normal LATTICE
composition root and PostgreSQL remains the durable truth. The tunnel and any
future ChatGPT adapter must not retain authoritative task state, issue writer
leases, or introduce orchestration logic.

## Current Compatibility Findings

- The OpenAI MCP guide supports remote servers over Streamable HTTP or HTTP/SSE;
  raw stdio is not a direct public remote-MCP transport.
- The Secure MCP Tunnel guide explicitly supports a private MCP server reachable
  over stdio or HTTP, and explicitly lists ChatGPT as a supported product.
  Therefore a tunnel profile may launch the existing `latticed` stdio command
  without modifying Rust Core task semantics or opening an inbound listener.
- `apps/lattice-runtime/src/mcp.rs` exposes only
  `lattice_delivery_run` and `lattice_delivery_status`, has closed tool
  dispatch, rejects unknown names and extra call fields, and bounds both frame
  size and invocation count. TASK-038 restored both input schemas to closed
  empty objects and moved the immutable binding back to composition-owned
  process configuration.
- No tunnel, developer-mode app, API key, authentication material, public
  endpoint, or listener was created during Phase 1. Those require separately
  authorized account/credential actions.

## Required Platform Preconditions

The future operator needs a target ChatGPT workspace with developer-mode
access, a Platform tunnel with the target workspace association, and the
applicable tunnel permissions. `tunnel-client` needs only outbound HTTPS to
OpenAI and local reachability to `latticed`; it must run inside the same trust
boundary as the private LATTICE process.

Use ChatGPT tool approval for any action-capable tool by default. The server
must continue to enforce LATTICE policy itself: client/tool approval is not
authority to obtain a writer lease or bypass policy.

## Contract Decision Implemented

`docs/modules/latticed/MODULE_CONSTITUTION.md` describes two zero-parameter
tools. The earlier `apps/lattice-runtime/src/mcp.rs` implementation instead
required a fixed five-field immutable binding from the caller. TASK-038 chose
the already-approved constitution contract: public calls accept only omitted
arguments or `{}`, while composition injects the fixed typed binding into the
existing service boundary.

This is an implementation correction, not a constitution amendment. Contract
tests reject the retired five fields and generic shell, SQL, filesystem, Git,
credential, provider, arbitrary-task, non-object, or other input before service
dispatch.

## Local Tunnel Evidence

- Official `tunnel-client` v0.0.11 was downloaded into ignored build output and
  matched the published Windows amd64 SHA-256
  `eb912c86c6ccde90cda805cb17009507176a656725cf86c36fabe1901a12e29b`.
- The local launcher generated an isolated `sample_mcp_stdio_local` profile for
  the real built `latticed.exe` without storing a key.
- `doctor --explain` failed closed because `CONTROL_PLANE_API_KEY` is absent.
  This proves local preflight behavior only; it is not live readiness.
- The launcher accepts no credential parameter and gives the child a closed
  environment containing only the explicit runtime key, required `LATTICE_*`
  process configuration, and bounded Windows process variables. Hostile
  tunnel/MCP/control/log/health/UI/profile/proxy overrides are removed and the
  parent environment is restored exactly.

## Identity Boundary Still Open

Official `tunnel-client` v0.0.11 documents that stdio bindings have no HTTP MCP
session ID. Connector OAuth headers apply to an HTTP MCP target, not to the
private stdio child. The direct-stdio design is sufficient for this fixed,
server-owned two-tool checkpoint, but it does not prove per-human ChatGPT
actor/session binding, LATTICE-side actor authorization, durable audit
correlation, or per-actor rate limiting.

The next identity slice must make an explicit architecture decision: either
approve one tunnel/profile as a single fixed gateway actor with appropriately
narrow policy, or introduce a thin authenticated loopback HTTP adapter. The
latter changes the transport/public-contract boundary and requires a versioned
design review before implementation. Neither choice may accept caller-declared
identity as authority.

## Non-Goals

- Publishing a public MCP endpoint or plugin.
- Creating or storing credentials, tunnels, workspace access, or developer-mode
  configuration.
- Changing Rust task semantics, tool names, durable state, policy, or writer
  ownership.
- Claiming production `Hermes -> Memory -> Status` completion before TASK-037
  passes.

## Sources

- OpenAI, [MCP and Connectors](https://developers.openai.com/api/docs/guides/tools-connectors-mcp): remote MCP transport, tool approval, and allow-list guidance.
- OpenAI, [Secure MCP Tunnel](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels): ChatGPT private-stdio tunnel support, workspace permissions, and outbound-only network model.
- OpenAI, [tunnel-client configuration v0.0.11](https://github.com/openai/tunnel-client/blob/v0.0.11/docs/configuration.md): configuration precedence, runtime/admin key split, stdio limits, readiness, and local admin surfaces.
- OpenAI, [connector behavior v0.0.11](https://github.com/openai/tunnel-client/blob/v0.0.11/docs/connectors.md): command forwarding, stdio session limitations, OAuth-header behavior, and reconnect/error semantics.
