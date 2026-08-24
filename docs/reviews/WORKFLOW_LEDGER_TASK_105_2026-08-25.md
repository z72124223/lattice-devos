# TASK-105 workflow ledger

## Request

- Classification: high-risk persistence/public-contract integration
- Base/target: product `387f556` → `feature/task-105-durable-foreman-runtime`
- Dependency: TASK-094 exact `1e4ac5d`, merged as `d116e423`

## Stage status

| Stage | Status | Evidence | Gate |
|---|---|---|---|
| Repository inspection | valid | live Git/worktree/remote audit; workflow audit 2026-08-25 | machine-observed |
| Requirements | valid | sole-foreman frozen delegation | documented-only |
| Specification/ADR | valid | SPEC-009 / ADR-027 | documented-only |
| Module governance | valid | TASK-105 versioned amendments through Store 1.19 / Writer adapter 1.8; repository check PASS | machine-enforced tests |
| Ticket/worktree | valid | TASK-105, clean isolated product-based worktree | machine-observed |
| TDD implementation | valid | checkpoint domain, effect ordering, MCP, bootstrap and live failures each drove bounded RED/GREEN corrections | machine-enforced tests |
| Focused verification | valid | domain/adapter/runtime/Control suites below; format/check PASS | machine-enforced tests |
| Full strict lint | failed | changed domain/adapter crates PASS; runtime/full command retains 29 runtime plus 21 Hermes baseline diagnostics | current negative evidence |
| Independent reviews | valid | final code/test and architecture reviews found no P0-P3 findings at accepted source | independently verified |
| CI/product merge/deploy | pending | parent-owned non-force push, PR/CI, product merge and post-deploy replay remain separate live gates | unverified |

## Evidence log

- Merge gate: `npm.cmd run check` PASS; focused Foreman/Ledger/Store/Writer tests
  PASS before merge commit `d116e423`.
- RED checkpoints exposed exact-generation gaps, replay identity drift,
  effect-order ambiguity, MCP schema/error drift, bootstrap ordering, Writer-v3
  procedure routing, Ledger finalizer event admission, one SQL token boundary,
  and PostgreSQL-invalid `{1,256}` quantifiers. Each correction has a focused
  regression and clean commit; no historical migration before `0007` changed.
- Focused GREEN: Foreman 11/11; Task Ledger 50/50; Ports 10/10; Orchestrator
  checkpoint effect ordering 7/7; Store lib 43 pass/1 coordinated-live ignored,
  migration 42/42 and schema-v6 5/5; Writer all-targets 16/16; runtime lib
  130 pass/2 coordinated-live ignored, MCP 35/35, dispatch 7/7, composition
  22/22, coordination 1/1, task-control 2/2; Control 17/17.
- `npm.cmd run check -- --scope TASK-105`, `cargo fmt --all -- --check`, and
  `git diff --check` PASS. Strict Clippy passes for Foreman, Task Ledger, Ports,
  Orchestrator, Store and Writer. The combined strict command is explicitly
  not a PASS: current Rust 1.97 reports 29 runtime and 21 Hermes diagnostics,
  including pre-existing whole-file/style debt outside this bounded slice.
- Accepted source is `f932432a5471d03eba869cec61c1b5f376ffc740`.
  Marker-owned PostgreSQL PASS run `0e5c2971d099499183ee1643fe291e3d`
  used dynamic port `59685`. It passed initialize/bootstrap/current retry,
  process-A checkpoint/exact retry/ID reuse/generation gap/blocked projection,
  fresh-process replay, coherent-future versus corrupt-history taxonomy across
  profile/runtime/Task Ledger paths, five legacy/fault upgrade cases and the
  dual-process Writer race. Teardown proved `root_absent=True` and
  `listener_absent=True`.
- Final Store full tests and strict Clippy passed; runtime library was 131 pass
  with two coordinated-live fixtures ignored; npm check, Rust formatting and
  diff check passed. Final independent code/test and architecture reviews had
  no P0-P3 findings.
- Ticket is `complete` with `delivery_archive: keep_open`. No feature push, PR,
  product merge, install or product service/database mutation had occurred at
  this evidence checkpoint; those remain separately verified parent gates.
