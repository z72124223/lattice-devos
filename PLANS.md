# LATTICE DevOS v1.0 Plan

## Goal

Build a local, offline-first Phase 1 MVP for **LATTICE DevOS v1.0 — Controlled
Swarm** that enforces:

> One Gateway. One Truth. One Writer.

The MVP must make planning, execution approval, single-writer implementation,
verification, read-only review, merge approval, scope control, and audit
evidence observable and testable without using a real model, external API,
cloud deployment, Hostinger account, or the playmate website.

## Global Strategy

1. Preserve the user-provided request as the authoritative product boundary.
2. Define behavior, module constitutions, and dependency direction before code.
3. Use a dependency-light Node.js core with injected ports so all orchestration
   can be exercised through a deterministic Fake Runtime.
4. Use a hash-chained Task Ledger as the only durable control-plane truth;
   derive task status by replay instead of maintaining a second mutable task
   record.
5. Enforce the single writer twice: by role permissions and by an exclusive
   repository/project lock with a fencing token.
6. Treat execution and merge as separate, digest-bound human approvals.
7. Make integration fail closed on conflicts; the Integrator may mutate Git
   metadata but may never edit product code to resolve a conflict.
8. Treat Git-based Scope Check as a detection gate, not an operating-system
   sandbox or proof of hostile-process containment.
9. Keep the OpenClaw integration as a current-format scaffold with static local
   checks; live OpenClaw validation remains a later capability-preflight gate.
10. Implement one ticket at a time with recorded RED/GREEN evidence.

## Non-Goals

- Purchase, configure, or deploy Hostinger Managed OpenClaw.
- Authenticate OpenAI, Codex, Telegram, GitHub, or any other account.
- Use a real model, paid credit, API key, OAuth token, or external inference.
- Read or modify the playmate website repository.
- Add Graphify or Hermes to the Phase 1 execution path.
- Merge to the primary branch, publish a package, push a repository, or deploy.
- Claim that local scaffold checks prove Hostinger or OpenClaw runtime support.

## Scope

In scope:

- Task state machine and Task Packet JSON Schema.
- Event-sourced Task Ledger, deterministic replay, command idempotency, and
  optimistic sequence checks.
- Agent roles, capabilities, maximum-agent policy, and single-writer invariant.
- Policy Engine for approvals, protected actions, and scope checks.
- Exclusive project lock.
- Git branch/worktree adapter with an injectable command executor.
- Hash-chained append-only audit log with secret redaction.
- Deterministic Fake Runtime and end-to-end orchestration tests.
- Native OpenClaw plugin scaffold for the authenticated `/lattice` command.
- Local verification scripts, CI definition, documentation, and handoff.

Expected repository root:

`C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos`

## Confirmed Facts

- The user explicitly named the repository `lattice-devos`, the primary OpenClaw
  agent `lattice-pm`, and the command `/lattice`.
- The user explicitly limited v1 to at most four agents and exactly one
  code-writing Implementer; reviewers and planning/mapping roles are read-only.
- Execution, merge, and production deployment are separate approval boundaries.
- Phase 1 must include a task state machine, Task Packet Schema, agent
  permissions, Policy Engine, Git branch/worktree handling, Scope Check, Audit
  Log, Fake Runtime tests, and an OpenClaw plugin scaffold.
- Phase 1 must not use real model credit, modify the playmate website, sign in to
  Hostinger, or deploy to cloud.
- The supplied attachment directory contains only `pasted-text.txt`; the
  referenced 17-file ZIP and two linked Markdown deliverables are not present in
  that supplied directory.
- A read-only search of the supplied attachments, relevant Documents,
  Downloads, OneDrive locations, Git configs, and the contents of 17 discovered
  ZIP files found no LATTICE blueprint/archive or prior repository.
- The starting workspace was empty and was not a Git repository when audited on
  2026-07-29.
- Local commands currently available include Git 2.54.0, Node.js 24.16.0,
  npm 11.13.0, and pnpm 11.9.0.
- Current official OpenClaw documentation requires a native plugin to include
  `openclaw.plugin.json`, package entry metadata, and a runtime module; custom
  slash commands use `api.registerCommand(...)`.

## Assumptions

- Node.js ESM is the smallest coherent core implementation because the target
  OpenClaw plugin is a Node/TypeScript ESM package and the verified local
  runtime already satisfies OpenClaw's documented Node 24 floor.
- The core can avoid third-party runtime dependencies; the OpenClaw scaffold
  can declare OpenClaw only as a peer dependency and defer live package
  resolution to Phase 3.
- The pasted request contains enough Phase 1 behavior to proceed even if the
  referenced ZIP is unavailable. Any later recovered blueprint must be compared
  against this plan before code is changed.
- A local feature branch is sufficient for building this new repository. The
  product's own worktree behavior will be tested in disposable repositories.
- `max_agents` counts concurrently scheduled worker-agent roles; the
  deterministic Orchestrator is control-plane code, not an additional model
  agent.
- Phase 1 approval authentication is an injected verifier exercised with fake
  owner evidence. Telegram/OpenClaw channel authentication remains a later
  live preflight and cannot be inferred by the core.

## Open Questions

- The exact Hostinger runtime and installed OpenClaw version are intentionally
  unknown until the later capability preflight; live plugin validation is
  therefore not part of Phase 1 acceptance.

This open question does not change the explicit offline Phase 1 safety boundary.

## Acceptance Ownership

| Acceptance | Direct evidence Codex can produce | Final owner |
|---|---|---|
| State, approval, scope, lock, Git, and audit behavior | Automated local tests and command exit codes | Codex |
| OpenClaw scaffold file/manifest consistency | Static local checks against current official format | Codex |
| Live OpenClaw plugin loading and `/lattice` routing | Requires a managed/local OpenClaw runtime in Phase 3 | User capability-preflight gate |
| Hostinger purchase, account, OAuth, tokens | Not authorized in Phase 1 | User |
| Merge to primary branch | Requires explicit approval after evidence review | User |

## Implementation Steps

- [x] Step 1: Read the request, active workflow rules, official OpenClaw
  references, and audit the starting workspace.
- [x] Step 2: Create the workflow ledger, behavior specification,
  ADRs, module constitutions, and dependency-aware tickets.
- [x] Step 3: Initialize Git and establish the documented
  feature-branch plan.
- [x] Step 4: Implement Task Packet and deterministic state
  transitions using ticket-scoped RED/GREEN cycles.
- [x] Step 5: Implement the exclusive project lock and Git
  worktree/integration adapter in TASK-004, including disposable-repository
  validation.
- [ ] **CURRENT — Step 6:** Implement detection-only Scope Check in TASK-005.
- [ ] Step 7: Implement the deterministic Orchestrator/Fake Runtime vertical
  slice in TASK-006 using the completed Task Ledger.
- [ ] Step 8: Add and statically verify the OpenClaw `/lattice` plugin scaffold.
- [ ] Step 9: Run focused and full verification, independent code review,
  architecture review, and integration-readiness checks.
- [ ] Step 10: Write `HANDOFF.md` with the complete workflow ledger and exact
  human gates.

## Verification

- Every behavior ticket must record a failing focused test before its
  implementation and a passing focused test after it.
- `npm run check` must validate syntax, JSON artifacts, manifest/package
  alignment, module constitutions, and repository scope.
- `npm test` must exercise state transitions, stale/wrong approvals, protected
  actions, traversal and out-of-scope paths, concurrent lock attempts, audit
  tampering/redaction, Git worktree commands, reviewer read-only behavior,
  maximum-agent policy, stop/failure handling, and the full Fake Runtime flow.
- `npm run verify` must run all local checks and tests with exit code 0.
- The complete Git diff must stay within the active tickets' `allowed_paths`.
- Independent review must report findings before integration readiness is
  classified.

## Risks

- The missing ZIP may contain additional requirements not present in the pasted
  request. Mitigation: preserve this source boundary and require a blueprint
  comparison before adopting later material.
- A static OpenClaw scaffold can drift from the installed managed version.
  Mitigation: pin/document the verified official contract and keep live
  validation as a separate fail-closed preflight.
- Path matching and Git command construction are security-sensitive.
  Mitigation: reject traversal/absolute paths, avoid shell interpolation, and
  test disposable repositories.
- A passing Git Scope Check cannot prevent a hostile child process from writing
  elsewhere. Mitigation: label it detection-only and keep OS sandbox/process
  containment as a required Real Runtime gate.
- A role table alone cannot guarantee one writer.
  Mitigation: combine permission checks with an exclusive filesystem lock and
  audit every acquire/release attempt.
- Local CI files are documented automation until a remote repository actually
  runs them; branch protection and required reviews will remain missing.

## Drift Log

- 2026-07-29: The requested linked ZIP and Markdown files were not present in
  the supplied attachment directory. Planning proceeds from the actual pasted
  request and records live OpenClaw/Hostinger verification as a later gate.
- 2026-07-29: Current official OpenClaw plugin documentation was checked because
  the external plugin contract is version-sensitive. No OpenClaw installation
  or authentication was performed.
- 2026-07-29: Read-only architecture review selected an event-sourced Task
  Ledger as the single truth and made merge conflicts fail closed. This
  tightened, rather than changed, the original One Truth/One Writer direction.
- 2026-07-29: Removed duplicated Git work from Steps 5/6 after TASK-003. Step 5
  now maps exactly to TASK-004, Step 6 to TASK-005, and Step 7 to TASK-006.
- 2026-07-29: TASK-004 review probes exposed Windows junction, lock
  initialization, Git hook/driver, ownership-marker, and failure-cleanup gaps.
  The plan did not change; fail-closed tests and implementation were tightened
  before unlocking TASK-005.
