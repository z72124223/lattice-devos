# LATTICE DevOS

LATTICE is a local-first development tool that keeps project/work-item truth
around Codex without rebuilding the Codex agent loop, chat interface, sandbox,
approval flow, or thread lifecycle.

## Current release

`v0.1.0-alpha.1` is a Windows x86_64 developer preview of the simplified
module boundary. It provides only deterministic `version` and read-only
`status` commands. It does **not** yet create, dispatch, persist, or resume work
items, so it must not be presented as the completed product.

```powershell
npm.cmd run verify
npm.cmd run test:core
npm.cmd run alpha:package
```

The package is written under `dist`. See
[`docs/releases/v0.1.0-alpha.1.md`](docs/releases/v0.1.0-alpha.1.md) for the
exact included capabilities and limitations.

## Small MVP

```text
Codex App / App Server
        |
        v
LATTICE project + work-item state
        |
        v
minimal PostgreSQL truth
```

The MVP proves one useful path: identify a project and work item, let Codex do
the work, record the current state and verified result, and resume later.

## Component tiers

| Tier | Components | Meaning |
|---|---|---|
| Core | Rust core, Codex, minimal PostgreSQL truth | Required for the small MVP |
| Optional | Graphify, Codebase Memory | Independently usable; never blocks the core |
| Deferred | Hermes, Guardian, full `latticed` composition | Retained until a concrete need justifies integration |
| Retired | OpenClaw, old mandatory full-chain startup | Preserved in Git history; not maintained or started by default |

Writer Lease, Approval Verifier, Artifact Store, the durable foreman, and
workflow-learning code remain research/advanced modules. They are not core
release prerequisites. A real multi-writer or production-control need may
reactivate them later.

## Modular verification

Each module owns its focused tests. Only components connected by the current
product path require pairwise integration tests. The old rule that every
component must start and pass in one run is retired.

```powershell
# Inspect the retained components and lifecycle tiers.
cargo run -p lattice-cli -- status

# Inspect the exact preview version.
cargo run -p lattice-cli -- version

# Test the default core set only.
npm.cmd run test:core

# Test one independently maintained module.
cargo test -p lattice-codebase-memory --locked

# Explicit, slower all-module regression; not an MVP release prerequisite.
npm.cmd run test:all-modules

# Repository tooling and Node regression tests.
npm.cmd run verify
```

The old full-chain programs are still available for research and historical
diagnostics, but their names make that status explicit:

```powershell
npm.cmd run legacy:full-chain:smoke
npm.cmd run legacy:full-chain:test
```

## Parallel development

The simplified design keeps multi-branch and multi-agent development:

- one worktree and branch per independent change;
- one writer per worktree;
- stable module contracts between branches;
- focused tests inside each module;
- pairwise integration only when two modules actually connect;
- a separate integration branch for reviewed combinations.

An optional or deferred module can fail without stopping unrelated core work.
Default-branch mutation, merge, deployment, release, destructive cleanup, and
public exposure still require explicit authority.

## Preserved work

No Rust crate or historical branch is deleted by this reset. TASK-037/038/051
full-chain evidence remains useful diagnostic history, but those old chains no
longer define product completion. The engineering map is a navigation aid, not
the authority for whether tested product behavior works.
