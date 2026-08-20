---
module_id: engineering-status-dashboard
name: Engineering Status Dashboard
constitution_version: 1.2
status: active
---

# Engineering Status Dashboard module constitution

## Mission

Project current repository evidence into one local, static, plain-language
engineering status page that a non-engineer can refresh and understand quickly.

## Non-Goals

- Own or mutate task truth, Git state, GitHub state, LATTICE state, runtime
  processes, credentials, deployment, release, or authorization.
- Replace the ticket, acceptance receipt, test, review, CI, or human gate.
- Add a server, database, UI framework, package dependency, or public host.

## Owned Data

- A generated local `status.json` snapshot and self-contained `index.html`.
- Presentation-only classifications derived from evidence, including explicit
  `UNKNOWN` and `STALE` states.
- The Git-ancestry presentation tree, Chinese purpose guide, and fail-closed
  new-work eligibility projection.
- Presentation-only Codex model-tier and reasoning-effort advice derived from
  the user's proposed work description and a dated, current surface inventory.

The default output directory is outside Git worktrees. Generated data is a
disposable projection, not authoritative repository state.

## Public Contracts

- `node scripts/export-lattice-engineering-status.mjs [options]`
- `npm.cmd run status:refresh`
- `npm.cmd run status:open`
- `Open-LATTICE-Engineering-Status.cmd`
- Snapshot schema `lattice.engineering-status/2.0`.

The exporter must return a nonzero exit code when it cannot identify the source
repository or cannot write a valid complete snapshot. Per-worktree and optional
GitHub failures are represented in the snapshot instead of silently omitted.

## Invariants

1. Collection is read-only with respect to Git, GitHub, tickets, and LATTICE.
2. Explicit task outcomes outrank inferred Git/CI presentation state.
3. Missing or conflicting evidence never becomes success.
4. Default display omits full local paths and secrets.
5. Repository-sourced text is rendered as text, never executable markup.
6. Each snapshot includes generation time, source repository identity, schema
   version, completeness, and source-level errors.
7. A failed refresh leaves any previous snapshot untouched by writing temporary
   files first and replacing only complete artifacts.
8. New-work eligibility requires explicit purpose, complete terminal evidence,
   a clean tree, and a verified synchronized remote, except that the verified
   GitHub default branch may act as the stable root.
9. Selecting or copying a new-work request never creates a task, branch, commit,
   push, or authorization.
10. Tree parents come from Git commit ancestry; branch names and TASK numbers do
    not invent parentage.
11. A snapshot older than 24 hours, implausibly future-dated, or missing its Git
    ancestry graph disables every new-work selection and recommendation.
12. The V2 writer validates the complete tree, Chinese display, eligibility,
    freshness, and recommendation structure before replacing prior output.
13. Model advice is non-authoritative: it recommends the smallest capable
    Codex tier, names when to escalate, never invokes a model or paid API, and
    never claims that the current task changed models.

## Allowed Dependencies

- Node.js standard library.
- Installed Git executable, read-only commands, and repository files.
- Optional installed GitHub CLI for one bounded read-only enrichment query.
- The operating system's default local-file opener when `--open` is requested.
- Dated Codex surface metadata used only to maintain static presentation advice.

No dependency on `lattice-cli`, PostgreSQL, MCP, Hermes, project runtime
processes, or third-party JavaScript packages is allowed.

## Forbidden Dependencies

- PostgreSQL, MCP, Hermes, `lattice-cli`, LATTICE task writers, and project
  runtime process control.
- Third-party JavaScript packages, analytics, public hosts, or browser storage
  used as an authority source.
- Git or GitHub mutation commands and credential/environment-value collection.
- Model execution, model download, paid API integration, or automatic model
  switching.

## Failure, Compatibility, And Migration

- A failed worktree source remains visible with `UNKNOWN` evidence and a short
  reason. Other worktrees continue.
- Unavailable GitHub evidence remains `UNKNOWN` and does not block generation.
- An invalid template, invalid snapshot, or output-write failure fails closed
  before replacing the previous complete output.
- The launcher keeps the terminal visible on fatal refresh failure and does not
  open a knowingly incomplete new page.

Snapshot version 2 is `lattice.engineering-status/2.0`; it adds the verified
default root, Git-ancestry tree, Chinese purpose fields, eligibility fields,
and the explicit snapshot freshness policy.
Generated snapshots are disposable and require no data migration.

## Acceptance Gates

- Repository validator: `npm.cmd run check` (no separate module-constitution
  validator exists in this repository).
- Focused tests: `node --test test/engineering-status-dashboard.test.js`.
- Full regression: `npm.cmd run verify`.
- Live local snapshot inspection and a local browser preview.
- Code/security, architecture, and integration review with no unresolved P0/P1.

## Change Policy

Changes to status precedence, snapshot schema, owned data, mutation authority,
network behavior, output privacy, or public hosting require a new specification
version and explicit review. Public deployment or any write capability also
requires separate user authorization.

## Amendment History

- 1.0 (2026-08-20): initial local read-only dashboard boundary for SPEC-004.
- 1.1 (2026-08-20): replace the card-first view with a Chinese Git-ancestry
  branch tree and fail-closed, copy-only new-work selection for SPEC-004 v2.
- 1.2 (2026-08-20): add dated, presentation-only Codex model and reasoning
  guidance while preserving the page's read-only and non-authoritative boundary.
