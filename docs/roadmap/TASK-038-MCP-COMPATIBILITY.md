# TASK-038 Phase 1 — ChatGPT MCP Compatibility

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
- `apps/lattice-runtime/src/mcp.rs` currently exposes only
  `lattice_delivery_run` and `lattice_delivery_status`, has closed tool
  dispatch, rejects unknown names and extra call fields, and bounds both frame
  size and invocation count. The tunnel must preserve that surface unchanged.
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

## Contract Drift To Resolve Before Phase 2

`docs/modules/latticed/MODULE_CONSTITUTION.md` describes two zero-parameter
tools, while the current `apps/lattice-runtime/src/mcp.rs` implementation
requires a fixed five-field immutable binding. Both forms remain bounded and
reject arbitrary task selection, but they are not the same public schema.

Phase 2 must choose and test one exact published schema through the normal
constitution/specification change process. It must not add generic shell, SQL,
filesystem, Git, credential, provider, or arbitrary-task inputs.

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
