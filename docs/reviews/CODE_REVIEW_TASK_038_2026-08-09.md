# TASK-038 Code Review — 2026-08-09

## Verdict

PASS. Independent final review of SPEC-003 v2 and the complete additive
`2026-07-28` compatibility diff found P0-P3 = 0.

## Findings Resolved During Review

1. P1 environment override: official tunnel-client configuration gives
   environment variables precedence over YAML. The launcher now constructs a
   closed child environment containing only bounded Windows variables,
   `CONTROL_PLANE_API_KEY`, and required `LATTICE_*` configuration. The harness
   proves 58 hostile overrides are absent, one LATTICE canary survives, and the
   parent environment is restored.
2. P2 contract coverage: rejection matrices now cover both tools, the retired
   five binding fields, every prohibited property class, and all non-object
   input shapes. The real `latticed` binary executes empty run and omitted-arg
   status calls and reaches bounded downstream errors rather than binding
   rejection.
3. Formatting: the Rust test diff was formatted and the format gate passes.
4. Protocol negotiation: `server/discover` and stateless list/call results now
   match the final schema while the legacy initialize lifecycle remains intact.
5. Downgrade and metadata validation: protocol version, client capabilities,
   optional implementation/icons, log level, progress token, and extension key
   grammar fail closed before dispatch when malformed.
6. Stateless lifetime: modern requests do not consume the legacy 64-call
   pseudo-session counter or emit a reserved modern error code.
7. Tool annotations: Run remains destructive and non-idempotent; Status remains
   read-only. Annotations appear only on the modern catalog.

## Verification Reviewed

- Focused MCP: 21/21 passed.
- Real-binary composition: legacy and modern paths both passed.
- Complete `lattice-runtime` test package: passed.
- Hostile-environment tunnel harness and PowerShell AST parsing: passed.
- Format, project checks, Node tests, and diff whitespace gate: passed.
- Strict scoped Clippy passes with the exact unchanged `manual_inspect`
  baseline exception. Baseline-wide Clippy also reports eleven unchanged
  `lattice-hermes-adapter` lints outside TASK-038.

## Residual Risks

- The official stdio child shares the trusted tunnel process environment and
  therefore sees the runtime key. This is a same-user trust-boundary residual,
  not an exposed MCP surface; stricter credential compartmentalization needs a
  separate native child-environment design.
- TASK-037 production acceptance was not retried.
- Live Phase 1 discovery later passed independently of this source review;
  successful production tool execution and per-human actor/session authority
  remain outside the reviewed slice.
