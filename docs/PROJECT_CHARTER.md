# LATTICE DevOS Product Charter

## Identity

- Formal name: **LATTICE DevOS**
- Chinese name: **織網 AI 開發中樞**
- Direction: **Codex-first modular development tool**
- Repository: `lattice-devos`
- Human interface and agent loop: Codex App / App Server

## Mission

LATTICE keeps the small amount of durable project and work-item truth that is
specific to the user's development workflow. Codex owns conversation, agent
execution, tool calls, sandboxing, approvals, and thread lifecycle.

LATTICE does not need to reproduce capabilities already supplied by Codex. Its
first obligation is a usable, resumable path from a work item to a tested
result.

## Governing principle

> **Small core. Independent modules. Explicit integration.**

### Small core

The MVP contains only:

1. project identity;
2. work-item state and priority;
3. a reference to the Codex work that executes it;
4. a verified result or honest failure;
5. minimal PostgreSQL persistence for LATTICE-owned truth.

### Independent modules

Modules compile and test independently. Graphify and Codebase Memory are
optional tools. Hermes and advanced authority modules are deferred. OpenClaw
is retired. Missing optional/deferred modules cannot block the core.

### Explicit integration

Integration is required only across interfaces used by the current product
path. A module test plus a pairwise contract test is preferred over a permanent
all-component startup gate. One small end-to-end MVP smoke test may be added
after the core path exists; it must not reactivate the retired full chain.

## Component lifecycle

| Lifecycle | Components |
|---|---|
| Core | Rust core, Codex, minimal PostgreSQL project/work-item truth |
| Optional | Graphify, Codebase Memory |
| Deferred | Hermes, Guardian, Writer Lease, Approval Verifier, Artifact Store, durable foreman, workflow learning, full `latticed` composition |
| Retired | OpenClaw and mandatory TASK-037/038/051-style full-chain acceptance |

Retired means removed from active startup and release criteria, not erased from
Git history. Deferred modules require a concrete observed need before returning
to the MVP.

## Multi-agent development

Multi-agent development remains supported through Git worktrees and branches,
not through a custom mandatory runtime:

- each agent owns one isolated worktree and branch;
- one writer operates in each worktree;
- shared interfaces have an explicit owner and compatible version;
- agents run focused tests for their owned module;
- an integration branch combines reviewed module changes;
- unrelated module failures do not consume core capacity.

Writer Lease or durable foreman coordination may be reactivated when multiple
independent writers must coordinate the same mutable production state. They are
not required merely because several Git branches exist.

## Truth and safety

- Codex thread history is Codex-owned execution context.
- PostgreSQL stores only LATTICE project/work-item state and verified results.
- Git owns source history.
- Dashboard, Graphify output, Hermes memory, transcripts, and generated files
  are observations, never authority.
- Unknown credentials, destructive operations, public exposure, default-branch
  mutation, merge, deployment, and release remain denied without explicit
  authority.

## Acceptance policy

Quality thresholds are not lowered; the tested scope is reduced to what is
actually used.

- Core and changed-module focused tests are required.
- Pairwise tests are required when a real interface changes.
- Full workspace verification is explicit and non-default.
- Historical full-chain failure does not fail the new MVP.
- After two failed attempts at the same acceptance, record the limitation and
  simplify or replace the path instead of creating another bridge task.

## Compatibility

Existing modules, branches, tests, and research remain available for selective
reuse. No old branch must be retrofitted to current dashboard or ticket rules.
Reactivation is demand-driven: demonstrate a current product gap, identify the
smallest module that closes it, and pass that module's focused and pairwise
contracts before integration.
