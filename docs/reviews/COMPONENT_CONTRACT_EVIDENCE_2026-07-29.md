# Component Contract Evidence — 2026-07-29

## Purpose

This is a dated research snapshot used to design replaceable adapters. It is
not installation evidence, a lockfile, or a claim that any live integration is
currently compatible. A future runtime pin must identify the exact installed
binary, binary digest, generated protocol/schema digest where available,
observed feature probes, observation time, and expiry policy.

## Research Snapshot

| Component | Evidence version | Confirmed design input | LATTICE must not assume |
|---|---|---|---|
| OpenClaw | `v2026.7.1` documentation | A TypeScript/ESM plugin can expose a command and can use a Codex harness | That the harness must own LATTICE's writable Codex process, or that documentation proves a local installed runtime |
| Codex | local CLI `0.144.6`; repository tag `rust-v0.144.6`; current official manual | app-server uses an initialize/initialized handshake, JSON-RPC-shaped messages over stdio JSONL, thread/turn APIs, interruption, schema generation, and token-usage events | A stable protocol, complete automatic capability negotiation, worktree confinement, or native monetary-cost accounting |
| Graphify | `v0.9.24` documentation | Code extraction produces graph artifacts and distinguishes extracted from inferred relationships | That Graphify is globally read-only, byte-for-byte reproducible, or safe to run install/hook behavior inside a product repository |
| Hermes | `v2026.7.20` documentation and security policy | Programmatic/API integration and an optional Codex app-server runtime exist; Hermes has its own memory, skills, tool, and session surfaces | Exact-schema/provenance output, security isolation from a profile alone, or authority for LATTICE memory, approval, code, or release state |

## Adapter Consequences

### OpenClaw

- The first vertical slice defines a fake OpenClaw IPC client and typed
  submit/status/stop contract.
- The live plugin is a later exact-version preflight.
- OpenClaw may initiate or display an approval workflow, but a normal gateway
  session alone cannot satisfy a protected release-promotion approval.

### Codex

- Pin the exact executable identity, version, and digest.
- Generate the protocol schema using that exact binary and bind its digest to
  the capability observation.
- Run explicit feature probes for every method/notification LATTICE needs.
- Use a dedicated LATTICE-owned `CODEX_HOME`; verify the initialized
  `codexHome`. The user's normal Codex home is outside scope without explicit
  approval.
- Enforce worktree confinement through LATTICE RPC allowlists, a fixed
  permission profile, independently verified OS/process containment, and an
  exact post-run Git/path Scope Check. app-server is not assumed to provide
  this boundary.
- Record emitted token usage when present. Monetary cost is derived only from a
  separately pinned pricing/account model and otherwise remains `unknown`.

### Graphify

- Never invoke `graphify install`, hook installation, or repository mutation
  flows.
- Mount or expose the product source snapshot read-only.
- Direct all writable output to a separate LATTICE-owned artifact staging
  directory.
- The initial code-only lane forbids semantic backends, live PostgreSQL
  introspection, and optional external integrations.
- If an exact version cannot place output outside the source root, preflight
  fails closed.
- Graph bytes are derived and rebuildable for a pinned input/tool/config tuple.
  Byte-for-byte reproducibility is a LATTICE acceptance test, not an upstream
  guarantee.

### Hermes

- A dedicated `HERMES_HOME`/profile is state separation, not a security
  boundary.
- The whole process must run under independently enforced OS containment with
  read-only product input, a separate candidate-output directory, and no Git or
  database credentials.
- Settings such as memory/skill write approval and guard-agent creation are
  defense in depth only.
- Hermes may return arbitrary or malformed output. LATTICE accepts only a
  versioned, schema-valid candidate envelope with required provenance; anything
  else is rejected or quarantined.

## Primary Sources

- [OpenClaw Codex harness, v2026.7.1](https://github.com/openclaw/openclaw/blob/v2026.7.1/docs/plugins/codex-harness.md)
- [OpenClaw plugin construction, v2026.7.1](https://github.com/openclaw/openclaw/blob/v2026.7.1/docs/plugins/building-plugins.md)
- [Codex app-server README, rust-v0.144.6](https://github.com/openai/codex/blob/rust-v0.144.6/codex-rs/app-server/README.md)
- [Current official Codex app-server manual](https://learn.chatgpt.com/docs/app-server.md)
- [Graphify README, v0.9.24](https://github.com/Graphify-Labs/graphify/blob/v0.9.24/README.md)
- [Hermes programmatic integration, v2026.7.20](https://github.com/NousResearch/hermes-agent/blob/v2026.7.20/website/docs/developer-guide/programmatic-integration.md)
- [Hermes API server, v2026.7.20](https://github.com/NousResearch/hermes-agent/blob/v2026.7.20/website/docs/user-guide/features/api-server.md)
- [Hermes Codex runtime, v2026.7.20](https://github.com/NousResearch/hermes-agent/blob/v2026.7.20/website/docs/user-guide/features/codex-app-server-runtime.md)
- [Hermes security policy, v2026.7.20](https://github.com/NousResearch/hermes-agent/blob/v2026.7.20/SECURITY.md)
