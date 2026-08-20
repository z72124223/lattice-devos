---
ticket_id: TASK-043
spec_id: SPEC-002
spec_version: 27
module_id: codebase-memory
constitution_version: 1.0
status: completed
parallel_safe: true
depends_on:
  - TASK-033
allowed_paths:
  - crates/lattice-codebase-memory/src/lib.rs
  - crates/lattice-codebase-memory/tests/**
  - crates/lattice-codebase-memory/benches/**
  - crates/lattice-codebase-memory/Cargo.toml
  - docs/tickets/TASK-043-codebase-memory-benchmark.md
branch: feature/task-043-codebase-memory-benchmark
base_commit: 845328dcc06d51c7554c93a09739a27ddd827941
---

## Objective

Create a repeatable, quantitative, fully offline pure-Rust retrieval benchmark
for the TASK-033 structural Codebase Memory contract. Measure Traditional
Chinese, mixed-language, Rust symbol/path, error-code, exact-filename,
irrelevant no-answer, deterministic tie/order/digest, and exact project,
snapshot, and changed-commit isolation. Wall-clock duration is diagnostic only
and is never the sole acceptance condition.

This is PLANS Step 11 pure Phase A only. It neither exercises nor claims live
PostgreSQL, Graphify, Hermes, runtime/MCP, network, model, or database behavior.

## Locked Fixture

One in-memory structural graph uses project `benchmark-project`, snapshot
`benchmark-snapshot-a`, commit `1111111111111111111111111111111111111111`,
retrieval limit `3`, and these benchmark IDs. Benchmark IDs are test-owned
selectors for the deterministic record IDs produced by the public contract;
they are not a second durable identity or truth source.

| Benchmark ID | Subject | Category | Exact source path |
|---|---|---|---|
| `zh_retrieval_flow` | `記憶體查詢流程` | `函式` | `src/memory/retrieval_zh.rs` |
| `mixed_retrieval_pipeline` | `記憶體 Retrieval Pipeline` | `module` | `src/memory/mixed_retrieval.rs` |
| `chinese_memory_only` | `記憶體` | `contrast` | `src/memory/chinese_only.rs` |
| `english_retrieval_only` | `retrieval` | `contrast` | `src/memory/english_only.rs` |
| `rust_symbol_plan_retrieval` | `plan_retrieval` | `function` | `crates/lattice-codebase-memory/src/lib.rs` |
| `error_e0425` | `E0425 unresolved name` | `compiler_error` | `tests/ui/e0425.rs` |
| `exact_filename_test` | `retrieval_prioritizes_exact_identifier` | `test` | `crates/lattice-codebase-memory/tests/normalization_and_retrieval.rs` |
| `stable_tie_alpha` | `stable tie alpha` | `benchmark` | `tests/fixtures/tie_alpha.rs` |
| `stable_tie_beta` | `stable tie beta` | `benchmark` | `tests/fixtures/tie_beta.rs` |
| `irrelevant_gateway` | `GatewayService` | `struct` | `src/gateway.rs` |

All sources are typed `TrackedSource` values with fixed synthetic SHA-256
digests. The fixture contains labels and paths only, never raw source text,
credentials, external state, or accepted/trusted memory.

## Locked Retrieval Matrix

Each positive row uses top-k `k=1` acceptance against the listed benchmark ID.
The request still binds limit `3`, so unexpected lower-ranked false positives
remain observable without changing the production contract.

| Case | Query | Expected ID at rank 1 | Required result |
|---|---|---|---|
| Traditional Chinese | `記憶體查詢 查詢流程` | `zh_retrieval_flow` | `RESULTS` |
| Mixed language | `retrieval 記憶體` | `mixed_retrieval_pipeline` | `RESULTS` |
| Rust symbol | `plan_retrieval` | `rust_symbol_plan_retrieval` | `RESULTS` |
| Rust path | `crates/lattice-codebase-memory/src/lib.rs` | `rust_symbol_plan_retrieval` | `RESULTS` |
| Error code | `E0425` | `error_e0425` | `RESULTS` |
| Exact filename | `normalization_and_retrieval.rs` | `exact_filename_test` | `RESULTS` |
| Irrelevant Chinese | `支付 發票` | none | `NO_ANSWER`, zero IDs |
| Irrelevant error | `HERMES_RUN_FAILED_QUOTA` | none | `NO_ANSWER`, zero IDs |

Quality acceptance is exact and deterministic: positive `Hit@1 = 6/6`, mean
reciprocal rank `MRR = 1.0`, and irrelevant-query specificity `2/2`. No
wall-clock threshold may substitute for these outcomes.

The mixed-language row is discriminative: English-only query `retrieval` must
rank `english_retrieval_only` first, Chinese-only query `記憶體` must rank
`chinese_memory_only` first, and only the combined query may rank
`mixed_retrieval_pipeline` first. Ignoring either language therefore cannot
pass the complete mixed-language acceptance.

## Locked Determinism And Isolation Matrix

- Query `stable tie` must return exactly the two tie records first, with equal
  scores and record IDs ordered lexicographically. Reversing raw node input and
  repeating retrieval 32 times must preserve normalized analysis, ordered
  record IDs, scores, disposition, and result-set digest byte-for-byte. The
  exact v1 golden result-set digest is
  `17ee9a2ff916ec56f4a742b7a2b67c4eef379bd11ee31e1e3b5e04d4a89c66eb`.
- A query bound to `benchmark-project` must be rejected against an otherwise
  equivalent `other-project` analysis.
- A query bound to `benchmark-snapshot-a` must be rejected against an otherwise
  equivalent `benchmark-snapshot-b` analysis.
- A query bound to commit `111...111` must be rejected against an otherwise
  equivalent changed commit `222...222`; the changed commit's analysis digest,
  record IDs, ordered results, and result-set digest must differ from the old
  snapshot-bound result.
- Cross-binding rejection produces no retrieval plan. These pure tests make no
  persistence-side-effect or live PostgreSQL claim.

## TDD And Minimal-Fix Rule

Add the matrix through the public `normalize_analysis` and `plan_retrieval`
contracts and record observable RED/GREEN evidence below. Add no dependency,
public caller-selected query, alternate index, or persistence path. The
existing algorithm/version, exact identifier/path/token precedence, stable
record-ID tie-break, and result digest contract remain unchanged. Any scoring
or ranking change would require an algorithm/contract version change under the
module constitution and is therefore outside this ticket's allowed paths.

## Acceptance Criteria

- [x] Locked quality matrix passes all 6 positive and 2 no-answer cases.
- [x] Stable tie/order/digest is identical across reversed input and 32 runs.
- [x] Project, snapshot, and changed-commit substitutions fail closed; changed
  commit produces distinct exact-bound identities and digests.
- [x] Existing normalization/retrieval regression tests remain green.
- [x] Focused crate tests, strict crate Clippy, format, workspace tests/Clippy,
  `npm.cmd run verify`, and `git diff --check` were run with exact evidence;
  workspace tests pass and workspace Clippy is blocked only by the separately
  completed TASK-042 Hermes baseline checkpoint `f9c916d`.
- [x] A separate read-only review finds no unresolved P0/P1/P2 issue.
- [x] Only allowed paths change; one clean local checkpoint commit is created.

## Workflow Ledger

| Stage | Status | Evidence |
|---|---|---|
| Repository inspection | valid | clean base `845328d`; AGENTS/PLANS/HANDOFF/SPEC-002/constitution/current tests read |
| Requirements and specification | valid | user-bounded TASK-043 plus SPEC-002 AC-11/12/22 and Step 11 pure Phase A |
| Module constitution | valid | Codebase Memory 1.0 pure deterministic exact-bound retrieval boundary |
| Branch/worktree | valid | dedicated branch/worktree from exact clean base; no upstream created |
| TDD implementation | valid | initial RED exposed unsupported two-character partial scoring; review removed the production change and relocked a v1-compatible discriminative matrix; final GREEN is benchmark-only |
| Focused verification | valid | crate 8/8, benchmark 3/3 with fixed quality/determinism/isolation metrics, strict crate Clippy, fmt, and diff checks pass |
| Full verification | partial | workspace tests and npm 44/44 pass; strict workspace Clippy reaches only 11 out-of-scope Hermes baseline errors fixed on TASK-042 `f9c916d`, which is intentionally not integrated here |
| Independent review | valid | first pass found P1 version drift and P2 weak mixed-language case; second read-only pass reports `No findings`, blocker none |
| Architecture review | skipped | final diff changes only crate-owned offline tests and this ticket; no production/public contract/dependency/data-owner/schema/I/O change remains |
| Integration/CI/merge | skipped | explicitly outside TASK-043; no push, merge, deploy, or release authorized |

## Evidence Log

- Baseline `cargo test -p lattice-codebase-memory --locked`: exit 0; 5 existing
  integration tests passed before TASK-043 changes.
- RED: `cargo test -p lattice-codebase-memory --test retrieval_benchmark
  --locked`: exit 1; isolation and determinism passed, while the proposed
  two-character Traditional Chinese partial query returned `NO_ANSWER`.
- Independent review rejected an unversioned scoring change (P1) and found the
  first mixed-language row non-discriminative (P2). The production change was
  withdrawn; the ticket/fixture were relocked to current v1 and gained Chinese-
  only plus English-only ablation controls.
- GREEN: `cargo test -p lattice-codebase-memory --test retrieval_benchmark
  --locked -- --nocapture`: exit 0; `Hit@1=6/6`, `MRR=1.0`, no-answer `2/2`,
  deterministic repetitions `32/32`, tie order `2/2`, project/snapshot/changed-
  commit isolation `1/1` each, and golden result digest `17ee9a2f...c66eb`.
- `cargo test -p lattice-codebase-memory --locked`: exit 0; 5 existing plus 3
  TASK-043 integration tests passed.
- `cargo clippy -p lattice-codebase-memory --all-targets --all-features
  --locked -- -D warnings`, `cargo fmt --all -- --check`, and
  `git diff --check`: exit 0 with `CARGO_BUILD_JOBS=2` and
  `RUST_TEST_THREADS=4` for the Rust checks.
- `npm.cmd run verify`: exit 0; project check passed and Node tests 44/44.
- Second independent read-only code/architecture re-review: `No findings`;
  original P1/P2 resolved, no integration blocker. Reviewer ran no tests.
- `cargo test --workspace --locked`: exit 0 in 345.4 seconds with
  `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=4`; TASK-043 tests were included.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D
  warnings`: exit 1 only in `lattice-hermes-adapter`, with 11 baseline errors
  (`too_many_lines`, `large_stack_arrays`, `needless_pass_by_value`,
  `match_same_arms`, and `unnecessary_semicolon`) across `broker.rs`, `lib.rs`,
  and `production.rs`. The separately completed TASK-042 commit
  `f9c916d7e45b35fb742b73046aeba785b4c8ecf8` changes those exact Hermes paths;
  it was not integrated or copied into TASK-043.
- No PostgreSQL or WSL process was started, and no live PostgreSQL acceptance
  is claimed.

## Non-Goals

- PostgreSQL persistence/schema/store changes, runtime/MCP composition,
  Graphify/Hermes/OpenClaw, caller-selected public query input, external
  network/model/database, performance-index selection, a second truth source,
  root Cargo/Cargo.lock, PLANS/HANDOFF, push, merge, deployment, or release.
