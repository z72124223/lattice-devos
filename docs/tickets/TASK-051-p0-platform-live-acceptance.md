---
ticket_id: TASK-051
title: P0 current-machine LATTICE MCP platform live acceptance
spec_id: SPEC-003
spec_version: 4
module_id: latticed
constitution_version: 1.7
status: waiting_dependency
parallel_safe: false
depends_on:
  - TASK-050
allowed_paths:
  - docs/tickets/TASK-051-p0-platform-live-acceptance.md
  - scripts/run-task051-p0-platform-live-acceptance.ps1
  - scripts/test-task051-p0-platform-live-acceptance.ps1
  - target/task051-p0-platform-live-acceptance/**
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-051 — P0 current-machine LATTICE MCP platform live acceptance

## Objective

After TASK-050 reaches an accepted completion state, prove on the current machine that Codex can discover the registered LATTICE MCP server, make one real typed call with semantic success, and recover the resulting PostgreSQL-backed task safely from a fresh client/server process after a physical database restart. Preserve the exact four-tool public surface and the existing six-field `lattice_task_status` wire result.

This is an acceptance-only ticket. It does not repair product code or expand the public contract.

## Status vocabulary

- `VERIFIED`: current-run evidence proves the stated behavior and binds it to timestamps and exact source/tree/binary/config/runtime identities.
- `VISIBLE_UNVERIFIED`: an object is visible but has not passed a current semantic invocation.
- `NOT_RUN`: the current authorized run has not attempted the check.
- `WAITING_DEPENDENCY`: a named prerequisite is incomplete or lacks acceptable evidence.
- `FAIL`: an attempted check violated an exact requirement, including `isError=true`, an error envelope, wrong semantics, unexpected effects, or schema drift.
- `UNKNOWN`: available evidence cannot determine the state. `UNKNOWN` is never acceptance.

## Current baseline at ticket creation

| Item | State | Meaning |
| --- | --- | --- |
| TASK-050 implementation and verification | `WAITING_DEPENDENCY` | It is being implemented in its dedicated worktree; this ticket neither reads nor changes that work. |
| MCP registration loaded by the current Codex desktop process | `UNKNOWN` | No current process-bound discovery receipt is accepted by this ticket. |
| Four LATTICE tools discoverable from a fresh Codex process | `WAITING_DEPENDENCY` | Must be checked only after the accepted TASK-050 candidate is registered. |
| Real typed Codex-to-LATTICE call | `NOT_RUN` | No current semantic call is claimed. |
| PostgreSQL durable write, fresh read, and restart recovery | `WAITING_DEPENDENCY` | Requires the accepted TASK-050 candidate and a fresh disposable runtime. |
| Four-tool and six-field regression after TASK-050 | `WAITING_DEPENDENCY` | Historical or static evidence is not current acceptance. |

## Fail-closed prerequisites

Execution may start only when all of the following are recorded in a preflight receipt:

1. TASK-050 has one of these two explicit states:
   - `TASK050_FULLY_VERIFIED`: a clean, identified commit/tree has passed its focused tests, PostgreSQL migration/integration/restart and fresh-process replay, MCP regression checks, and required review with no unresolved P0/P1 finding; or
   - `TASK050_ACCEPTED_FOR_TASK051_MACHINE_GATE`: a clean, identified commit/tree has passed all non-desktop checks, including durable Ledger semantics, lease/fence behavior, unknown-version rejection, PostgreSQL migration/restart coverage, and four-tool/six-field contract tests; the only deferred items are explicitly assigned to TASK-051 as current-machine registration, discovery, invocation, and restart acceptance by an authority reference.
2. No TASK-050 `FAIL`, unknown event/schema version, ambiguous authority digest, unresolved lease/fence violation, dirty candidate, or unowned change remains. Otherwise this ticket stays `WAITING_DEPENDENCY`.
3. Record exact accepted source commit/tree, built `latticed` path and SHA-256, build provenance, Codex executable/version/process identity, MCP configuration path and pre-run hash, PostgreSQL executable identity, run id, data root, port, and database identity. If source-to-binary linkage cannot be proven, stop as `UNKNOWN`.
4. If TASK-050 changed any referenced specification or module version, reconcile this ticket to the accepted versions before execution. Do not infer compatibility.
5. Use a fresh disposable PostgreSQL instance and dynamic port. Exclude ports `5432`, `55432`, `64272`, every current listener, and every cluster without exact run-id ownership. Never stop, inspect destructively, or reuse an unknown cluster.
6. Unknown or extra MCP fields, unknown Ledger/receipt event versions, authority/subject/digest mismatch, stale lease, fence mismatch, or ambiguous process/config generation must fail closed.

## Exact execution and mutation scope

Repository writes are limited to the frontmatter `allowed_paths`. Product source, migrations, existing specifications, existing tickets, `PLANS.md`, `HANDOFF.md`, module constitutions, and repository configuration are outside scope.

The only permitted external mutation during a separately authorized TASK-051 run is the exact `[mcp_servers.lattice]` table in `C:\Users\f7212\.codex\config.toml`, and only when a reversible switch is required to point at the accepted binary. Before changing it, capture the full-file hash and exact prior table bytes; do not alter environment values, credentials, or any other table. Restore the original bytes and verify the original hash during cleanup. If this narrow mutation is not authorized at execution time, report `WAITING_DEPENDENCY`.

All disposable PostgreSQL data, isolated homes, local canary repository, logs, and receipts must live under `target/task051-p0-platform-live-acceptance/<run_id>/`. No other filesystem root is writable under this ticket.

## Acceptance procedure and evidence

### A. Registration and discovery

1. Start a fresh Codex process/config generation after the accepted binary is registered.
2. Capture configuration hash, exact registered executable path/hash, Codex process identity, MCP initialize result, and `tools/list` result.
3. Verify exactly these four LATTICE tools and their accepted schemas are discoverable:
   - `lattice_delivery_run`
   - `lattice_delivery_status`
   - `lattice_task_submit`
   - `lattice_task_status`
4. Visibility alone is `VISIBLE_UNVERIFIED`, not `VERIFIED`.

### B. Minimal real typed call and semantic success

1. From Codex, call `lattice_task_submit` once with the existing closed `CONTROLLED_CODEX_CANARY` contract and a unique bounded `client_request_id` derived from the run id.
2. Require MCP transport success, `isError=false`, the exact success envelope, a valid task reference, and the expected accepted task semantics. Process exit code alone is insufficient.
3. Capture exact request/response hashes with credentials and sensitive environment values redacted.
4. Any error envelope, unexpected downstream effect, schema drift, or ambiguous task state is `FAIL`.

### C. Durable truth and safe replay

1. Prove the submitted task and TASK-050 autonomy receipt are committed to the disposable PostgreSQL Task Ledger with the canonical subject, authority digest, event/schema version, fence, and linked digests expected by the accepted TASK-050 contract. Do not expose this internal receipt on the public MCP wire.
2. From a fresh client session, call `lattice_task_status` with the returned task reference and prove the same durable result is read without re-execution.
3. Stop only run-owned processes, physically restart the same disposable PostgreSQL cluster, launch a new accepted `latticed` process and fresh Codex process, then call `lattice_task_status` again.
4. Require the same task semantics, ledger head/result digests, internal receipt projection, and zero duplicate external/domain effects after fresh read and restart.
5. Record PostgreSQL system identifier, pre/post restart postmaster identities, candidate binary hashes, process/config generations, and effect counters. Same-process cache replay is not acceptance.

### D. Public-contract regression

1. Fresh discovery still exposes exactly the four existing tools; no fifth tool or new public schema is allowed.
2. `lattice_task_status` returns exactly these six fields and no internal receipt field:
   - `schema_version`
   - `status`
   - `task_state`
   - `task_ref`
   - `ledger_head_digest`
   - `result_digest`
3. Existing delivery-tool schemas and task submit/status validation remain exact and fail closed on unknown or extra fields.

### E. Reversible cleanup

1. Stop only processes and the PostgreSQL cluster whose PID, data directory, port, executable hash, and run id all match this run. Unknown ownership means preserve and report `UNKNOWN`.
2. Restore the prior MCP table byte-for-byte if changed, verify its original hash, and confirm a fresh Codex process no longer uses the temporary generation.
3. Preserve canonical redacted evidence before removing run-owned disposable data. Do not remove any unowned path or process.
4. The final receipt must include per-gate status, exact identities/hashes, discovery and semantic-call evidence, PostgreSQL restart evidence, effect counters, cleanup/rollback result, and a final overall state. Any required `UNKNOWN`, `WAITING_DEPENDENCY`, `NOT_RUN`, or `FAIL` prevents `VERIFIED`.

## Acceptance conditions

TASK-051 is `VERIFIED` only when one current run proves all of the following:

- accepted TASK-050 provenance and prerequisites;
- current Codex registration and exact four-tool discovery;
- one real typed submit with semantic success;
- durable PostgreSQL write, fresh-session read, physical restart, new-process safe replay, and no duplicate effects;
- exact four-tool surface and six-field `lattice_task_status` wire output without regression;
- complete reversible cleanup and configuration rollback;
- redacted, canonical, hash-linked evidence binding every result to the same run and identities.

## Non-goals

- No TASK-050 implementation, repair, duplication, or acceptance substitution.
- No product feature fix, migration change, module refactor, new MCP tool/schema/field, or public receipt exposure. A product defect found here is reported as `FAIL` and repaired under a separate authorized ticket.
- No Hermes, Graphify, Codebase Memory, model platform, scheduler, autonomous-control implementation, ChatGPT tunnel, or unrelated module acceptance.
- No Git/GitHub integration, push, PR, merge, default-branch change, deployment, or release. A run-owned local disposable Git repository is permitted only if the existing closed canary requires it; it is an effect counter, not Git platform acceptance.
- No production database, credential/account change, package installation/update, security-control weakening, arbitrary SQL/shell/path surface, or unknown resource cleanup.

## Human gate

Creating this ticket does not authorize its execution. A future explicit TASK-051 execution authorization must cover the listed paths and, if needed, the exact reversible MCP table switch. Merge, deployment, release, destructive cleanup, credentials, and every action outside this ticket remain separate user decisions.
