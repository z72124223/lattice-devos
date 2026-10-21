---
spec_id: SPEC-001
status: superseded_for_new_work
version: 1
superseded_by: SPEC-002
modules:
  - module_id: task-domain
    constitution_version: 1.0
  - module_id: task-ledger
    constitution_version: 1.0
  - module_id: policy-engine
    constitution_version: 1.0
  - module_id: workspace-git
    constitution_version: 1.0
  - module_id: scope-check
    constitution_version: 1.0
  - module_id: orchestrator-runtime
    constitution_version: 1.0
  - module_id: openclaw-adapter
    constitution_version: 1.0
---

# Controlled Swarm Core

> Historical V1 prototype specification. It remains immutable compatibility
> evidence, but it is not an active implementation specification after the
> 2026-07-29 direction change. New work follows blocked draft SPEC-002 only
> after its architecture and module amendments are approved.

## Problem

Multi-agent development can produce conflicting plans, concurrent code edits,
stale approvals, scope drift, and unverifiable claims. LATTICE needs an offline
control core that turns a request into a bounded, reviewable Task Packet and
proves that only one approved writer can advance it.

## Intended Behavior

### Task specification and packet

The core accepts a proposed immutable `TaskSpec`, validates it, derives its
`spec_hash`, and records it in the Task Ledger. The externally returned
`TaskPacket` combines that immutable spec with replayed status and evidence.
Mutable status is not included in `spec_hash`.

Required Task Spec fields:

- `schema_version`
- `task_id`
- `revision`
- `created_at`
- `created_by`
- `project_id`
- `base_ref`
- `base_commit_sha`
- `goal`
- `non_goals`
- `risk_class`
- `depends_on`
- `scope.allowed_paths`
- `scope.forbidden_paths`
- `scope.allowed_operations`
- `acceptance_criteria`
- `verification_commands`
- `required_checks`
- `requested_capabilities`
- `budget.max_agents`
- `budget.max_duration_seconds`
- `budget.max_attempts`
- `budget.max_model_calls`
- `budget.max_external_cost`
- `runtime_profile`
- `network_policy`
- `deployment_policy`
- `execution_approval_required`
- `merge_approval_required`

Phase 1 requires:

- `budget.max_agents <= 4`
- `budget.max_model_calls === 0`
- `budget.max_external_cost === 0`
- `runtime_profile === "fake"`
- `network_policy === "deny"`
- `deployment_policy === "deny"`
- both approval flags are `true`

### State machine

Main path:

```text
DRAFT
  -> AWAITING_EXECUTION_APPROVAL
  -> PREPARING
  -> EXECUTING
  -> VERIFYING
  -> REVIEWING
  -> AWAITING_MERGE_APPROVAL
  -> MERGING
  -> COMPLETED
```

Exception states:

- `REJECTED`
- `BLOCKED`
- `FAILED`
- `STOPPING`
- `CANCELLED`

Invalid transitions fail with a stable reason code and do not append a
transition event.

Transition evidence:

| Transition | Required evidence |
|---|---|
| draft to awaiting execution approval | valid schema, DAG, policy preflight, base commit and frozen `spec_hash` |
| awaiting approval to preparing | owner approval bound to `task_id + revision + spec_hash` |
| preparing to executing | worktree/base evidence and exclusive writer lease |
| executing to verifying | runtime stopped and writer lease revoked |
| verifying to reviewing | required checks and Scope Check pass |
| reviewing to awaiting merge approval | required reviewers pass and reviewed commit/diff hashes are frozen |
| awaiting merge approval to merging | owner approval bound to reviewed commit/diff hash |
| merging to completed | non-conflicting integration evidence is appended |

### Roles and permissions

- Unknown roles, actions, states, and capabilities default to deny.
- Planner, Code Mapper, Graphify, and all Reviewers are read-only.
- Only Implementer may perform create/modify/delete/rename on product paths, and
  only while `EXECUTING` with current approval and writer lease.
- Integrator may perform approved Git metadata integration but cannot edit a
  product file or resolve a merge conflict.
- At most four worker agents may be active concurrently.
- Phase 1 denies real-model calls, external network, deployment, purchase,
  credential changes, secret acquisition, public publication, and access to the
  playmate website.

### Approval records

Execution and merge approvals are separate records:

- `approval_id`
- `kind`
- `task_id`
- `task_revision`
- `subject_hash`
- `approver_id`
- `authority`
- `issued_at`
- `expires_at`
- `nonce`
- `channel`

The injected verifier must return authenticated owner evidence. Missing,
expired, wrong-kind, wrong-task, wrong-revision, wrong-subject, or replayed
approval fails closed.

### Task Ledger

Every accepted command, policy decision, state transition, approval, lease
acquire/revoke, runtime start/stop, Git action, scope/test/review result, and
integration result is append-only evidence.

Each event includes:

- `event_id`
- `task_id`
- `sequence`
- `timestamp`
- `command_id`
- `correlation_id`
- `actor_id`
- `role`
- `action`
- `outcome`
- `reason_code`
- `subject_hash`
- sanitized `payload`
- `previous_hash`
- `hash`

The ledger must:

- require `expected_sequence`;
- return the existing command receipt for the same `command_id`;
- reject a reused command ID with different content;
- detect changed, reordered, or truncated hash-chain content;
- replay to the same projection after restart;
- redact keys matching token, secret, password, credential, authorization, and
  API-key patterns;
- fail before side effects when intent evidence cannot be appended.

### Writer lock and fencing

The repository/project lock:

- is acquired atomically;
- permits one active writer;
- binds project, task, revision, spec hash, attempt, worktree, lease ID, and
  fencing token;
- rejects a second writer;
- does not auto-break an unknown/stale lock;
- rejects writes with an old fencing token;
- is revoked before verification/review begins.

### Git workspace

The Git adapter uses argument arrays, never shell interpolation. It:

- verifies the repository and exact base commit;
- derives a safe task branch;
- creates an isolated worktree;
- reports tracked, staged, unstaged, untracked, deleted, renamed, and type
  changes;
- cleans up only its known disposable/test worktree;
- refuses conflicts instead of choosing ours/theirs;
- is integration-tested only with a disposable temporary repository.

### Scope Check

Allowed paths are repository-relative canonical patterns. The checker:

- rejects absolute, empty, traversal, `.git/**`, symlink, junction, and escaped
  paths;
- checks both sides of a rename;
- evaluates changed operations against allowed operations;
- returns a stable-sorted violation manifest with rule/evidence hashes;
- never changes the filesystem or task state.

Its result is a detection gate only.

### Fake Runtime and orchestration

The Fake Runtime implements the same port expected of a future real runtime and
uses an injected clock. It makes no network, model, credential, Hostinger,
OpenClaw, or user-project call.

It can deterministically simulate:

- success;
- non-zero exit;
- timeout/hang;
- cancellation;
- malformed output;
- out-of-scope write;
- stale/second writer;
- build/test failure;
- reviewer rejection;
- integration conflict.

The Orchestrator is the only command entry and only Task Ledger appender. It
must stop at the first failed gate, revoke the writer before review, and never
reuse a stale approval.

### OpenClaw scaffold

The scaffold registers authenticated `/lattice` command metadata using the
current native plugin package shape. In Phase 1 it remains inert and returns a
clear message that no live orchestrator bridge, model, API, deployment, or
repository action occurred.

## User Stories Or System Scenarios

1. As the owner, I can inspect a plan whose task ID, risk, scope, non-goals, and
   acceptance evidence are frozen before execution.
2. As the owner, I know an old approval cannot authorize a revised plan.
3. As an operator, I can prove a second Implementer was denied.
4. As a reviewer, I can inspect evidence without obtaining write capability.
5. As an Integrator, I am blocked by conflicts instead of becoming a second
   code writer.
6. As an auditor, I can detect a changed event and replay the task state.
7. As a deployer, I receive an explicit Phase 3 gate instead of a false claim
   that static plugin files are live-compatible.

## Goals

- Deterministic, fail-closed local orchestration.
- Observable and testable approval and single-writer guarantees.
- One durable task/audit truth.
- Strict Phase 1 offline/cost/deployment boundaries.
- Current-format but inert OpenClaw scaffold.

## Non-Goals

- Real Codex/OpenAI/OpenClaw execution.
- Distributed locks or multi-host consistency.
- Hostile-process containment.
- Graphify, Hermes, embeddings, or long-term memory.
- Production database, web UI, Telegram integration, or deployment.
- Automatic merge to primary.

## Constraints

- Standard-library Node.js core.
- Cross-platform path logic; local Git proof runs on Windows in this task.
- No external runtime dependency installation for the core.
- No real user repository may be used in tests.
- No hidden fallback from fake to real runtime.

## Module Impact

All listed modules are new at version 1.0. Their public contracts, ownership,
dependency direction, and acceptance gates are defined in
`docs/modules/*/MODULE_CONSTITUTION.md`. No existing module is amended.

## Data, Privacy, And Security

- Task specs and audit payloads may contain repository metadata but must not
  store tokens, credentials, environment dumps, or raw secrets.
- Approval authority is injected and untrusted until verified.
- All unknown actions/capabilities default to deny.
- Path and Git inputs are validated before subprocess execution.
- No shell interpolation is permitted.
- The Phase 1 plugin must not load credentials or call a network.

## Compatibility And Migration

- Task schema begins at version `1.0`.
- Unknown schema versions fail closed.
- Ledger events include a version and are replayed deterministically.
- No automatic migration exists in Phase 1; a future version requires a
  versioned migration and ADR.
- The plugin does not claim an OpenClaw compatibility floor in Phase 1. Phase 3
  must pin the exact tested target range; live compatibility remains unverified
  until then.

## Error Cases And Edge Cases

- Duplicate task/command/approval.
- Cyclic dependency DAG.
- More than four agents.
- Stale, expired, replayed, or substituted approval.
- Dirty/mismatched base commit.
- Absolute/traversal/empty/renamed/symlinked paths.
- Second writer and old fencing token.
- Ledger sequence conflict, truncation, or hash corruption.
- Runtime failure, timeout, malformed result, or cancelled stop.
- Verification or reviewer failure.
- Merge conflict or changed reviewed diff.
- Audit append failure before/after an external side effect.

## Acceptance Criteria

- [ ] AC-01: Valid Phase 1 Task Spec produces a stable hash and
  `AWAITING_EXECUTION_APPROVAL`; invalid/unsafe specs are rejected.
- [ ] AC-02: Every allowed and forbidden state transition has deterministic
  tests and reason codes.
- [ ] AC-03: Permission matrix defaults unknown values to deny and proves only
  Implementer can write product code.
- [ ] AC-04: Execution and merge approvals are independently verified against
  their exact subjects; stale/replayed approvals fail.
- [ ] AC-05: A second writer and a stale fencing token fail; release is
  evidenced before review.
- [ ] AC-06: Scope Check covers canonical operations and fails every escaped,
  forbidden, or out-of-scope path without modifying it.
- [ ] AC-07: Ledger replay is deterministic, duplicate commands are idempotent,
  sequence conflicts fail, tampering is detected, and secrets are redacted.
- [ ] AC-08: Git adapter creates and inspects a worktree in a disposable
  repository without shell interpolation or user-repository access.
- [ ] AC-09: Fake Runtime covers success, failure, stop, writer, scope, review,
  and conflict scenarios with no real network/model/credential call.
- [ ] AC-10: End-to-end orchestration stops at each failed gate and reaches
  `COMPLETED` only after separately bound execution and merge approvals.
- [ ] AC-11: OpenClaw scaffold manifest/package/entry IDs agree, `/lattice`
  requires authentication, and the Phase 1 handler is inert.
- [ ] AC-12: `npm run verify` passes and the complete diff is within ticket
  scope.

## Verification Plan

| Criterion | Verification method | Expected evidence |
|---|---|---|
| AC-01 to AC-07 | focused Node test files | recorded RED then passing focused tests |
| AC-08 | disposable Git integration test | created/removable temp worktree; no external paths |
| AC-09 to AC-10 | deterministic Fake Runtime tests | call order, transitions, and denial reasons |
| AC-11 | static scaffold tests/check script | JSON and entry/command assertions |
| AC-12 | `npm run verify` and Git diff scope | exit code 0 and zero violations |

## Human Decisions

- Live OpenClaw load/command acceptance belongs to Phase 3 capability preflight.
- Real approval-channel identity and expiry policy belongs to deployment
  configuration and must be approved by the user.
- Primary-branch merge remains a user approval gate.

These deferred decisions do not authorize or block the offline Phase 1 core.

## Open Questions

None blocking the offline Phase 1 implementation. Recovery of the referenced
blueprint triggers a spec comparison and possible replan before further code.
