# TASK-077 V3 architecture trigger assessment — 2026-08-20

- The change adds deterministic, dated presentation advice to the existing
  selected-work panel. It does not change snapshot schema 2.0, Git/ticket state
  precedence, data ownership, writer authority, authorization, process/network
  behavior, dependencies, migration, or hosting.
- Constitution 1.2 explicitly keeps model advice non-authoritative: it cannot
  run, download, purchase, or switch a model. Advice older than 30 days stops
  naming a model and requests a fresh inventory.
- Existing V2 dependency direction and rollback remain unchanged. The copied
  request is still untrusted text and cannot grant TASK, branch, push, merge,
  deployment, or release authority.
- Architecture review trigger: `NOT_TRIGGERED`; new ADR/migration: not required;
  constitution conflict: none; integration blocker: none.

---

# TASK-077 V2 architecture review — 2026-08-20

## Triggers

- Changes the disposable public snapshot from
  `lattice.engineering-status/1.0` to `2.0`.
- Adds Git-ancestry hierarchy, a repository-owned Chinese purpose guide,
  fail-closed new-work eligibility, and a copy-only interaction.
- Amends the module constitution from 1.0 to 1.1 and the delivery protocol to
  1.0.3.
- Materially changes reliability, privacy presentation, and authorization
  wording while retaining the same read-only process/network boundary.

## Before and after

V1 rendered independent technical cards. V2 reads the same registered
worktrees, tickets, live remote heads, and optional GitHub metadata, then adds a
versioned, disposable projection of commit ancestry and Chinese branch purpose.
The page may classify a branch as a candidate starting point and prepare text
for Codex, but it cannot create a TASK, branch, worktree, commit, push, approval,
or other side effect.

## Ownership, contracts, and dependency direction

```text
launcher / npm delivery hook
  -> engineering-status-dashboard
       -> Node.js standard library
       -> Git read-only worktree, remote, and ancestry commands
       -> optional gh read-only query
       -> repository tickets and Chinese guide as read-only input
       -> disposable local HTML/JSON output
```

- ADR-001 remains intact: the Task Ledger and existing evidence remain truth;
  eligibility is labeled a presentation decision and grants no authority.
- ADR-002 and ADR-009 remain intact: selection never supplies an approval,
  lease, merge fact, or write permission; missing, stale, dirty, partial,
  unsynchronized, unmapped, or malformed evidence denies.
- ADR-004 remains intact: no safety-critical control-plane behavior moves out
  of Rust. This bounded Node module remains a local presentation adapter.
- ADR-006 remains intact: the page does not own a writable Codex process. Its
  copied request explicitly asks Codex to create a separate TASK and branch.
- No third-party package, database, service, public host, mutable shared state,
  or reverse dependency was added.

## Failure, compatibility, and rollback

- Schema 2.0 is an intentional versioned replacement for disposable snapshots;
  no persistent migration or dual read is required.
- Complete V2 structure is validated before atomic output replacement. A failed
  generation leaves the previous pair untouched.
- Git ancestry failure, an expired/untrusted timestamp, or incomplete evidence
  remains visible and disables all new-work choices.
- Commit ancestry proves version containment, not the historical moment a Git
  branch name was created. The UI states this limitation in plain Chinese.
- Rollback is a feature revert plus optional deletion of disposable local
  output; no LATTICE, database, credential, GitHub, deployment, or release state
  needs migration.

## Governance and risks

- SPEC-004 v2 and constitution 1.1 were approved by the user's direct acceptance
  correction before implementation; no silent constitution change occurred.
- Protocol 1.0.3 makes every future current branch provide a scoped Chinese
  name/purpose before refresh. The validator checks the live branch and guide.
- The guide is intentionally human-readable repository data and may require a
  normal task edit when purpose changes. Unmapped branches remain visible and
  unselectable.
- Path redaction remains heuristic for uncommon mount roots, but the generated
  artifact is local-only and common current-machine Windows/UNC/Unix forms are
  covered by preserved regressions.

## Decision

- Confirmed architecture violations: none.
- Constitution conflict: none; the approved 1.1 amendment matches V2.
- Migration or ADR required: none.
- Hidden/cyclic dependency or second truth/writer: none.
- Integration blocker: `PASS`, blocker-free.
