---
ticket_id: TASK-038
spec_id: SPEC-003
spec_version: 4
module_id: latticed
constitution_version: 1.4
status: in-progress
parallel_safe: false
depends_on:
  - TASK-014
  - TASK-021
  - TASK-033
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-contracts/src/delivery.rs
  - crates/lattice-contracts/src/task_control.rs
  - crates/lattice-contracts/tests/contracts.rs
  - crates/lattice-contracts/tests/delivery_contracts.rs
  - crates/lattice-contracts/tests/task_ingress_contracts.rs
  - crates/lattice-contracts/tests/task_control_contracts.rs
  - crates/lattice-task-domain/src/spec.rs
  - crates/lattice-task-domain/tests/task_domain.rs
  - crates/lattice-writer-lease/src/lib.rs
  - crates/lattice-writer-lease/tests/writer_lease.rs
  - crates/lattice-postgres-writer-lease/Cargo.toml
  - crates/lattice-postgres-writer-lease/src/lib.rs
  - crates/lattice-postgres-writer-lease/src/adapter.rs
  - crates/lattice-postgres-writer-lease/src/setup.rs
  - crates/lattice-postgres-writer-lease/tests/adapter_api.rs
  - crates/lattice-postgres-writer-lease/tests/setup_api.rs
  - crates/lattice-postgres-writer-lease/tests/extension_contract.rs
  - crates/lattice-postgres-writer-lease/tests/postgres_live.rs
  - db/extensions/writer-lease/v1.sql
  - crates/lattice-postgres-store/src/postgres_setup.rs
  - crates/lattice-postgres-store/src/task_ledger.rs
  - crates/lattice-postgres-store/tests/postgres_setup_api.rs
  - crates/lattice-postgres-store/tests/postgres_live.rs
  - crates/lattice-ports/Cargo.toml
  - crates/lattice-ports/src/lib.rs
  - crates/lattice-ports/tests/delivery_ports.rs
  - crates/lattice-ports/tests/task_control_ports.rs
  - crates/lattice-orchestrator/Cargo.toml
  - crates/lattice-orchestrator/src/lib.rs
  - crates/lattice-orchestrator/tests/delivery_order.rs
  - crates/lattice-orchestrator/tests/controlled_task.rs
  - crates/lattice-codex-adapter/Cargo.toml
  - crates/lattice-codex-adapter/src/delivery.rs
  - crates/lattice-codex-adapter/src/identity.rs
  - crates/lattice-codex-adapter/src/process.rs
  - crates/lattice-codex-adapter/tests/delivery_port.rs
  - crates/lattice-codex-adapter/tests/process.rs
  - apps/lattice-runtime/Cargo.toml
  - apps/lattice-runtime/src/lib.rs
  - apps/lattice-runtime/src/mcp.rs
  - apps/lattice-runtime/src/composition.rs
  - apps/lattice-runtime/src/delivery_ledger.rs
  - apps/lattice-runtime/src/git_delivery.rs
  - apps/lattice-runtime/src/task_control.rs
  - apps/lattice-runtime/tests/mcp.rs
  - apps/lattice-runtime/tests/composition.rs
  - apps/lattice-runtime/tests/dispatch.rs
  - apps/lattice-runtime/tests/task_control.rs
  - scripts/run-task019-postgres.ps1
  - scripts/run-task037-full-chain-verification.ps1
  - scripts/run-task038-task-submit.ps1
  - scripts/task038-local-process-environment.ps1
  - scripts/test-task038-local-acceptance.ps1
  - scripts/test-task038-child-environment.ps1
  - scripts/start-chatgpt-mcp-tunnel.ps1
  - scripts/test-chatgpt-mcp-tunnel-entrypoint.ps1
  - docs/adr/ADR-023-bounded-mcp-task-dispatch-and-postgres-writer-lease.md
  - docs/modules/latticed/MODULE_CONSTITUTION.md
  - docs/modules/lattice-contracts/MODULE_CONSTITUTION.md
  - docs/modules/task-domain/MODULE_CONSTITUTION.md
  - docs/modules/lattice-ports/MODULE_CONSTITUTION.md
  - docs/modules/orchestrator-runtime/MODULE_CONSTITUTION.md
  - docs/modules/writer-lease/MODULE_CONSTITUTION.md
  - docs/modules/postgres-writer-lease/MODULE_CONSTITUTION.md
  - docs/modules/postgres-store/MODULE_CONSTITUTION.md
  - docs/modules/codex-adapter/MODULE_CONSTITUTION.md
  - docs/specs/SPEC-003-chatgpt-mcp-gateway.md
  - docs/tickets/TASK-038-chatgpt-mcp-gateway.md
  - docs/roadmap/TASK-038-MCP-COMPATIBILITY.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_038_2026-08-09.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_038_2026-08-09.md
  - docs/reviews/CODE_REVIEW_TASK_038_2026-08-09.md
  - docs/reviews/INTEGRATION_TASK_038_2026-08-09.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - PLANS.md
  - HANDOFF.md
likely_files:
  - crates/lattice-contracts/src/task_control.rs
  - crates/lattice-writer-lease/src/lib.rs
  - crates/lattice-postgres-writer-lease/src/adapter.rs
  - db/extensions/writer-lease/v1.sql
  - crates/lattice-ports/src/lib.rs
  - crates/lattice-orchestrator/src/lib.rs
  - apps/lattice-runtime/src/mcp.rs
  - apps/lattice-runtime/src/task_control.rs
  - scripts/run-task038-task-submit.ps1
branch: feature/task-038-chatgpt-mcp
---

## Objective

Deliver SPEC-003 v4 Phase 2: ChatGPT discovers and invokes bounded
`lattice_task_submit` / `lattice_task_status`; one fixed server-owned
tunnel/profile actor can submit only `CONTROLLED_CODEX_CANARY`; the same
`latticed` Gateway/Orchestrator constructs a complete Task Spec 2.1, persists
authoritative lifecycle/idempotency/audit in PostgreSQL, acquires a real
PostgreSQL Writer Lease/fence, dispatches the sole controlled Codex writer,
verifies and commits the bounded change, and supports fresh-process durable
status replay.

This ticket is executed before TASK-037 production-chain repair. It does not
claim TASK-037 or full downstream production acceptance.

## Acceptance Criteria

- [x] Canonical `latticed` discovery returns exactly four tools. Both delivery
      schemas remain unchanged empty objects and both task schemas are
      exact/closed.
- [x] Alternate `lattice-full-chain` advertises only the two delivery names,
      fixed-denies Delivery Run before service effects, and treats both task
      tools as unknown in legacy and stateless MCP.
- [x] Submit accepts only `CONTROLLED_CODEX_CANARY` plus one bounded
      `client_request_id`; Status accepts only the returned lowercase SHA-256
      `task_ref`.
- [x] Shell, SQL, path, arbitrary task/prompt, Git/test command, credential,
      provider, actor/session, lease/fence, and writable-thread properties are
      rejected before Gateway service dispatch.
- [x] The actor/audit binding comes only from verified process-start
      configuration; production Secure MCP Tunnel and local canonical
      acceptance use distinct non-substitutable commitments. `clientInfo` and
      caller data grant no authority.
- [x] The server constructs and Task Domain validates one complete Task Spec
      2.1; its digest is identical across Gateway, Task Ledger, Writer Lease,
      Codex, verification/Git, and status.
- [x] PostgreSQL proves exact retry, different-key substitution denial,
      fixed-profile audit, legal task transitions, durable terminal outcome,
      and restart replay.
- [x] Writer Lease 1.1 canonical snapshot/repository contracts pass; the
      independent PostgreSQL Writer Lease v1 extension proves exact install/
      verify, atomic current authority, concurrent single writer, monotonic
      non-reused fencing, heartbeat/release, restart, stale-fence denial, and
      ambiguous-outcome reconciliation.
- [x] No `FakeWriterLease`, hard-coded authority head, or synthetic epoch/fence
      can reach production Task Submit acceptance.
- [x] One canonical-local MCP submit reaches LATTICE, then Codex only after
      fixed ingress admission, Task Domain validation, and current lease
      evidence. Codex changes only the template-owned scope. This does not
      satisfy the separate real ChatGPT gate below.
- [x] No fake or caller-projected Project Registry authority is introduced to
      manufacture a Policy allow. This fixed canary does not claim the missing
      durable Project Registry / live Policy composition required before any
      project-selectable or general development task is exposed.
- [x] Fixed verification passes before exactly one local Git commit, and all
      evidence remains bound to the same Task Spec/lease/fence/worktree.
- [x] Fresh execution is greater than the 30-second finalization reserve and no
      greater than the fixed 300-second Task budget, which remains below the
      600-second Writer Lease TTL. Longer/general profiles remain closed until
      heartbeat, governed interruption, and orphan recovery exist.
- [x] Task Status exposes only the public allowlisted projection and never raw
      spec/prompt/diff/path/command/SQL/secret/lease/fence/process/database
      detail.
- [x] After PostgreSQL and `latticed` restart and in a new MCP session, Status
      returns the identical durable terminal projection with zero repeated
      Codex, verification, or Git effects. Graphify, Hermes, and Memory remain
      zero for the entire TASK-038 Task capability run.
- [x] Existing Phase 1 delivery-tool legacy/stateless and tunnel behavior
      remains compatible.
- [x] The non-MCP `lattice-runtime delivery-run` command is restricted to the
      exact visibly scripted fixture; official Codex mode fails before effects
      and cannot mint MCP/tunnel ingress evidence.
- [x] Focused/full tests, format, changed-slice strict Clippy, project checks,
      PowerShell harness tests, and repository diff checks have current zero-
      exit evidence. Full-workspace strict Clippy retains the separately owned
      eleven-lint Hermes baseline and is not relabeled PASS.
- [ ] A refreshed real Secure MCP Tunnel / ChatGPT session discovers and invokes
      `lattice_task_submit`, and a separate new ChatGPT request/session reads
      the same terminal result through `lattice_task_status`. Canonical local
      MCP evidence cannot satisfy this item.
- [x] TASK-037 remains open until its later production verifier independently
      proves `Hermes -> Memory -> Status`.

## Non-Goals

Free-form development tasks, arbitrary projects/paths/commands/SQL/tests/Git,
GPT-controlled lease/fence/writer thread, general approval/stop/release,
per-human identity, another gateway/orchestrator/truth, global migration
changes, TASK-037 repair, push, merge, deployment, release, or public exposure.

## Module And Constitution Constraints

- `latticed` 1.4 is the only official MCP/composition/Gateway writer entry.
- Contracts 1.12 and Ports 1.8 expose neutral closed values/traits only; Ports'
  only new dependency is Task Domain 2.2's closed `TaskState` value.
- Task Domain 2.2 owns and exports the complete validated canonical subject/
  document; no caller rebuilds a reduced Task Spec carrier.
- Orchestrator 2.3 alone orders task effects and appends workflow events.
- Task Domain 2.1 owns complete Task Spec validation; Task Ledger 2.1 owns
  event/idempotency/replay semantics; neither is duplicated in adapters.
- Writer Lease 1.1 owns semantics, snapshot/checkpoint bytes, and its repository
  trait. `postgres-writer-lease` 1.0 owns only physical persistence.
- Postgres Store 1.6 recognizes the exact combined catalog/ACL profile and may
  invoke only the fixed 15-field `writer_lease_assert_current_v1` predicate in
  the same transaction as a fenced Task Ledger append. It does not install,
  mutate, parse, depend on the lease crates, or own Writer Lease state.
- Codex Adapter 1.2 remains the sole supervised product-code writer lane.
- Global migrations `0001` through `0004` and Codebase Memory v2 extension are
  immutable in this ticket; no `0005` is introduced.

## Dependencies And Overlap

Not parallel-safe: shared Contracts, Ports, Orchestrator, runtime composition,
PostgreSQL harness, and current governance files define one security-sensitive
binding. TASK-038 Phase 1 commits remain the base. TASK-037 implementation is
not modified or retried until this ticket's bounded task capability is
complete and evidenced.

## TDD Behaviors

1. RED/GREEN exact four-tool discovery and closed task schemas while preserving
   both delivery schemas.
2. RED/GREEN fixed actor and hostile `clientInfo`/prohibited-property denial
   before service dispatch.
3. RED/GREEN server template -> complete Task Spec -> one digest mutation
   matrix across every owner/stage.
4. RED/GREEN `TaskLifecyclePort` admission/transition/result/load plus
   TaskCreated/idempotency/audit/status PostgreSQL
   replay and concurrency behavior.
5. RED/GREEN Writer Lease canonical bytes/repository conformance, then live
   extension install/currentness/fencing/restart/fault behavior.
6. RED/GREEN exact Orchestrator order with first-failure suppression and zero
   writer calls before current lease evidence.
7. RED/GREEN controlled Codex scope/fixed-test/single-commit behavior and
   fake/synthetic/stale/cross-binding denials.
8. RED/GREEN fresh-process Status equality and zero external-effect footprint.

## Verification

| Check | Command or service | Required evidence |
|---|---|---|
| Contracts/Task Domain | `cargo test -p lattice-contracts -p lattice-task-domain --locked` | closed values, complete spec, digest/lease/status substitution matrices pass |
| Writer Lease pure | `cargo test -p lattice-writer-lease --locked` | planner/snapshot/repository/fencing matrices pass |
| PostgreSQL Writer Lease | `cargo test -p lattice-postgres-writer-lease --locked` plus ignored live suite through the harness | extension closure, concurrent writer, restart, monotonic fence, stale denial pass |
| Store profile | focused Postgres Store setup/live tests | exact combined profile passes; partial/extra/drift/direct grants fail |
| Ports/Orchestrator | `cargo test -p lattice-ports -p lattice-orchestrator --locked` | trait separation and exact effect/failure order pass |
| Codex/runtime | `cargo test -p lattice-codex-adapter -p lattice-runtime --locked` | lease-bound writer plus four-tool real-binary composition pass |
| Canonical local canary | `powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/run-task038-task-submit.ps1` | canonical `latticed`, disposable PostgreSQL/Git, real controlled Codex, restart status equality, zero rerun footprint; does not claim a ChatGPT tunnel invocation |
| Repository | format, changed-slice strict Clippy, workspace tests, `npm.cmd run verify`, `git diff --check` | current zero exits; unchanged baseline exceptions recorded exactly |

## Human Gate

The user explicitly selected TASK-038 first, the server-owned fixed
tunnel/profile actor, and the single `CONTROLLED_CODEX_CANARY` intent on
2026-08-09. That authorizes bounded local implementation and acceptance using
the existing private tunnel, supplied configuration, disposable PostgreSQL,
isolated Git, and controlled Codex. It does not authorize broader templates,
public exposure, push, primary-branch merge, deployment, release, payment,
account changes, or secret disclosure.

## Current Evidence State

Canonical-local Phase 2 acceptance passed on 2026-08-10 through the production
`latticed` executable, not `lattice-full-chain` or a mock executable:

- acceptance ID `8c21e96b9bc44b1d87de0dea884b9678`;
- evidence `target/lattice-delivery/8c21e96b9bc44b1d87de0dea884b9678/evidence/final.json`;
- canonical `latticed` SHA-256
  `130ef9f92f2582055d9828828c95526a58f01aa7772e43c4db31062219d278b2`;
- task ref `ab8724dd51419cf190ad491f1f8973894bca56dc0c3aed55ebc3723f6214177d`;
- ledger head `f3d3b84625f80f26e90b6ed06514bb4f7e8a65f112c089fb6d8a3c82b7d6cdc2`;
- result `457bab1f71b5bd69e99f3240ca170a25fb88895b94474efd7d747916b2c86bcd`;
- fresh second `latticed` PID returned the exact six-field terminal projection
  after a physical PostgreSQL stop/start, with identical Git, database, and
  timestamp-sensitive Codex Home footprints;
- one Codex invocation, one verified outcome, exactly one controlled Git
  commit, Writer Lease fencing high-water `1`, two immutable lease commands,
  two transitions, and no current writer remained;
- Graphify/Hermes/Memory effect delta was zero; the execution home was removed
  and the read-only credential source remained unchanged;
- real Writer Lease and Store profile live suites passed without `SKIP:`.
- terminal Status and `Merging + result` recovery replay existing Writer Lease
  snapshot/checkpoint and physical history; absent or drifted history fails
  before authoritative completion.

Current repository evidence: workspace tests, changed-slice strict Clippy,
format, `npm.cmd run verify`, TASK-038 static tests, tunnel entrypoint tests,
and `git diff --check` pass. Full workspace strict Clippy is not marked PASS:
it stops on eleven pre-existing lints in unchanged
`crates/lattice-hermes-adapter`; TASK-038 does not expand into that TASK-041/042
slice.

TASK-038 remains `in-progress` until the actual Secure MCP Tunnel is refreshed
and a real ChatGPT session discovers/invokes both new task tools, followed by a
separate-session status replay. The local scope is explicitly
`LOCAL_CANONICAL_MCP_NOT_CHATGPT_TUNNEL` and is not relabeled as ChatGPT proof.
