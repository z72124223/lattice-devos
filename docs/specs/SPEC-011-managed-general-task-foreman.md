---
spec_id: SPEC-011
title: Durable managed general-task foreman closed loop
version: 2.0
status: approved
approved_by: delegated_product_owner
approved_at_local: 2026-08-28
modules:
  - module_id: foreman-state
    constitution_version: 1.6
  - module_id: task-ledger
    constitution_version: 3.2
  - module_id: artifact-store
    constitution_version: 1.1
  - module_id: approval-verifier
    constitution_version: 1.4
  - module_id: policy-engine
    constitution_version: 2.8
  - module_id: lattice-ports
    constitution_version: 2.6
  - module_id: orchestrator-runtime
    constitution_version: 3.1
  - module_id: codex-adapter
    constitution_version: 1.5
  - module_id: postgres-foreman
    constitution_version: 2.0
  - module_id: latticed
    constitution_version: 3.9
  - module_id: workspace-git
    constitution_version: 1.2
---

# SPEC-011 - Durable managed general-task foreman closed loop

Version 2.0 adopts ADR-029 only for Store-v8 deployment compatibility. It
does not change managed-task lineage, authorization, execution, review, or
protected-effect semantics.

## Problem

`GENERAL_TASK_INTAKE_V1` durably records a natural-language objective and a
registered project in `DRAFT`, but it deliberately grants no Task Spec,
approval, budget, Writer Lease, model, or execution authority. The product has
no formal path that promotes that intake, atomically claims dependency-ready
work, runs the exact Codex lifecycle, recovers a stalled attempt, independently
verifies the result, and replays the same evidence after restart.

## Authoritative lineage

- The existing general-intake stream remains immutable and create-only.
- One intake may be linked exactly once to one immutable `TASK_SPEC` successor
  stream. The link binds public `task_ref`, intake/event digest, Project
  Registry identity and receipt, complete Task Spec digest, approval-subject
  digest, budget digest, verification-policy digest, and successor stream ID.
- Before the first successor effect, one immutable promotion intent pins that
  exact lineage plus a verified clean base ref/commit. Restart reuses it and
  never re-samples mutable HEAD; zero successor is pre-admission, one exact
  successor is replayable, and any other matching-lineage shape fails closed.
- Foreman intake links use the retained Store-v7-owned unique submission-stream and
  global event-digest keys as separate foreign keys, then revalidate their
  exact pair on every write replay and read. Mixing one valid stream with a
  different valid event is corrupt lineage and must raise before successor or
  provider effects; no new Store constraint is introduced.
- Only the successor Task Spec stream owns executable Task Domain state.
  Worker-attempt child records never define a second task state machine.
- PostgreSQL Task Ledger and a same-database subordinate foreman extension own
  the durable truth. The extension stores typed child rows and live Approval
  Verifier/Artifact Store records, binds the exact current Store-v8 database
  and manifest identity after its append-only Store-v7 base rebind, and never
  stores a second `task_state`. Control catalog
  and UI rows are locators or observations only.
- The server-owned foreman identity remains `SoleForemanBinding`; an observed
  Codex UI task/thread ID is evidence and can never select product authority.
- A formal dispatch identity may be derived only from replay-verified Foreman
  Runtime status with a nonzero latest generation, exactly one `ACTIVE`
  snapshot, no `BLOCKED` or `COMPLETED` snapshot, and `next_action=CONTINUE`.
  Empty, terminal, ambiguous, or non-continuing replay fails closed before
  task claim or provider dispatch.

## Promotion and authorization

- Promotion captures a bounded Task Spec from the verified intake, current
  Project Registry snapshot, trusted repository rules, a closed verification
  profile, and server-owned budget defaults. Objective text is data and is
  never interpreted as a shell command, approval, cost authority, merge,
  deployment, release, payment, external-message, or deletion authority.
- The successor transitions `DRAFT -> AWAITING_EXECUTION_APPROVAL`. It may
  enter `PREPARING` only when the existing Policy/Approval path proves either
  a task/spec/budget-bound current approval or the closed policy's exact
  `not_required` result for a bounded reversible local profile.
- If authorization cannot be verified, status remains
  `AWAITING_EXECUTION_APPROVAL`, no Codex thread/turn is created, and the public
  projection exposes one bounded plain-language next action.
- Closed-policy authorization is the only product-enabled Phase-4 ingress for
  bounded reversible local execution. Verified responsible-user approval is
  fully owner-typed and replayable, but its PostgreSQL ingress is fail closed to
  the general Runtime role until a separately authenticated Approval-owner
  connector/role exists. Migrator-only persistence is acceptance evidence, not
  Runtime execution authority. A task that requires that unavailable lane stays
  `AWAITING_EXECUTION_APPROVAL`; Runtime cannot self-attest owner state.
- Execution authorization cannot satisfy merge, default-branch, push,
  deployment, publication, payment, external-message, or permanent-delete
  gates.

## Claim, routing, and budgets

- The foreman takes only dependency-ready successors. After validating its
  replay-verified sole-active generation/checkpoint, the current Task Ledger
  head, Writer Lease/fence, immutable retry budget, model availability, and
  attempt packet, it appends one worker-attempt intent event and durably
  reserves that exact event/payload without consuming capacity.
  Reservation replay is exact; changed event, payload, or maximum-attempt
  budget fails closed.
- A separate serialized PostgreSQL claim transaction atomically moves that
  exact reservation to the active-attempt row only when global and per-task
  capacity allow it. Capacity rejection preserves the reservation and restart
  discovery reports `CAPACITY_WAIT`; a crash before reservation is reported as
  promoted work with no attempt. Neither state is reported as a running worker.
- The global active-attempt limit is four. Default per-task concurrency is one.
  Default repair retries are two, so the default maximum attempt count is
  three. Time, token/model-call, external-cost, and deadline bounds are part of
  the immutable budget digest.
- Allowed models are exactly `gpt-5.6-luna`, `gpt-5.6-terra`, and
  `gpt-5.6-sol`. Terra is the default engineering model; Luna is limited to
  bounded state/evidence/document work; Sol requires P0, architecture,
  security, high-risk, or retained evidence that Terra was insufficient.
  Selection and reasoning-effort rationale are durable. No unavailable model
  may be silently substituted.
- An attempt packet binds task/project/spec/approval/budget/profile digests,
  worktree, base commit, model/reasoning, deadline, writer fence, prior
  terminal evidence, and a bounded continuation summary. The full prompt and
  secrets are not persisted.
- Before claim, the Workspace/Git owner creates or exact-replays one task-owned
  worktree under a configured absolute managed root. Codex and the verifier use
  that isolated checkout, never the registered source checkout. A path-free
  `GIT_SNAPSHOT` baseline binds ownership, canonical locators, base/tree,
  branch/HEAD, `.git` pointer, index, and Git control state. Its content digest
  is the attempt `worktree_ref`; it is persisted after claim and before
  `thread/start`. Restart/retry compares actual control state to that durable
  digest and cannot accept current dirty files as a new baseline.

## Exact Codex lifecycle

- An attempt selects one immutable execution environment. `NATIVE_WINDOWS` and
  `WSL2_LINUX` are distinct typed domains; retry and reconnect may not silently
  change domains. The attempt-packet digest binds the environment descriptor.
- A WSL2/Linux descriptor binds the Windows WSL gateway identity separately
  from the Linux Codex launcher path/version/digest, Linux `CODEX_HOME` and
  config digest, Linux Node/Git/helper identities, Linux repository/cwd/Git
  identity, and one canonical Windows-to-Linux locator mapping. `wsl.exe` is
  never accepted as the Codex launcher identity.
- Before attempt claim, the same-database Foreman owner persists one bounded,
  canonical typed execution-environment descriptor and its domain-separated
  digest. The attempt packet and worker-attempt row bind that exact ref. Fresh
  process, restart, retry, reconnect, Git, and verifier paths independently
  reload and recompute it; a changed descriptor/ref, path mapping, credential-
  authority summary, toolchain identity, or execution-domain digest fails
  closed before a provider or verifier effect.
- Provider execution, Git baseline/status/diff/commit, verification, interrupt,
  retry, and reconnect use the same execution domain. Cross-domain Git or path
  evidence fails closed rather than being normalized as equivalent.
- WSL2 provider processes run under a digest-pinned Linux-side subtree fence.
  The fence binds distro/boot identity, PID start time and process group (or an
  equivalent cgroup). No replacement or repair may start until an exact
  terminal or zero-member subtree receipt is verified.
- WSL2 authentication is isolated in the Linux `CODEX_HOME` and Linux keyring.
  Windows keyring credentials, Windows homes, plaintext `auth.json`, token
  environment variables, and ambient credential fallback are forbidden.
  Inventory, sandbox/write, Git, and `account/read(refresh=false)` readiness are
  zero-model preflights; only their exact passing receipt authorizes one bounded
  real provider attempt.

- Managed Codex uses one stable server-owned `CODEX_HOME` outside source and
  task worktrees. Its config bytes are exact, require
  `cli_auth_credentials_store = "keyring"`, and define the closed task-shell
  environment allowlist. Any `auth.json`, non-keyring config, home overlap, or
  config drift blocks before connector launch; LATTICE never reads the file.
- Codex owns credential enrollment and OS-keyring access. Before claim, the
  connector calls public `account/read` with refresh disabled and discards the
  raw response. The only admissible result is a sanitized ChatGPT-ready value
  bound to exact App Server generation and opaque Codex-home/config digests.
  It contains no account identity, token, provider data, prompt, or locator.
- Missing enrollment, non-ChatGPT auth, unavailable/tampered readiness,
  generation substitution, or digest mismatch is
  `CREDENTIAL_READ_ISOLATION_NOT_VERIFIED`. Fresh start and recovery bridges
  recheck readiness before any provider effect; no ambient-home, plaintext,
  environment-token, or silent auth fallback is allowed.
- `thread/start` and `turn/start` RPC acceptance produce only
  `ACCEPTED|STARTING`. Only the exact matching `turn/started` notification with
  `inProgress` permits `EXECUTING`.
- Each attempt durably records attempt number, thread ID, turn ID, model,
  reasoning, model reason, packet digest, accepted/start/progress/terminal
  times, Writer fence, exact App Server session/home/config identity digest,
  and normalized resource observations. Generation alone is never identity.
- Every observation payload/event and PostgreSQL child row binds that nonzero
  App Server identity digest. Within one attempt identity and generation remain
  fixed; only a durable `RECONCILED` observation may rotate the pair, after
  which subsequent observations must bind the replacement exactly.
- A `(task_ref, attempt)` has at most one Codex thread and turn. Restart first
  reads/resumes/reconciles the retained exact IDs. It cannot create another
  thread or turn while the prior attempt is non-terminal or uncertain.

## Progress, stall, and repair

- Meaningful progress is an exact lifecycle notification, terminal/process
  observation, or bounded verified work/evidence change; elapsed wall time by
  itself is not a stall.
- Closed stall reasons are: `HEARTBEAT_TIMEOUT_ACTIVE_TURN`,
  `PROCESS_EXIT_WITHOUT_TERMINAL`, `RECONCILIATION_EXHAUSTED`, and
  `DEADLINE_EXCEEDED`.
- Recovery is always read/resume/reconcile first. If interruption is required,
  the foreman interrupts only the exact active turn and waits for its exact
  `interrupted|failed` terminal before claiming attempt N+1.
- A retained pre-start ambiguity with no exact provider terminal retains its
  Writer authority and active capacity and forbids retry. The sole exception
  begins only after the PostgreSQL owner, under the same claim/dispatch/
  observation advisory lock, atomically binds the immutable original blocker
  to a distinct typed proof that no provider effect exists. Possible effect,
  substituted evidence, or any later new observation rejects closure.
- That owner closure is terminal-equivalent only for releasing the old
  attempt's capacity and admitting a budgeted repair predecessor. It never
  fabricates `turn/started`, a provider terminal, verification, completion, or
  Writer-release authority. Within budget the same task advances to N+1 with
  a higher Writer fence and the proof digest as prior evidence. Closure alone
  cannot release the Writer; release is permitted only after the separately
  governed durable blocked/failed or verified-success path.
- Repair attempts retain all earlier evidence, increment attempt and Writer
  fence, reuse the same `task_ref`, carry a bounded continuation packet, and do
  not repeat work already proven complete. Non-repairable failure or exhausted
  budget becomes `BLOCKED` or `FAILED` with one closed blocker and next action.

## Independent verification

- Agent text, exit zero, a commit, or a final UI message never completes a
  task. The foreman independently verifies the exact base/commit/tree,
  changed-path scope, trusted command identities and results, required tests,
  artifact digests, review outcome, absence of unauthorized external effects,
  and absence of a residual active turn/process.
- Verification commands are selected only by Task Spec, trusted captured
  project rules, or a closed server policy. No objective substring becomes a
  command.
- Verification failure may schedule a repair attempt only within budget and
  is never overwritten as success. A programming task that passes verification
  and review stops at `AWAITING_MERGE_APPROVAL`; it does not push, merge,
  deploy, or publish. A no-merge task may transition to `COMPLETED` only when
  its immutable spec says so.
- A passing independently reviewed candidate commit is retained through one
  exact task/attempt-owned local ref only after its durable verification row is
  recorded. Ref replay cannot overwrite another commit and grants no merge,
  push, checkout, deployment, or publication authority.

## Durable evidence and status

- PostgreSQL retains the promotion link, foreman generation/checkpoints,
  attempt lifecycle, exact Codex IDs, routing rationale, stalls/retries,
  verification identities/results, Git commit/tree/diff digests, review,
  normalized token/resource observations, cost status, terminal state,
  blocker, next action, and evidence chain.
- PostgreSQL also retains the independently queryable typed execution-
  environment descriptor/ref selected for each attempt. It contains only
  bounded environment identity: WSL distribution/version, canonical Linux
  paths, launcher/Node/npm/Git/supervisor/sandbox/Rust toolchain identities,
  credential-authority kind plus opaque digest, and execution-domain digest;
  it contains no secret, token, account identity, prompt, or raw output.
- For a retained pre-start no-effect closure, PostgreSQL retains the immutable
  original blocker, distinct exact proof, closure/fence, and retry lineage.
  Replay never substitutes that proof for a provider terminal.
- When that lineage reaches its immutable maximum attempt, Artifact Store
  retains a separate exact `RetryBudgetExhausted` decision bound to the
  original blocker and closure-proof or terminal evidence. It records the
  concrete `BLOCKED` status and next action without rewriting either original
  object; fresh replay must not infer exhaustion from generic Blocked state.
- Artifact Store retains immutable bounded evidence objects and provenance.
  Task Ledger events store their exact refs/digests. Unknown or tampered
  artifact bytes, child rows, event linkage, or digest chains fail closed.
- Multiple artifacts in one attempt use their exact Task Ledger event sequence
  as replay ordinal; duplicate, reordered, or substituted ordinal evidence
  fails closed.
- Artifact references remain limited to one MiB each and are atomically capped
  at 64 objects/eight MiB per attempt and 192 objects/24 MiB per task. Exact
  replay at a quota boundary succeeds; a new reference fails with a closed
  quota result and cannot add a Task Replay record.
- Persisted text is bounded and sanitized. Passwords, tokens, full prompts,
  raw sensitive output, and unredacted credential-bearing remote URLs are
  rejected. Monetary cost without a trusted quote is `UNAVAILABLE`, not zero.
- `lattice_task_status` returns a new versioned managed-task projection with
  Task Domain phase, real-running boolean, attempt, model, exact worker IDs,
  last meaningful progress, blocker, verification summary, evidence digest,
  resource observation, and next action. Fresh processes reproduce the same
  projection from PostgreSQL and Artifact Store; no dashboard row is trusted.
  An unpromoted intake also uses v4 with no worker/attempt and a read-only
  durable dirty/currentness/approval blocker plus one plain next action.
- The foreman extension is installed or verified only by explicit PostgreSQL
  bootstrap. It is not a new database or a new Task lifecycle: every child row
  binds a replay-verified Task Ledger event/head, Project Registry identity,
  Writer fence, and extension identity.

## Acceptance criteria

- [x] Promotion is exactly-once; changed linkage/spec/budget reuse fails.
- [x] Crash after intent, successor admission, promotion binding, or before
      authority replays one intent/promotion/successor and starts no Agent.
- [x] Sole-active Foreman Runtime preclaim gating, durable reservation, atomic
      pending-to-active claim, duplicate prevention, global four-worker
      capacity, per-task capacity, model allowlist/availability, immutable
      retry budgets, and capacity-wait restart discovery pass.
- [x] Exact-start, heartbeat/non-stall, closed stalls, reconcile-first,
      exact interrupt/terminal, retry, and restart-resume tests pass.
- [x] App Server session/home/config identity survives PostgreSQL replay;
      exact replay rejects substitution, non-reconciled identity/generation
      drift fails closed, and only `RECONCILED` can lock a replacement pair.
- [x] Retained pre-start closure proves exact no-effect under the shared lock,
      preserves blocker plus proof, rejects tamper/post-closure observation,
      releases capacity without inventing a terminal, and replays one bounded
      higher-fence repair after fresh restart.
- [x] Writer cleanup retains every ambiguous or repairable provider path;
      release after `BLOCKED`/`FAILED` requires the separately durable closed
      decision plus exact terminal/no-provider-effect evidence, and closure
      alone never authorizes release.
- [x] Approval, execution, merge, push, deploy, publish, payment, external
      message, and permanent-delete authorities remain structurally separate.
- [x] Verification pass/fail, scope drift, evidence tamper, review, residual
      process, and command-identity tests pass.
- [x] Registered-source/worker isolation, ignored-file separation, durable
      baseline restart/retry, index/control drift rejection, predispatch
      artifact ordering, and protected-result-ref exact replay pass.
- [x] Per-attempt and per-task artifact count/byte quota, concurrent insertion,
      exact replay at limit, and non-polluting closed rejection tests pass.
- [ ] A disposable PostgreSQL profile plus disposable Git repository completes
      one real Codex happy path through automatic claim, exact start, a small
      edit, independent verification, durable evidence, and
      `AWAITING_MERGE_APPROVAL` without a remote effect.
- [x] Scripted App Server tests reproducibly cover stall/retry/capacity and are
      marked `SCRIPTED`; they are not reported as the real Codex result.
- [x] Foreman, Control, and PostgreSQL restart on the same `task_ref` replays
      identical spec/attempt/thread/turn/evidence digests and creates no extra
      Agent.
- [ ] Exact execution-environment persistence, independent query, packet/ref
      binding, substitution rejection, and fresh-process restart/reconcile
      reconstruction pass against disposable PostgreSQL 17.
- [ ] `cargo fmt --check`, `cargo check`, workspace tests, `npm.cmd run verify`,
      focused/live acceptance, and independent code/architecture/security
      reviews pass before the local commit.

## Non-goals

- SaaS, accounts, billing, public exposure, UI polish, push, merge, deployment,
  release, or modification of the AI script repository.
- A second foreman database, Control-owned task truth, a second task-state
  machine, caller-selected commands/paths/models, or unlimited model work.
