# LATTICE DevOS repository rules

Read `docs/contracts/ENGINEERING_PROTOCOL_V1.md` before editing. Classify the
actual risk first, then use the smallest workflow that can prove the result.

## Product boundary

- LATTICE is a local-first development tool. Its first obligation is a usable
  path from task submission to a tested, recoverable result.
- Codex owns the human interface and agent loop. LATTICE must not rebuild chat,
  thread lifecycle, sandbox, approval plumbing, or tool execution already
  supplied by Codex.
- Preserve **One Gateway. One Truth. One Writer.** PostgreSQL owns durable
  control truth and one governed Codex implementer owns product-code writes.
- Core, optional, deferred, and retired modules develop independently. Only the
  current core path is a release prerequisite; a real interface change adds a
  pairwise test rather than reviving the old mandatory full chain.
- Graphify, Hermes, dashboards, generated files, transcripts, memories, and
  filesystem locks are observations or caches, never independent authority.
- Unknown identity, authority, credential, destructive action, public
  exposure, default-branch mutation, deployment, and release remain denied.
- Preserve unrelated and uncommitted work. Never use destructive Git cleanup,
  force push, or broad deletion to make a check pass.

## Risk tiers

### Routine

Use for documentation, presentation, tests, local diagnostics, and bounded
single-module fixes without authority, persistence, security, or public API
changes.

- No new TASK, specification, module constitution, ADR, review file, PLANS
  marker, HANDOFF entry, dashboard refresh, or finisher is required.
- Inspect the diff and run focused checks for the changed behavior.

### Standard

Use for ordinary product behavior and refactors within an existing ownership
boundary.

- Reuse an existing task only when the work is already tracked; do not create a
  task merely to satisfy tooling.
- Run focused checks plus affected integration tests. A separate review file is
  optional; review the final diff in the current work record.
- Run the full workspace only when the change can affect the full workspace.

### High risk

Use for authorization, credentials, process containment, durable schemas or
migrations, writer/lease/fencing logic, destructive operations, public APIs,
live external components, default-branch integration, deployment, and release.

- Require an explicit bounded scope, applicable ownership contract, focused
  negative tests, and independent review when the risk warrants it.
- Use disposable resources for live verification and prove ownership/cleanup.
- Merge, default-branch changes, deployment, release, public exposure, account
  or credential changes, and irreversible actions require explicit authority.

## Complexity circuit breaker

- Do not create a new task only to repair a dashboard, ticket, validator,
  finisher, review, handoff, branch label, or other governance artifact.
- A product task may have at most one governance repair. A second governance
  blocker must be simplified, downgraded to a known limitation, or removed.
- Governance dependencies may not form chains. Do not create bridge tasks to
  make another bridge or historical branch satisfy newer metadata.
- A dashboard projection is a navigation aid. `UNKNOWN`, stale presentation,
  missing Chinese labels, or refresh failure does not make tested product
  behavior fail and must not automatically dispatch repair work.
- Historical branches keep their historical contract. Do not retrofit every
  old branch to the newest validator.
- If supporting evidence or helper code becomes larger than the product change,
  stop and simplify before continuing.
- OpenClaw and the mandatory full-chain startup are retired. Hermes is deferred;
  Graphify and Codebase Memory are optional and cannot block the core.
- After two failed attempts at the same acceptance, stop retrying. Record the
  current limitation and return to the shortest usable product path.

## Verification and delivery

- `npm.cmd run check` validates repository structure and syntax. It must be
  runnable on maintenance, active task, integration, and default branches.
- `npm.cmd run verify` is the available Node regression suite, not proof of live
  services or every Rust component.
- `npm.cmd run status:refresh` is optional diagnostics. The engineering map is
  never completion authority.
- `npm.cmd run delivery:finish` is opt-in for an explicitly authorized remote
  feature delivery. Ordinary local completion, checkpoints, and maintenance do
  not require it.
- The finisher remains strict when invoked: no default-branch push, force push,
  merge, deployment, release, credential use, or destructive cleanup.
- A TASK branch may be `in_progress` while checks run. Terminal ticket and
  delivery metadata are required by the finisher, not by ordinary verification.
- `PLANS.md` may contain zero or one current focus. It is not a global lock for
  parallel work.

## Working practice

1. Inspect `git status`, branch, HEAD, relevant code, and the smallest current
   requirement before editing.
2. For a module change, read its `MODULE_CONSTITUTION.md`; preserve unrelated
   work and stay within the requested product scope.
3. Add or update the smallest behavioral test when behavior changes.
4. Run focused verification, inspect the final diff, and report current
   evidence without inventing live, CI, review, merge, or release success.
5. Create a concise handoff only when work genuinely needs another session to
   resume. Do not append routine history to the root handoff or plan.
