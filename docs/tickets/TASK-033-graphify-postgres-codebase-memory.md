---
ticket_id: TASK-033
spec_id: SPEC-002
spec_version: 27
module_id: graphify-adapter
constitution_version: 1.1
additional_modules:
  - module_id: codebase-memory
    constitution_version: 1.0
  - module_id: lattice-contracts
    constitution_version: 1.11
  - module_id: lattice-ports
    constitution_version: 1.7
  - module_id: orchestrator-runtime
    constitution_version: 2.2
  - module_id: latticed
    constitution_version: 1.1
  - module_id: postgres-store
    constitution_version: 1.4
status: in-progress
parallel_safe: false
depends_on:
  - TASK-021
allowed_paths:
  - README.md
  - Cargo.toml
  - Cargo.lock
  - package.json
  - apps/lattice-runtime/**
  - crates/lattice-contracts/**
  - crates/lattice-ports/**
  - crates/lattice-orchestrator/**
  - crates/lattice-graphify-adapter/**
  - crates/lattice-codebase-memory/**
  - crates/lattice-postgres-store/**
  - db/extensions/codebase-memory/v1.sql
  - scripts/run-lattice-graph-memory.ps1
  - scripts/run-task019-postgres.ps1
  - PLANS.md
  - HANDOFF.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-020-durable-postgres-project-registry.md
  - docs/adr/ADR-022-exact-graphify-postgres-codebase-memory.md
  - docs/modules/graphify-adapter/**
  - docs/modules/codebase-memory/**
  - docs/modules/lattice-contracts/**
  - docs/modules/lattice-ports/**
  - docs/modules/orchestrator-runtime/**
  - docs/modules/latticed/**
  - docs/modules/postgres-store/**
  - docs/tickets/TASK-022-postgres-project-registry.md
  - docs/tickets/TASK-033-graphify-postgres-codebase-memory.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_033_2026-08-05.md
  - docs/reviews/CODE_REVIEW_TASK_033_2026-08-05.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_033_2026-08-05.md
  - docs/reviews/INTEGRATION_TASK_033_2026-08-05.md
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Materialize one exact tracked Git commit, run pinned Graphify v0.9.33 in
headless code-only mode, strictly normalize provenance-bound graph evidence,
persist candidate structural memory plus deterministic retrieval audit in the
single PostgreSQL truth, and replay exact status after restart through the
existing two zero-parameter `latticed` tools. Use the scripted TASK-032 fixture;
official Codex live stays `FAILED_DIAGNOSTIC` and is not retried.

The PostgreSQL substep is `BLOCKED_PENDING_VERSIONED_AMENDMENT`: HEAD's
Postgres Store 1.4 and ADR-020 retain the Project Registry's global
`0005`/schema-v4 authority. TASK-033 may continue through pure Codebase Memory,
Graphify adapter, ports, and orchestrator; it may not wire the proposed
same-database extension until its owning module constitution is explicitly
versioned and approved.

## Acceptance Criteria

- [x] Contracts/ports/pure orchestrator bind exact project/commit/tree/tracked
  manifest, Graphify/config/output digests, memory records and retrieval audit;
  order is snapshot -> Graphify -> validate -> persist -> retrieve, with zero
  later effects after failure or ambiguity.
- [x] Production Graphify is `graphifyy==0.9.33`, commit
  `4e7e6b1f7e0df10ed07d5f28f9189bbde42940f1`, Apache-2.0, wheel SHA-256
  `c32b5792c783a6e66b1100b35bc65df3538e3f69b9df45fb098c9634c1b8eb01`;
  only `extract <snapshot> --code-only --no-cluster --max-workers 1 --out
  <staging>` is reachable with provider env cleared and query logging disabled.
- [x] Production execution binds the complete dependency payload and fixed
  WSL/Python/bubblewrap identity; verified namespaces copy runtime/source into
  private tmpfs, validate exact bytes, enforce Landlock ABI 3 plus a direct
  truncate-denial probe, expose only private output writes, hide unbound host
  siblings, and have no network.
- [x] Controlled Git fixture proves exact commit binding, changed-source
  invalidation, untracked/secret exclusion and deterministic manifest/order.
- [x] Timeout, non-zero exit, missing/malformed/partial graph, foreign source,
  overflow and teardown ambiguity reject before durable mutation.
- [x] Pure Codebase Memory stores only `OBSERVATION/CANDIDATE`,
  `trusted_context=false` structural records; exact identifier/path/token
  relevance is deterministic and irrelevant queries return no answer.
- [ ] After the required amendment is approved, the independent
  `db/extensions/codebase-memory/v1.sql` profile uses exact embedded bytes/hash,
  a Memory identity/extension ledger, explicit admin runner, four domain
  tables, fixed `SECURITY DEFINER` functions, and a V3+Memory catalog/ACL
  verifier. It preserves global v3 and the Registry-reserved global
  `0005`/schema-v4 profile, atomically persists complete analyses/records/
  retrieval audit, and replays exact project/commit/query evidence after
  PostgreSQL stop/start.
- [ ] `latticed` still exposes exactly `lattice_delivery_run` and
  `lattice_delivery_status`, both closed zero-parameter schemas; no third tool
  or shell/SQL/path/query/credential/provider input exists.
- [x] Focused/full tests, strict format/Clippy, independent code/architecture
  review, HANDOFF and checkpoint commit pass without an official-live claim.

## Non-Goals

- Official Codex/sandbox retry or unelevated/no-sandbox switch.
- Graphify install/hooks/query/watch/global/postgres/backend, raw source or
  secrets in memory, trusted promotion, Hermes, OpenClaw, deployment, push,
  payment, release, TASK-022 completion, or unrelated work.

## Verification

- `cargo test -p lattice-contracts -p lattice-ports -p lattice-codebase-memory -p lattice-orchestrator`
- `cargo test -p lattice-graphify-adapter -p lattice-postgres-store -p lattice-runtime`
- `powershell -File scripts/run-lattice-graph-memory.ps1`
- `cargo fmt --check`; strict workspace Clippy; locked full Rust tests;
  `npm.cmd run verify`; allowlist/secret/diff checks.

## Human Gate

Pure Codebase Memory, Graphify adapter, ports, and orchestrator need no further
gate. PostgreSQL extension implementation requires explicit approval of its
versioned owning-module amendment. Push, primary merge, publication/deployment/
payment/protected promotion, official Codex retry, and safety-posture changes
remain excluded.
