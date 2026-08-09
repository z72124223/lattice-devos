# TASK-038 Architecture Review — 2026-08-09

## Verdict

PASS. Independent final review found no P0-P3 ownership, dependency, authority,
state, or public-contract violation. No constitution or ADR amendment is
required for the zero-argument correction plus additive MCP wire compatibility.

## Boundary Evidence

- Public MCP remains exactly two closed zero-argument tools.
- Composition derives one immutable `SubjectBinding` from
  `fixed_gateway_submission()` and injects it into both normal and full-chain
  MCP entrypoints. The service retains its typed equality check.
- PostgreSQL remains durable truth, Orchestrator owns effect order, and Codex
  writer/lease semantics are unchanged.
- The tunnel launcher owns only validated transport/profile invocation and
  child-environment isolation. It does not parse or retain task, policy,
  approval, lease, workflow, or result state.
- No Cargo dependency, listener, alternate store, adapter-to-adapter call, or
  reverse dependency was added.
- `server/discover`, per-request protocol selection, modern result/cache fields,
  and modern-only annotations remain within the existing bounded MCP stdio
  transport responsibility. They add no gateway action or authority.
- The legacy 64-call bound remains attached to its stateful session. Stateless
  modern requests do not create a process-lifetime pseudo-session or use an MCP
  reserved custom error code.

## Open Architecture Boundary

Official stdio bindings have no HTTP MCP session ID and receive no connector
bearer header. A shared tunnel/profile therefore maps to the same fixed subject;
per-human actor/session authorization, durable audit correlation, and
per-actor rate limiting are not implemented.

Live readiness and exact two-tool ChatGPT discovery subsequently passed, but
that does not close the per-human identity/audit boundary or prove successful
production tool execution.

The next identity slice must explicitly choose a fixed tunnel/profile gateway
actor or a thin authenticated loopback HTTP adapter. The latter is a new
listener/transport/public-contract boundary and requires versioned design,
ADR/constitution review, and approval before implementation. Caller-declared
identity cannot become authority.

The closed environment also intentionally rejects ambient enterprise proxy,
private-CA, and mTLS settings. An enterprise deployment requiring them needs a
separately typed, audited configuration extension.
