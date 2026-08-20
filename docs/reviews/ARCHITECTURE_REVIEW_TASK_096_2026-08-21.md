# TASK-096 architecture review

## Trigger

The compatibility CLI and bounded MCP error output gain additive closed
terminal-cause fields.

## Result

No architecture blocker.

The change stays inside the `latticed` composition edge. It neither changes
orchestrator effect order nor adds a dependency, port, tool, durable store,
adapter-to-adapter call, or caller-controlled input. The two existing MCP tools
remain closed zero-argument tools. PostgreSQL, Graphify, and Codex adapter
behavior are untouched.

The new contract is intentionally edge-only: it validates the already typed
stage/code after durable receipt equality, then projects static text. This
preserves One Gateway, One Truth, One Writer and the existing
reconciliation-required semantics.

## Residual risk

Future delivery adapters introducing a new cause code will fail closed until
their code is deliberately added to the composition allowlist and its tests.
That is the intended compatibility and privacy boundary.
