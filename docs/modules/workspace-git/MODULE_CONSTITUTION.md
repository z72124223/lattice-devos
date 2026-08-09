---
module_id: workspace-git
name: Workspace and Git
version: 2.0
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-09
---

## Mission

Safely create and inspect LATTICE-owned Git worktrees, produce normalized
Git/filesystem evidence, and analyze non-conflicting integration while consuming
current Project Registry identity and Writer Lease/fencing authority.

## Non-Goals

- Own, issue, refresh, revoke, or persist a writer lease or fencing token.
- Decide policy authorization or project lifecycle.
- Let an Integrator edit product code or resolve conflicts automatically.
- Reset, clean, stash, force-remove, or otherwise repair a user checkout.
- Treat a local guard file, worktree marker, branch, ref, or Git object as
  durable control-plane truth.

## Owned Data

- Ephemeral process-local worktree handles and defense-in-depth guard files
  created under the configured LATTICE task-workspace root.
- Argument-array Git requests and normalized Git/filesystem evidence.
- Merge-readiness analysis evidence; no product ref or merge authority.

Project Registry owns repository identity and canonical roots. Writer Lease
owns lease/fencing/daemon authority. PostgreSQL remains the only durable
control-plane truth, and the repository owns product code.

## Public Contracts

- Bind an accepted `RepositoryObservation` and current Project Registry
  receipt/head to an exact current Writer Lease receipt/head.
- Create one task worktree at an exact commit using only the registered root,
  the lease-bound worktree identity, and argument-array Git execution.
- Return stable normalized changed-path evidence for committed, staged,
  unstaged, untracked, ignored, delete, rename, type-change, and unmerged
  states, including physical path-kind evidence.
- Analyze a target/feature pair and return clean/conflict evidence without
  changing a checkout or resolving a conflict.
- Remove only an exact clean worktree represented by the current process-local
  owned handle and matching current Registry/Lease authority.

## Invariants

1. No worktree creation, inspection, integration analysis, or removal proceeds
   unless both independent owner heads exactly match their receipts, Registry
   lifecycle is `ACTIVE`, Writer Lease status/admission are `ACTIVE`, and all
   project/snapshot/worktree/fencing bindings agree.
2. Every Git child is invoked through an executable plus argument array; shell
   command strings are absent.
3. Worktrees remain outside the registered checkout and inside one canonical,
   non-link LATTICE task-workspace root.
4. A local guard is defense in depth only: it grants no authority, issues no
   token, and cannot replace or outlive owner currentness checks.
5. Hooks, fsmonitor, inherited Git configuration injection, includes, and
   external filter/merge/diff drivers fail closed before a product worktree
   effect.
6. Conflicts are evidence only; the module never chooses ours/theirs or edits
   a product file to resolve them.
7. Cleanup never uses force, reset, clean, stash, broad recursion, or an
   unverified path. Dirty, ambiguous, stale, or mismatched state requires
   reconciliation.

## Allowed Dependencies

- `lattice-project-registry` only for its immutable public
  `RepositoryObservation`; this is the Registry-owned canonical root and
  physical repository identity input, not mutable Registry access.
- `lattice-contracts` for neutral Project Registry and Writer Lease authority
  receipts/current heads.
- `lattice-cjson` and exact SHA-256 mechanics for module-owned evidence hashes.
- Rust standard-library filesystem, path, and process APIs.
- Test-only `lattice-project-registry` and `lattice-writer-lease` dependencies
  to obtain receipts/current heads from the formal owners in disposable
  repositories.

## Forbidden Dependencies

- Mutable Project Registry access or any Writer Lease implementation dependency
  in production code, a second lease/fencing counter, Policy decisions, Task Ledger writes,
  provider adapters, OpenClaw, credentials, network, or broad cleanup tools.
- Shell interpolation, automatic conflict resolution, `git reset`,
  `git clean`, forced worktree removal, or mutation of an unrelated checkout.

## Failure, Compatibility, And Migration

Version 2.0 replaces the V1 file-backed `ProjectLock` authority with formal
Registry and Writer Lease receipts/current heads. V1 Node code remains
characterization evidence only. Unknown owner/runtime/path/Git/configuration
state fails closed. A failure after an observed Git side effect is reported as
reconciliation-required and is never guessed safe or automatically repaired.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Authority contract tests | focused Rust mismatch/stale/suspect/fencing matrix | Engineering | yes |
| Argument-array Git tests | injected executor assertions and unsafe-config denial | Engineering | yes |
| Disposable repository tests | exact worktree/change/ignored/rename/link/cleanup fixtures | Engineering | yes |
| Conflict fail-closed | disposable merge-readiness fixtures; no checkout/ref mutation | Architecture review | yes |
| Compatibility and full checks | preserved Node verification plus Rust workspace format/Clippy/tests | Engineering | yes |

## Change Policy

Registry/Lease ownership, local-guard authority, command construction,
worktree containment, cleanup, conflict, or evidence semantics require a
versioned amendment, architecture/security review, and responsible-human
approval.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-07-29 | ADR-002 | Initial Node project lock and Git boundary | Current user task |
| 2.0 | 2026-08-09 | SPEC-002 v27; approved V2 amendment; TASK-026 | Move lease/fencing and project identity to their formal owners; retain only guarded Git/worktree evidence | Responsible user |
