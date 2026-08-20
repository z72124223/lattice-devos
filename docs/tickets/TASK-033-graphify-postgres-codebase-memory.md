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
  - module_id: postgres-codebase-memory
    constitution_version: 1.0
display_name_zh_tw: 任務 033：Graphify 與 PostgreSQL 程式碼記憶終態交付
display_purpose_zh_tw: 重新驗證已提交實作、補齊審查與遠端終態，讓依賴者只依賴可查證的 TASK-033。
status: in_progress
parallel_safe: false
depends_on: [TASK-021]
branch: feature/task-033-terminal-delivery
delivery_remote: origin
delivery_repository: github.com/z72124223/lattice-devos
delivery_push: authorized_non_force_feature_branch
delivery_archive: keep_open
implementation_checkpoint: 52389375cd7dde552ceec9319120d3659dd7bb2f
terminal_delivery_base: fd9561c2f488c30365135ab94b392f212fe68afc
allowed_paths:
  - AGENTS.md
  - docs/contracts/ENGINEERING_PROTOCOL_V1.md
  - docs/tickets/TASK-091-legacy-governance-bridge.md
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
  - crates/lattice-postgres-codebase-memory/**
  - db/extensions/codebase-memory/v1.sql
  - scripts/run-lattice-graph-memory.ps1
  - scripts/run-lattice-delivery.ps1
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
  - docs/modules/postgres-codebase-memory/**
  - docs/tickets/TASK-022-postgres-project-registry.md
  - docs/tickets/TASK-033-graphify-postgres-codebase-memory.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_033_2026-08-05.md
  - docs/reviews/CODE_REVIEW_TASK_033_2026-08-05.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_033_2026-08-05.md
  - docs/reviews/INTEGRATION_TASK_033_2026-08-05.md
---

## Objective

Materialize one exact tracked Git commit, run pinned Graphify v0.9.33 in
headless code-only mode, strictly normalize provenance-bound graph evidence,
persist candidate structural memory plus deterministic retrieval audit in the
single PostgreSQL truth, and replay exact status after restart through the
existing two zero-parameter `latticed` tools. Use the scripted TASK-032 fixture;
official Codex live stays `FAILED_DIAGNOSTIC` and is not retried.

The approved PostgreSQL substep is complete: `postgres-codebase-memory` owns
the independent same-database extension, while Postgres Store admits only its
exact V3+Memory catalog/ACL profile. Global migrations and the Project
Registry's reserved `0005`/schema-v4 authority remain unchanged.

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
- [x] The independent
  `db/extensions/codebase-memory/v1.sql` profile uses exact embedded bytes/hash,
  a Memory identity/extension ledger, explicit admin runner, six owned tables,
  fixed `SECURITY DEFINER` functions, and a V3+Memory catalog/ACL
  verifier. It preserves global v3 and the Registry-reserved global
  `0005`/schema-v4 profile, atomically persists complete analyses/records/
  retrieval audit, and replays exact project/commit/query evidence after
  PostgreSQL stop/start.
- [x] `latticed` still exposes exactly `lattice_delivery_run` and
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

## Terminal Delivery Provenance

- Source implementation checkpoint:
  `52389375cd7dde552ceec9319120d3659dd7bb2f` (`feat: compose PostgreSQL
  codebase memory`).
- Clean terminal-delivery base:
  `fd9561c2f488c30365135ab94b392f212fe68afc`; this is the exact committed
  HEAD of the protected dirty `feature/v2-rust-postgres-bootstrap` worktree,
  but none of that worktree's uncommitted paths are included here.
- The four review files listed in `allowed_paths` were absent from the base and
  from visible Git history. They are not historical evidence; this repair
  regenerates those exact artifacts from current verification on this clean
  candidate. Reviewer independence is recorded explicitly rather than inferred.
- `status` remains `in_progress` until the current non-live and disposable live
  gates, review pass, clean commit, and exact remote delivery evidence succeed.

## Human Gate

Pure Codebase Memory, Graphify adapter, ports, and orchestrator need no further
product-boundary gate. PostgreSQL extension implementation already retains its
approved versioned owning-module amendment. The 2026-08-21 TASK-033 terminal
delivery repair delegation authorizes only this feature branch's non-force
push, exact remote verification, and dashboard refresh after all gates pass.
Primary merge, publication/deployment/payment/protected promotion, official
Codex retry, and safety-posture changes remain excluded; keep this Codex task
open for the foreman's independent verification.
