---
ticket_id: TASK-051
title: P0 current-machine LATTICE MCP platform live acceptance
spec_id: SPEC-003
spec_version: 5
module_id: latticed
constitution_version: 1.8
status: in_progress
parallel_safe: false
depends_on:
  - TASK-050
dependency_state: TASK050_FULLY_VERIFIED
accepted_task050_commit: 8e5ba40d38b781afff7028841bd981c8dd2b9721
accepted_task050_tree: b4478be2801814ffc630cbf113b0a4ffa3a1b591
execution_authorized_by: direct_user_reply
execution_authorized_at_utc: 2026-08-15T08:25:19Z
execution_authorization_source_thread_id: 019ffee6-488a-70e0-8990-9aa9133892a7
product_fix_authorized_by: direct_user_reply
product_fix_authorized_at_utc: 2026-08-16T03:14:27Z
product_fix_authorization_source_thread_id: 019ffee6-488a-70e0-8990-9aa9133892a7
allowed_paths:
  - docs/tickets/TASK-051-p0-platform-live-acceptance.md
  - apps/lattice-runtime/src/git_delivery.rs
  - scripts/run-task051-p0-platform-live-acceptance.ps1
  - scripts/test-task051-p0-platform-live-acceptance.ps1
  - target/task051-p0-platform-live-acceptance/**
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-051 — P0 current-machine LATTICE MCP platform live acceptance

## Objective

Current contract reconciliation:

- TASK-050 is accepted at commit `8e5ba40d38b781afff7028841bd981c8dd2b9721`, tree `b4478be2801814ffc630cbf113b0a4ffa3a1b591`, and is an ancestor of this branch.
- This ticket is reconciled to `SPEC-003` version 5, `SPEC-002` version 35, and latticed constitution version 1.8. No public-contract, module, dependency, or data-ownership amendment is required. On 2026-08-16 the user separately authorized the exact Windows Git long-path defect repair in `apps/lattice-runtime/src/git_delivery.rs` after the live gate proved that product defect.
- The current-machine gate uses an isolated, run-owned `CODEX_HOME` and process-local MCP registration. It does not mutate `C:\Users\f7212\.codex\config.toml`; the original file hash must remain byte-identical.
- The user authorized TASK-051 completion and a non-force feature-branch push in source thread `019ffee6-488a-70e0-8990-9aa9133892a7`. Merge, Draft promotion, default-branch movement, deployment, and release remain unauthorized.

After TASK-050 reaches an accepted completion state, prove on the current machine that Codex can discover the registered LATTICE MCP server, make one real typed call with semantic success, and recover the resulting PostgreSQL-backed task safely from a fresh client/server process after a physical database restart. Preserve the exact four-tool public surface and the existing six-field `lattice_task_status` wire result.

This remains an acceptance-led ticket and does not expand the public contract. Its sole product-code exception is the user-authorized Windows-only, command-scoped `GitRunner::output` repair that adds `-c core.longpaths=true`, plus necessary same-file regression tests. It may not write system/global/local Git configuration, modify `windows_probe_output`, or expand into unrelated product code.

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
| TASK-050 implementation and verification | `VERIFIED` | Accepted commit `8e5ba40d38b781afff7028841bd981c8dd2b9721`, tree `b4478be2801814ffc630cbf113b0a4ffa3a1b591`, is an ancestor of this branch. |
| MCP registration loaded by the current Codex desktop process | `UNKNOWN` | No current process-bound discovery receipt is accepted by this ticket. |
| Four LATTICE tools discoverable from a fresh Codex process | `NOT_RUN` | The dependency is satisfied; current-machine execution is now authorized and pending. |
| Real typed Codex-to-LATTICE call | `NOT_RUN` | No current semantic call is claimed. |
| PostgreSQL durable write, fresh read, and restart recovery | `NOT_RUN` | The dependency is satisfied; the unique marker-owned PostgreSQL 17 run remains pending. |
| Four-tool and six-field regression after TASK-050 | `NOT_RUN` | Historical evidence remains baseline only until the current run completes. |

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

Repository writes are limited to the frontmatter `allowed_paths`. Product source other than the one listed `apps/lattice-runtime/src/git_delivery.rs` exception, migrations, existing specifications, other tickets, `PLANS.md`, `HANDOFF.md`, module constitutions, and repository configuration are outside scope.

The authorized implementation uses a run-owned isolated `CODEX_HOME` and process-local `[mcp_servers.lattice]` registration under the ticket evidence root. `C:\Users\f7212\.codex\config.toml` must not be written. Capture its pre-run hash and require the same byte hash after cleanup. Any attempted persistent registration change is `FAIL`.

All disposable PostgreSQL data, isolated homes, local canary repository, logs, and receipts must live under `target/task051-p0-platform-live-acceptance/<run_slot>/`. The logical `run_id` remains a fresh 32-hex identifier. The physical `run_slot` is a separately recorded 6-hex value deterministically derived from `SHA-256("<run_id>|<slot_attempt>")`; the runner first creates a nonce-qualified owner-only staging directory with a byte-canonical `lattice.task051.run-root.v1` marker, then atomically renames it into the slot. Collisions are skipped without changing or deleting the existing slot, and only a staging directory whose exact marker has been reverified may be removed. No other filesystem root is writable under this ticket.

The controlled delivery parent is the fresh owner-only direct child `<run_root>/x`, published by the same nonce-stage plus canonical-marker atomic-rename discipline so an existing directory or junction is never modified. After submit succeeds, the repository is resolved only from the exact returned task reference as `<run_root>/x/task-<task_ref>/repo`; the legacy `<delivery_root>/repo` spelling is forbidden.

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
- No product feature, migration, module refactor, new MCP tool/schema/field, or public receipt exposure. The only authorized product repair is the already-proven `GitRunner::output` Windows command-scoped long-path defect and its same-file regression tests; any other product defect remains `FAIL` and requires separate authorization.
- No Hermes, Graphify, Codebase Memory, model platform, scheduler, autonomous-control implementation, ChatGPT tunnel, or unrelated module acceptance.
- The live acceptance does not treat Git/GitHub integration as evidence. After verified closure, the separately authorized non-force feature-branch push may publish the checkpoint; PR creation, merge, Draft promotion, default-branch change, deployment, and release remain non-goals without separate authorization. A run-owned local disposable Git repository is permitted only if the existing closed canary requires it; it is an effect counter, not Git platform acceptance.
- No production database, credential/account change, package installation/update, security-control weakening, arbitrary SQL/shell/path surface, or unknown resource cleanup.

## Human gate

The execution gate was explicitly authorized by the user in source thread `019ffee6-488a-70e0-8990-9aa9133892a7` at `2026-08-15T08:25:19Z`. The same user explicitly authorized the listed `apps/lattice-runtime/src/git_delivery.rs` product repair in that source thread on 2026-08-16, before delegating this goal-mode continuation. These gates are consumed only for the listed paths, current-machine acceptance, local reversible resources, bounded commits, and a non-force feature-branch push. They do not authorize PR creation, merge, Draft promotion, default-branch change, deployment, release, account/credential mutation, destructive unowned cleanup, or any action outside this ticket.

## 2026-08-16 execution status

- `264e4db1cda164d3efda7feb81811440a0bb36e5` repairs the authorized Windows Git long-path defect with command-scoped `-c core.longpaths=true`. The exact focused regression, the ten-test `git_delivery` module, formatting, and the acceptance wrapper self-test passed. The full run rooted at slot `daa9c7` then completed the disposable Git delivery with exact `LATTICE_DELIVERY_OK` content and commits, proving that the former `GIT_INIT_FAILED` blocker is closed.
- That run remained `FAIL`, not `VERIFIED`, because the current-Codex submit session produced valid `SESSION_OPEN` and `DISPATCH_ACCEPTED` records but the runner read before the natural `SESSION_CLOSED` record was available. Cleanup evidence proves the disposable PostgreSQL listener/cluster and live child processes were absent, copied credentials were removed, the subst mapping was absent, and the original Codex configuration hash remained unchanged.
- `8031ffb` adds a bounded 30-second natural child-exit wait before the unchanged strict dispatch/effect readers. Red/green self-test evidence covers natural exit, timeout, and already-absent PID behavior; independent code/security and architecture reviews report `P0=0 / P1=0 / P2=0 / P3=0` and `CLEAR`. It does not synthesize or relax `SESSION_CLOSED`, and it does not retry a tool call.
- The one post-fix live invocation stopped at the pre-resource `TASK051_CURRENT_CODEX_REJECTED` gate: the installed current Codex is now `codex-cli 0.148.0-alpha.9`, while this ticket's reviewed runner is pinned to `codex-cli 0.147.0-alpha.6.6`. No new PostgreSQL cluster, Codex child, latticed child, subst mapping, or run slot was created by that invocation.
- Current terminal state is `WAITING_DEPENDENCY`, not `VERIFIED`: a separately reviewed current-Codex identity update and a successful current-machine live run are still required. The feature checkpoint may be non-force pushed for preservation, but the push is not acceptance evidence.

## 2026-08-17 current-machine acceptance attempt

- Commit `f0ea8bc81f211e174cc778bd9c4f49a3e8b1aa69` updates only the TASK-051 runner and wrapper test for the installed current Codex. The reviewed identity is exact path `C:\Users\f7212\AppData\Local\OpenAI\Codex\bin\e305f1c75d8da435\codex.exe`, version `codex-cli 0.148.0-alpha.9`, SHA-256 `f29f609375f3731d8db507a95124862a84e306982e30ba4300ddce5638bc6946`, and user agent `lattice-task051-acceptance/0.148.0-alpha.9 (Windows 10.0.19045; x86_64) unknown (lattice-task051-acceptance; 1)`. An isolated `CODEX_HOME` probe accepted the existing stdio MCP config shape, strict app-server initialize, `mcpServerStatus/list`, the exact four-tool names, and their input/output schema shapes.
- TDD first rejected the stale identity pin with `TASK051_RUNNER_SHAPE_REJECTED`; after the narrow pin update, the wrapper self-test passed. Independent code/security review then identified a P1 preflight-to-launch identity gap. A second RED rejected the missing `Get-Task051CurrentCodexFileIdentity`; GREEN adds per-invocation exact path/non-reparse/SHA checks and binds every launched Codex process to its job, actual image, native identity, SHA, and lifetime before retaining a closeable authority through cleanup. The wrapper `-SelfTestOnly`, Windows PowerShell AST checks for both scripts, `git diff --check`, allowed-path check, focused identity helper, and `npm.cmd run check` all passed. Final independent code/security and architecture reviews are `CLEAR`, each with `P0=0 / P1=0 / P2=0 / P3=0`.
- Exactly one post-fix current-machine live invocation ran from clean source commit `f0ea8bc81f211e174cc778bd9c4f49a3e8b1aa69`, tree `47edf6129fb4611e00a59bec3d6ff876a1173b1f`, in run slot `1f1e26`. It built `latticed` SHA-256 `01fa97da83acb1dd2095864cbc5a245d62c158c5a5257de62de0fb493e43068a`, created a fresh marker-owned PostgreSQL 17 database on dynamic port `49158`, and produced process-bound discovery evidence for current Codex SHA-256 `f29f609375f3731d8db507a95124862a84e306982e30ba4300ddce5638bc6946` and exactly `lattice_delivery_run`, `lattice_delivery_status`, `lattice_task_status`, and `lattice_task_submit`.
- The invocation then failed closed with `TASK051_LIVE_ACCEPTANCE_REJECTED|e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855|TASK038_UNCLASSIFIED_REJECTED`. Preserved evidence bounds the failure after successful discovery and before any submit dispatch or controlled-task database mutation: the only MCP dispatch receipt is the discovery `SESSION_OPEN` with `dispatch_accepted_count=0`, while the controlled-task footprint remains at zero task events, commands, writer commands, and writer transitions. Because the TASK-038 classifier did not retain a more specific underlying exception, no narrower root cause is claimed and no tool call or full live run was retried.
- Cleanup receipts prove `HOLDER_STOPPED`, `CLEANUP_COMPLETED`, and `RECEIPT_CLOSED`; the disposable cluster directory is empty, port `49158` has no listener, no process image or command line remains under run slot `1f1e26`, and no subst mapping remains. The runner's byte-hash rollback guard accepted the original Codex config; its post-run SHA-256 is `69712ba5da5685da34eed9ae8699cb16390b73948931a41d212cfd814feb6fc3`.
- Current terminal state is `FAIL`, not `VERIFIED`. Discovery is proven for this run, but the required real typed submit, durable write, fresh-process status read, physical-restart replay, exact six-field status result, and zero-duplicate-effect proof did not complete. The run evidence is preserved; another live attempt requires a separately reviewed correction and new authorization rather than retrying this acceptance.

## 2026-08-20 current-machine acceptance attempt

- Commits `b4d8ade43f6d1a2324069d96e2780a27842d03ab` and `11d46e450b382a4b1299250a43073f81ddaf31c6` close the prior unclassified-failure evidence gap and harden exact-process cleanup. Unclassified exceptions now produce owner-only, `FileMode.CreateNew` canonical evidence in per-fingerprint files; replay compares exact UTF-8 bytes and rejects a BOM or any content mutation. Cleanup retains exact Windows process handles, waits on those handles before PID-level absence checks, and then closes them. The runner and wrapper self-tests, both PowerShell AST parses, `npm.cmd run check`, `npm.cmd test`, `cargo fmt --check`, and `cargo test --workspace` passed. Independent code/security and architecture reviews are `CLEAR`, each with `P0=0 / P1=0 / P2=0 / P3=0`. The global `cargo clippy --workspace --all-targets --all-features -- -D warnings` remains blocked only by 17 pre-existing warnings in unchanged `lattice-hermes-adapter` paths outside this ticket.
- Exactly one current-machine invocation ran from clean source commit `11d46e450b382a4b1299250a43073f81ddaf31c6`, tree `7fe787cedabd1aa3f510f543391f31740db5323f`, in run slot `ead2b2`. It built `latticed` SHA-256 `3f4530dc3cf7ec51e2be8f7ed801254590158584334684eac74ed5c42c9929b`, created fresh PostgreSQL run `723631a5c6b14358a9b5bbdb306a0c39` on dynamic port `65189`, and used current Codex SHA-256 `18fbf51f77adfc543c9d86c78c0a54553f89ba79236ed8b0a3c48e2a3b4f010e`.
- Process-bound discovery succeeded and exposed exactly `lattice_delivery_run`, `lattice_delivery_status`, `lattice_task_status`, and `lattice_task_submit`. The single required `lattice_task_submit` dispatch was accepted in session `6e9b52da2a2c4bd8a827adcfb9162990`; it returned semantic success and created task reference `85a4f6dac60aaffb9b9de4341d85e8e2a3c7331b09813244674a2d0e1294f719`. The observed-effect chain records one dispatch, one Codex effect, ten database effects, four filesystem effects, ten network effects, five process effects, and final classification `MCP_RESULT`.
- The invocation then failed closed at the strict dispatch-receipt gate. The canonical dispatch file contains only `SESSION_OPEN` and `DISPATCH_ACCEPTED`, and the observed-effect file ends at `PROBE_COMPLETED`; neither contains `SESSION_CLOSED`. The unchanged strict reader requires exactly three dispatch records for one accepted call, so the preserved bytes deterministically classify this as `TASK038_MCP_ACCEPTANCE_EVIDENCE_REJECTED`. No submit tool-call receipt, pre-restart status call, physical restart, post-restart status call, or final acceptance receipt was produced.
- Root cause is a lifecycle-contract mismatch, not another read-before-exit race. `latticed` writes both close records only after MCP stdin reaches EOF. The current official Codex Windows stdio launcher instead terminates the registered MCP process tree with `taskkill /T /F` when its transport closes, bypassing that graceful EOF path; the implementation is visible in `openai/codex` at `codex-rs/rmcp-client/src/stdio_server_launcher.rs`. A fix would require either a separately authorized product-lifecycle change outside the sole `git_delivery.rs` exception or a separately reviewed acceptance-topology change that places a run-owned graceful relay outside the Codex-terminated process tree. This ticket does not authorize either change, and the runner does not synthesize or relax `SESSION_CLOSED`.
- Cleanup completed without touching unrelated resources. The holder receipt proves `HOLDER_STOPPED`, `CLEANUP_COMPLETED`, and `RECEIPT_CLOSED` with `cleanup_complete=true`; the run-owned cluster root and listener are absent, port `65189` has no listener, the `Z:` alias is absent, and no run-owned Codex or `latticed` process remains. Current terminal state is `FAIL`, not `VERIFIED`; another live attempt requires a reviewed correction and new explicit authorization.
