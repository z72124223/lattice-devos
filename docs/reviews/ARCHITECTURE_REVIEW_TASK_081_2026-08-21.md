# TASK-081 architecture review — 2026-08-21

## Trigger

The dashboard's public projection semantics now distinguish a historical
functional outcome from current delivery readiness, so this is a public
contract/reliability review.

## Result

No architecture blocker.

Before and after, the `engineering-status-dashboard` module owns only a
disposable local projection and uses Git through read-only argument-array
commands. TASK/ISSUE records remain repository evidence; delivery and archive
authority remain in the separate finisher and Codex App. The added historical
lookup neither writes Git nor adds a database, runtime, MCP, Hermes, package,
network write, or dependency direction.

The safe failure path is explicit: no terminal candidate preserves `UNKNOWN`;
incomparable candidates preserve a partial error; any evidence source that is
not the worktree HEAD becomes `STALE` delivery and cannot dispatch or archive.
SPEC-004 v4 and constitution v1.3 record the changed contract, so no ADR or
constitution amendment outside this module is required.

## Remaining gate

The same preserved PLANS/TASK-078 mismatch blocks `npm.cmd run check`; no
architectural exception or bypass is claimed.
