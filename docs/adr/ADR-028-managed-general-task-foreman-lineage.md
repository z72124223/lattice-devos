# ADR-028: Managed general-task foreman lineage

- Status: accepted for Phase 4
- Date: 2026-08-26
- Amended: 2026-08-31 by ADR-029
- Related: ADR-023, ADR-024, ADR-027, SPEC-009, SPEC-011

## Decision

Keep `GENERAL_TASK_INTAKE_V1` permanently create-only and add one immutable,
typed intake-to-TaskSpec successor linkage. The public intake `task_ref` is the
stable lineage key; only the linked TaskSpec stream may use Task Domain state,
Policy, Approval Verifier, Writer Lease, Codex, verification, review, or
completion transitions.

Per-task worker attempts are typed child evidence of Task Ledger events, not a
second task state machine. The existing server-owned sole-foreman stream keeps
global generation/checkpoint identity. PostgreSQL enforces unique promotion,
recoverable pending attempt reservations followed by atomic capacity claims,
exact thread/turn binding, monotonic attempt and fence values, and
child-row/event linkage. Capacity rejection leaves the exact Ledger-bound
reservation discoverable after restart but does not make it active. Artifact
Store owns bounded evidence bytes and provenance; Task Ledger owns their
authoritative refs and workflow order. PostgreSQL atomically caps subordinate
artifact references by per-attempt and per-task count and total bytes.

The composition root derives a formal managed-foreman identity only from a
replay-verified runtime projection whose latest generation is nonzero, whose
snapshot counts are exactly one `ACTIVE`, zero `BLOCKED`, and zero `COMPLETED`,
and whose next action is `CONTINUE`. Any empty, terminal, multi-active, or
otherwise non-continuing projection returns the fixed
`LATTICE_MANAGED_FOREMAN_NOT_ACTIVE` failure before task claim or provider
dispatch. UI task/thread observations never participate in that decision.

The exact Codex lifecycle is consumed through the already repaired Control
connector contract. A worker cannot become executing before its exact
`turn/started`, and restart must reconcile saved exact IDs before any new start.
Control SQLite remains a locator/UI projection and cannot authorize or complete
formal work.

Managed execution may select a first-class `WSL2_LINUX` domain. The immutable
attempt packet then binds a typed execution-environment identity whose subject
separates the Windows `wsl.exe` gateway from the Linux Codex launcher, Node,
Git, supervisor, keyring daemon, `CODEX_HOME`, repository, and typed UNC/Linux
path mapping. Production admission independently re-observes those identities
without a model call immediately before worktree or provider effects. Git,
verification, review, retry, reconnect, interrupt, and process-subtree receipts
must remain in that same domain; a native/WSL rotation requires a new attempt
identity and cannot repair or resume an existing attempt. Until all verification
tool identities are available inside the selected Linux domain, dispatch fails
closed and cannot be represented as live acceptance.

The subordinate same-database Foreman extension persists that exact canonical
descriptor and its domain-separated reference before attempt claim. The row is
independently queryable but grants no new authority: the Task Ledger attempt
packet remains the semantic binding, while PostgreSQL enforces exact replay and
recomputes descriptor identity. Fresh construction and every restart/reconcile
path must reload the same value. Descriptor/ref, canonical path, WSL version,
credential-authority summary, verifier toolchain, or execution-domain
substitution fails before provider or verifier effects. Only secret-free typed
identity is retained; credentials remain Codex/keyring-owned.

Codex remains the sole credential reader. Managed execution uses one stable,
server-owned `CODEX_HOME` outside worktrees with an exact keyring-only config;
the presence of `auth.json` or config drift is denied before connector launch.
LATTICE calls public App Server `account/read` only with refresh disabled and
reduces the result to a sanitized ChatGPT-ready observation bound to the exact
App Server generation, per-child opaque session identity, and opaque
Codex-home/config digests. Raw account, credential, provider, prompt, and
locator data never cross that projection. The connector repeats this exact
sanitized read immediately before every provider effect and fences the RPC on
the expected generation/session; drift occurs before provider dispatch and is
reconciliation-required. The owned marker/config identities remain sealed
against write/delete/link substitution for the whole child-process lifetime.
The task shell receives a closed non-secret environment allowlist and a
server-owned exact `PATH` whose admitted directories cannot contain Codex
launchers; credential homes, ambient `PATH`, and token/key/secret variables are
excluded. Only the outer absolute `LATTICE_CODEX_BIN` selects Codex. Missing
keyring enrollment or any readiness/session/digest/generation mismatch is the
typed pre-claim blocker
`CREDENTIAL_READ_ISOLATION_NOT_VERIFIED`; there is no plaintext or ambient-home
fallback.

Each worker observation stores the canonical digest of the exact App Server
session, Codex-home digest, and config digest alongside generation. That pair
is immutable for an attempt. Only an exact durable `RECONCILED` observation may
rotate it (including a fresh child whose generation restarts at one), and old
identity/generation pairs cannot append later progress or terminal evidence.

A retained pre-start provider ambiguity is not a terminal and initially keeps
the Writer and capacity fenced. The one accepted exception is an owner-atomic
attempt closure that preserves the immutable blocker and binds a separate
typed exact no-provider-effect Artifact. Closure shares the PostgreSQL advisory
lock with reservation/claim, provider dispatch, and observation append; it
rejects possible provider effect, substitution, and every new post-closure
observation. It is terminal-equivalent only for old-attempt capacity and
bounded retry-predecessor admission, never a Codex terminal, verification,
completion, merge, or Writer-release authority.

Approval Verifier owns an append-only execution-binding command and strict
snapshot/checkpoint replay for task/successor/spec/subject/budget-bound user
approval. The same-database extension can persist and tamper-test that owner
snapshot only through the migrator/owner boundary. The general Runtime role is
explicitly denied the verified-approval ingress and cannot self-attest an owner
snapshot or receipt. Until a separately authenticated Approval-owner connector
and database role are composed, production Phase 4 enables only the closed
Policy lane for bounded reversible local execution; a task requiring responsible-
user approval remains `AWAITING_EXECUTION_APPROVAL`.

The persistence addition remains an immutable same-database
`foreman-execution/v1` extension and does not own the global Store schema.
ADR-029 supersedes only its Store compatibility boundary: migrations 0001
through 0009 remain immutable, Store appends the reviewed v8 runtime successor,
and the extension preserves its Store-v7 installation evidence while appending
one exact Store-v8 rebind. It remains subordinate to Task Ledger state and
contains no authoritative task-state column. Normal serving never installs or
migrates it; only explicit PostgreSQL bootstrap installs, rebinds, or verifies
the extension profile.

## Consequences

- Existing phase-three task refs remain stable and become executable only after
  a unique successor link and valid execution policy evidence exist.
- A corrupt linkage, attempt row, Artifact Store object, approval head, Writer
  fence, or replay mismatch stops dispatch and status projection.
- A valid model alone is insufficient for claim: the same pre-claim probe must
  also return exact sanitized keyring-backed readiness. A fresh provider bridge
  rechecks readiness and connector generation/session fencing before any
  thread/read/start/resume/interrupt effect.
- A valid checkpoint digest or generation number alone is insufficient to
  dispatch: the replayed sole-foreman state must also be uniquely active and
  continuing at every identity load or restart reload.
- A crash between the Ledger attempt intent and capacity admission is
  recoverable: restart distinguishes promoted-no-attempt, capacity-wait,
  active-reconciliation, and terminal-pending-verification work without
  launching a duplicate attempt.
- A crash after retained no-effect closure replays the same blocker, proof,
  closure and Writer fence. Within budget, retry keeps the task/spec/approval/
  budget/worktree lineage, increments attempt and Writer fence, and anchors
  continuation to the proof. Without exact closure, ambiguity retains capacity
  and Writer authority; closure alone never authorizes Writer release.
- Verification success for programming work stops before merge. Local
  execution authorization cannot leak into protected external-effect gates.
- The implementation must add physical PostgreSQL Approval-owner snapshot and
  Artifact Store adapters rather than using deterministic fakes in live gates.
  Approval snapshot acceptance runs through the controlled migrator/owner
  boundary; enabling responsible-user approval for Runtime remains a separate
  connector/role activation gate and is fail closed in Phase 4.
- A future Store global-schema migration remains independent. Phase 4 does not
  require re-emitting Store, Task Ledger, Project Registry, Memory, or Writer
  functions merely to retain subordinate foreman evidence.
