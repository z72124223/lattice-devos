---
spec_id: SPEC-003
status: ready
version: 4
modules:
  - module_id: latticed
    constitution_version: 1.4
  - module_id: lattice-contracts
    constitution_version: 1.12
  - module_id: lattice-ports
    constitution_version: 1.8
  - module_id: orchestrator-runtime
    constitution_version: 2.3
  - module_id: writer-lease
    constitution_version: 1.1
  - module_id: postgres-writer-lease
    constitution_version: 1.0
  - module_id: postgres-store
    constitution_version: 1.6
  - module_id: codex-adapter
    constitution_version: 1.2
  - module_id: task-domain
    constitution_version: 2.2
---

# ChatGPT MCP Gateway

## Problem

TASK-038 Phase 1 proved private Secure MCP Tunnel discovery/invocation for the
two closed delivery tools. ChatGPT still cannot submit a governed development
task through LATTICE. Mapping a new tool directly to the fixed delivery helper,
PostgreSQL adapter, Codex child, or an MCP-specific queue would bypass One
Gateway, create a second workflow/truth path, and falsely treat the current
synthetic delivery authority as a production Writer Lease.

The next bounded capability must let ChatGPT express one typed high-level
intent while LATTICE remains responsible for the complete Task Spec,
lease/fencing, writer, verification, Git, durable audit/status, and downstream
ordering. It must not forge a live Project Registry authority merely to claim
that the broader Policy composition already exists.

## Intended Behavior

### Phase 1 Compatibility

The official private Secure MCP Tunnel continues to launch the one `latticed`
stdio composition. Legacy MCP `2025-11-25` and stateless `2026-07-28` clients
retain the existing discovery/call behavior. These two tools remain unchanged:

- `lattice_delivery_run`: closed empty object;
- `lattice_delivery_status`: closed empty object.

### Phase 2 Task Tools

Discovery adds:

- `lattice_task_submit`;
- `lattice_task_status`.

`lattice_task_submit` has this closed semantic input:

```json
{
  "intent": "CONTROLLED_CODEX_CANARY",
  "client_request_id": "bounded-caller-retry-key"
}
```

Only the exact intent is valid. `client_request_id` uses the existing bounded
safe identifier alphabet and the public 64-byte MCP limit. Both fields are required and
`additionalProperties` is false.

`lattice_task_status` accepts exactly one lowercase SHA-256 `task_ref` returned
by Submit. The reference can select only an already admitted task in the fixed
server profile and grants no mutation authority.

The MCP adapter derives one closed ingress actor from verified process-start
configuration and invokes the same `FullChainService` / Orchestrator
composition used by `latticed`; it creates no second gateway or workflow
owner. Production Secure MCP Tunnel evidence and local canonical acceptance
evidence use distinct non-substitutable actor/adapter commitments. MCP
`clientInfo` remains informational and cannot influence actor, session, Task
Spec, project, lease, or writer authority.

### Server-Owned Task Template

For `CONTROLLED_CODEX_CANARY`, LATTICE owns project, snapshot, repository/base,
repository-relative scope, declared operation, fixed verification, capability,
budget, approval requirement, workspace, and Codex prompt template. The server
constructs the complete Task Spec 2.1 and revalidates it through Task Domain
before Task creation.

The resulting `spec_digest` must be identical in:

- Gateway Submit/Status binding;
- Task Ledger stream and command/audit receipts;
- Writer Lease identity/current-head evidence;
- Codex request and result;
- scope, fixed-verification, and Git evidence;
- durable task status.

The previous five-field fixed submission, `{profile, run_id}` digest, and
synthetic delivery identity cannot substitute.

### Governed Execution

Orchestrator alone owns this effect order:

1. validate the fixed peer/action and complete Task Spec;
2. append TaskCreated plus fixed-profile admission/audit to PostgreSQL;
3. acquire and independently re-read a live PostgreSQL Writer Lease;
4. prepare the bounded workspace;
5. invoke the sole typed Codex writer with the same spec/lease/fence/worktree;
6. inspect changed scope and run the server-selected fixed verification;
7. create one local Git commit only after passing evidence;
8. release the lease or record typed reconciliation;
9. append durable outcome/status.

The first failure or uncertainty stops all later effects. TASK-038 Phase 2 is
implemented and verified before resuming TASK-037 production-chain repair.
This ordering does not mark the later TASK-037 `Hermes -> Memory -> Status`
gate complete. Task Submit/Status is deliberately `WriterOnly`: its acceptance
must prove that Graphify, Hermes, and Memory effects remain zero. The compatible
Delivery Run tool uses the same governed writer first and may continue to the
separately gated downstream chain only after writer release and durable Task
completion.

The fixed canary process deadline is greater than the 30-second finalization
reserve and no greater than its Task Spec budget of 300 seconds. Its Writer
Lease TTL is 600 seconds, so this profile cannot outlive the lease and does not
require a heartbeat loop. Any future longer-running/general task profile must
add governed heartbeat, interrupt, and orphan recovery before exposure.

### PostgreSQL Truth And Status

Task lifecycle, exact idempotency, fixed actor/profile audit, Writer Lease
state/fencing, outcomes, and status are durable
PostgreSQL facts. Process memory and the MCP/tunnel session are not truth.

The first profile contains exactly one server-owned
`CONTROLLED_CODEX_CANARY` task subject. Exact retry returns the same accepted
task/receipt. A different key after that subject is admitted is denied and
does not invoke Codex. Broader task multiplicity and quotas are out of scope.

Task Status opens the configured PostgreSQL-backed owners, verifies replay
against independent current heads/checkpoints, and returns an allowlisted
projection. A new process/new MCP session must return the same durable terminal
projection without rerunning external effects.

## User Stories Or System Scenarios

1. ChatGPT refreshes canonical `latticed` and discovers exactly four bounded
   LATTICE tools.
2. GPT submits the one typed controlled intent without selecting a path,
   command, SQL, credential, lease, writer, or test.
3. The same `client_request_id` can be retried safely after transport
   uncertainty.
4. LATTICE rejects a changed request under the same key and a second in-flight
   canary before any writer effect.
5. LATTICE constructs one Task Spec and sends the task to Codex only after
   fixed ingress/Task Domain admission and a current PostgreSQL Writer
   Lease/fence pass.
6. Scope, fixed verification, and Git completion are bound to the same task and
   lease; failure stops the chain.
7. ChatGPT sees only a bounded public state/result projection.
8. A later ChatGPT request/session can query the same result from PostgreSQL
   without rerunning the task.

## Goals

- Add useful typed Task Submit/Status without a generic execution surface.
- Preserve one `latticed`, one service/composition root, one Orchestrator, PostgreSQL
  truth, and Codex as the sole product-code writer.
- Replace fake/synthetic writer authority with a real PostgreSQL Writer Lease
  and monotonic fencing for formal task dispatch.
- Make task lifecycle, idempotency, audit, and status restart-
  durable.
- Preserve exact Phase 1 delivery-tool compatibility.

## Non-Goals

- Free-form task/prompt submission, arbitrary repositories, paths, commands,
  SQL, verification, Git operations, provider settings, or credentials.
- GPT acquiring a lease/fence, selecting a writer thread, controlling Codex,
  approving/rejecting/stopping tasks, or invoking protected release.
- A second MCP server, HTTP listener, gateway service, orchestrator, queue,
  database, SQLite/file truth, or direct MCP-to-adapter call.
- Per-human ChatGPT identity, OAuth/bearer authorization, or arbitrary
  per-actor quota algorithms in this first fixed-profile slice.
- Modifying global migrations `0001` through `0004`, creating `0005`, or
  placing Writer Lease state in the Codebase Memory extension.
- Claiming TASK-037 production `Hermes -> Memory -> Status` acceptance before
  its later real verifier passes.
- Push, primary-branch merge, deployment, release, public exposure, payment,
  or account/credential changes.

## Constraints

- One Gateway, One Truth, One Writer.
- Server-owned fixed tunnel/profile actor; `clientInfo` is never authority.
- Complete Task Spec 2.1 is built and validated server-side.
- One spec digest binds every control and writer stage.
- PostgreSQL Writer Lease uses the independent exact
  `db/extensions/writer-lease/v1.sql`; no Fake/synthetic acceptance path.
- Only public allowlisted status fields may cross MCP.
- All secrets remain process-private and absent from schemas, results, errors,
  ordinary logs, audit rows, and test evidence.

## Module Impact

- `latticed` 1.4: the canonical public tool list expands from two to four; both task tools
  map into the existing `FullChainService` / Orchestrator composition under
  one fixed server actor.
  Its Task lifecycle edge wrapper implements `TaskLifecyclePort` over the
  existing `PostgresTaskLedger` public append/replay API without owning Task
  Domain legality or Ledger semantics.
- `lattice-contracts` 1.12: adds closed controlled-task/fixed-profile values and
  unified Task-Spec/lease/fence writer/status binding.
- `lattice-ports` 1.8: adds a neutral `TaskLifecyclePort`, the explicit one-way
  Task Domain 2.2 `TaskState` dependency, and lease-bound sole-writer request
  without concrete I/O types.
- Task Domain 2.2: exports the complete normalized Task Spec 2.1 canonical
  subject/document so every boundary uses the domain-owned hash carrier.
- `orchestrator-runtime` 2.3: owns bounded Submit/Status and exact admission/lease/
  writer/verification/Git/durable-status order.
- `writer-lease` 1.1: owns canonical snapshot/checkpoint bytes and the sole
  abstract repository trait while remaining pure/no-I/O.
- `postgres-writer-lease` 1.0: new independent extension installer/verifier and
  PostgreSQL repository implementation; it owns physical persistence only.
- `postgres-store` 1.6: read-only recognition of the exact combined V3 +
  Codebase Memory v2 + Writer Lease v1 profile plus only the fixed 15-field
  current-authority predicate in the same transaction as a fenced Task Ledger
  append; no lease install/state mutation/repository dependency/ownership.
- `codex-adapter` 1.2: accepts only the Task-Spec/lease/fence/worktree-bound
  production writer request and retains one supervised writable child.
- Task Ledger 2.1, Policy, Gateway IPC 1.1, Codebase Memory,
  Graphify, Hermes, and existing global migration ownership remain unchanged.

The fixed canary does not fabricate a Project Registry receipt in order to
obtain a Policy allow. The repository has no completed durable Project
Registry live adapter at this checkpoint. Project-selectable, free-form, or
broader task templates remain unavailable until that owner can supply an
independently loaded live current head and the normal Policy gates are
composed.

## Data, Privacy, And Security

The public Submit result may contain only schema/tool version, `task_ref`,
fixed intent, typed disposition, Task Spec digest, and command receipt digest.
The public Status result may additionally contain typed task state/terminal
disposition, ledger-head and observation-receipt digests, and bounded typed
verification/Git/downstream disposition when durably available.

Raw Task Spec bytes, prompt, diff, source content, command, path, SQL,
environment, credential/token/key, actor/session secret, lease/fencing token,
Codex thread/turn control, process output, child stderr, DSN, database schema,
and unbounded diagnostic text are prohibited from public projections.

The fixed actor/profile and idempotency decisions are retained only as
typed bounded audit commitments in PostgreSQL. No raw `clientInfo` or ChatGPT
conversation content is retained as authority.

## Compatibility And Migration

The two delivery tools retain their exact names, empty schemas, result/error
shape, and Phase 1 legacy/stateless compatibility on canonical `latticed`.
Discovery clients that refresh canonical `latticed` see the two additive task
tools. An old client that does not know them continues to use delivery tools
unchanged.

The alternate `lattice-full-chain` executable is not another writer entry. Its
legacy observer catalog contains only `lattice_delivery_run` and
`lattice_delivery_status`; Delivery Run returns the fixed
`LATTICE_DELIVERY_RUN_REQUIRES_CANONICAL_LATTICED` denial before service
dispatch, while Status remains read-only. Both task names are absent from
legacy/stateless discovery and return `Unknown tool` before argument parsing or
service dispatch.

The non-MCP `lattice-runtime delivery-run` command remains available only for
the exact visibly scripted repository fixture. Official Codex mode is rejected
before identity, database, workspace, or process effects so CLI-selected paths
cannot create a second official writer or false MCP ingress audit. Canonical
`latticed` is the sole official writer entry.

Writer Lease v1 is an independent explicit extension operation. Normal daemon
startup does not auto-install or repair it. Missing/partial/drifted/wrong-owner/
wrong-ACL state fails closed. Global migrations `0001` through `0004` and
Codebase Memory v2 extension bytes remain unchanged.

No current Phase 1 success claim is upgraded into Phase 2 or production-chain
acceptance merely by this specification amendment.

## Error Cases And Edge Cases

- Unknown intent, missing field, extra property, non-object input, malformed or
  oversized `client_request_id`/`task_ref`: MCP `-32602` before service
  dispatch.
- Unknown tool or reserved metadata downgrade: existing bounded protocol
  error; no service dispatch.
- Hostile `clientInfo` or caller identity/lease/fence fields: ignored as
  informational where protocol-required or rejected as tool properties; never
  authority.
- Same key and same request: identical accepted task/receipt; no repeated
  Codex effect.
- Same key and changed request: stable substitution denial without receipt
  disclosure or mutation.
- Another key for the already admitted fixed task: typed substitution denial
  and zero writer calls.
- Task Spec validation, database, current-head, lease,
  workspace, Codex, scope, verification, Git, or durable-outcome failure:
  fail closed at that stage and suppress later effects.
- Expired/suspect/stale/cross-spec/cross-fence/Fake/synthetic lease: no Codex or
  Git mutation.
- Unknown database/Codex/Git outcome: reconciliation required, never success.
- Missing/corrupt/incomplete status state: bounded not-found/corrupt/
  reconciliation result; never synthesized `Completed`.
- Fresh-process status must not create a task or rerun any external stage.

## Acceptance Criteria

### Phase 1 Preserved

- [x] Existing private tunnel discovery shows the two closed delivery tools.
- [x] Legacy and stateless MCP generations reach the same delivery binding.
- [x] Delivery tool arguments remain empty and caller properties fail before
      dispatch.

### Phase 2 Required — Not Yet Claimed

- [x] Canonical `latticed` discovery reports exactly four tools; legacy
      delivery schemas are byte-equivalent and both task schemas are closed.
- [x] Alternate `lattice-full-chain` is a delivery observer only: both task
      tools are absent/unknown in both MCP generations and Delivery Run is
      fixed-denied before service effects.
- [x] Every prohibited task field is rejected before Gateway service dispatch.
- [x] The server-owned fixed actor is used; hostile `clientInfo` or caller
      identity cannot change authorization/audit binding.
- [x] Submit constructs and validates the complete server-owned Task Spec 2.1.
- [x] The same Task Spec digest is proved across Gateway, Ledger,
      Writer Lease, Codex, verification/Git, and status.
- [x] Exact idempotency, different-key denial, and fixed-profile audit survive
      PostgreSQL restart.
- [x] PostgreSQL Writer Lease proves concurrent single-writer acquisition,
      monotonic non-reused fencing, current-head validation, heartbeat/release,
      stale-fence denial, and ambiguous-outcome reconciliation.
- [x] No Fake/synthetic writer authority exists in the canonical-local
      acceptance path.
- [x] A canonical-local MCP client submits one `CONTROLLED_CODEX_CANARY`;
      LATTICE admits it and only then dispatches the sole controlled Codex
      writer. This does not satisfy the separate real ChatGPT gate below.
- [x] Codex changes only the template-owned scope; fixed verification passes;
      one local Git commit is durably bound to the task/lease/fence.
- [x] Task Status returns only the public allowlisted projection.
- [x] A new process/new MCP session returns the identical durable terminal
       projection with zero repeated Codex/verification/Git effects and zero
       Graphify/Hermes/Memory effects throughout the Task capability run.
- [x] Repository governance, focused/full tests, strict changed-slice lint,
      format, and diff checks pass with current evidence.
- [ ] A refreshed real Secure MCP Tunnel / ChatGPT session discovers and invokes
      both task tools, and a separate new ChatGPT request/session reads the same
      durable result. Local canonical evidence cannot satisfy this item.
- [x] TASK-037 remains explicitly open until its later production verifier
      proves `Hermes -> Memory -> Status`; TASK-038 evidence does not imply it.

## Verification Plan

| Criterion | Verification method | Expected evidence |
|---|---|---|
| Four-tool closure | MCP unit and real-binary legacy/stateless discovery/call tests | exact names/schemas plus pre-dispatch rejection matrix |
| Fixed actor | hostile `clientInfo`/argument substitution tests | unchanged server actor/audit binding; tunnel/local commitments cannot substitute; zero unauthorized dispatch |
| Task Spec unity | Contracts, Task Domain, Orchestrator mutation matrices | one digest across every owner/stage; every substitution denied |
| Ledger truth | PostgreSQL task-ledger integration and restart tests | TaskCreated/transitions/idempotency/audit/status exact replay |
| Writer Lease | pure repository conformance plus PostgreSQL 17 live tests | concurrent single writer, monotonic fence, restart, stale denial, atomic checkpoint |
| Controlled writer | isolated repository real Codex canary | exact allowed change, fixed verification, one commit, lease-bound evidence |
| Fresh status | stop/start database and new canonical `latticed` process/session | identical terminal projection and zero Codex/verification/Git effect-footprint delta; downstream remains zero |
| Secret/status closure | schema/result/error/log/audit inspection | only allowlisted fields; no prohibited values |
| Repository regression | focused/workspace Rust tests, format, changed-slice Clippy, project checks, `git diff --check` | zero exit status, with any unchanged baseline failures recorded rather than relabeled |

The canonical-local acceptance harness is
`scripts/run-task038-task-submit.ps1`. It must install/verify the independent
Writer Lease extension explicitly, use a disposable bounded PostgreSQL 17
target and isolated Git fixture, capture redacted typed evidence, restart the
database and `latticed`, and verify zero repeated external effects.

## Human Decisions

On 2026-08-09 the user corrected execution order: implement and verify TASK-038
bounded GPT-to-LATTICE-to-Codex dispatch first, then resume TASK-037 production-
chain repair. The user selected the fixed tunnel/profile actor and the single
`CONTROLLED_CODEX_CANARY` template for this first slice. This authorizes the
versioned local implementation and its bounded PostgreSQL/Codex/Git acceptance;
it does not authorize public exposure, push, merge, deployment, release,
payment, account changes, secret disclosure, or broader task templates.

## Open Questions

None for the bounded Phase 2 implementation. Per-human identity, broader typed
templates, durable Project Registry / Policy composition, and task-multiplicity
or quota policy require later versioned decisions.
