# TASK-106 workflow ledger

## Request

- Classification: high-risk durable-state and Git-worktree integration
- Base/target: product `d248200` → `feature/task-106-dependency-continuation`
- Durable identity: LATTICE TASK-106, active checkpoint generation 3
- Dependency: TASK-105 product baseline `d248200`

## Stage status

| Stage | Status | Evidence | Gate |
|---|---|---|---|
| Live repository/runtime audit | valid | current Runtime call plus Git/remote/worktree inspection | machine-observed |
| Durable task identity | valid | PostgreSQL-retained TASK-106 checkpoint and fixed branch/worktree/base binding | machine-observed |
| Specification and module contracts | valid | SPEC-010, TASK-106 and five versioned module amendments | repository-enforced |
| TDD implementation | valid | closed blocker, replay, Git guard, CLI and MCP regression suites | machine-enforced tests |
| Full local verification | valid | Node/Rust suites, format, diff and scoped strict Clippy below | machine-enforced tests |
| Independent reviews | valid | final code and architecture review found no P0-P3 findings | independently verified |
| PostgreSQL fresh-process gate | valid | live run `a39cffb6dde741b8946e5aca2be61e34` below | machine-observed |
| Push/PR/CI/product merge | valid | PR #23, CI `verify`, merge commit `6dc1e303` and remote equality | service-verified |
| Deploy/install/runtime replay | valid | versioned artifact, Control receipt, fresh Codex reload/MCP and post-deploy PostgreSQL run | machine-observed |

## Evidence log

- Implementation commits are `2017a6b574c407286c16a855bc102334e1be53c1`
  and the Windows Node live-fixture correction
  `e45bb71f7cca758c95b6e76c9030603282355ced`.
- `npm.cmd run verify` passed: Control 17/17; main Node suite 117 pass,
  zero fail and one pre-existing platform skip. The main suite includes 13
  bounded Git-workspace cases and invokes the real dependency CLI.
- `cargo test -p lattice-runtime` passed: runtime library 134 pass, zero fail,
  two coordinated-live fixtures ignored; composition 22/22, MCP 37/37,
  coordination 1/1, dispatch 7/7, task-control 2/2 and retained integration
  tests all passed.
- Foreman 16/16 passed. Ports, PostgreSQL Store and Orchestrator complete test
  suites passed, including checkpoint ordering 7/7, Store library 45 pass with
  one coordinated-live fixture ignored, migration contract 43/43 and schema-v6
  5/5.
- `cargo fmt --check`, `git diff --check`, and strict Clippy for Foreman, Ports
  and PostgreSQL Store passed. Full Runtime strict Clippy is explicitly not a
  pass: Rust 1.97 reports the same 22 pre-existing diagnostics on product
  `d248200` and feature source, with zero TASK-106 symbol lines.
- Independent closure review first found and then verified closure of legacy
  blocker collision, Git hook/fsmonitor/optional-lock hardening, direct
  `BLOCKED` to `COMPLETED` rejection, and outer MCP evidence validation. Final
  code and architecture verdicts were GO with no P0-P3 findings. A follow-up
  review of `e45bb71` also remained GO with no findings.
- Marker-owned PostgreSQL 17 run `a39cffb6dde741b8946e5aca2be61e34`
  used dynamic loopback port `49241`. It passed bootstrap and migration
  classification, process-A checkpointing, fresh-process replay, corrupt and
  unavailable replay fail-closed cases, Writer contention, the actual Node CLI
  child-worktree creation, direct-completed rejection, exact retry without Git
  reprobe, safe dependency integration, resumed replay and dual-process race.
  Required markers were
  `TASK106_STAGE_DEPENDENCY_FRESH_PROCESS_REPLAY_PASS`,
  `TASK105_STAGE_FOREMAN_DUAL_PROCESS_RACE_PASS` and
  `TASK105_DURABLE_FOREMAN_LIVE_GATE=PASS`. Teardown proved
  `root_absent=True` and `listener_absent=True`.
- The earlier diagnostic run `724367c09576490db65c86c29389b9f1` failed before
  the Node CLI entered because Rust canonicalization supplied a Windows
  extended entry path that Node 24 resolved as `C:`. Its database and listener
  teardown passed. Commit `e45bb71` changed only the fixture path and the full
  fresh live run above then passed.
- GitHub PR #23 targeted only `product/lattice-control-mvp`; its head was the
  verified remote SHA `805e4cec6d263d12f3d1299e3a0f850fefaac083`.
  Workflow `verify` passed in 46 seconds. The repository has no branch
  protection, ruleset or required-review enforcement, so none is claimed;
  explicit user authorization covered this default-product merge. An isolated
  `GitWorkspace.verifyIntegration` run returned `can_integrate=true`,
  `outcome=clean`, zero conflicts and removed its owned verification tree.
  GitHub merged PR #23 as `6dc1e303ea9be878fbe082cbd90ce33a416dad83`;
  remote SHA and the clean product worktree then matched exactly.
- Release artifact
  `C:\Users\f7212\AppData\Local\LATTICE\build-cache\latticed-runtime-6dc1e303ea9b\release\latticed.exe`
  was built from the clean product merge with `--locked`. Its SHA-256 is
  `a7a1f74c6307a7b086ebc57727216336526da993c858e20622e35230de07b668`.
  Control recorded and replayed installation observation
  `5421ff01-e8d1-4ddc-93d4-815012ece89e`; receipt digest is
  `7e338c8d64b285f88e42d936319a7d63136eeb0d2ce3b7908857abd0200e67fa`.
- No Codex App restart occurred. A fresh Codex app-server used a one-process
  config override, successfully executed `config/mcpServer/reload`, and
  discovered exactly the seven bounded tools including the structured
  dependency blocker contract. A separately fresh `latticed` MCP process used
  the installed artifact and existing configured environment without printing
  credentials. `lattice_runtime_status` returned
  `lattice.foreman-runtime-projection/1.1`, generation 3, `replay_status=VERIFIED`,
  `next_action=CONTINUE`, and the same durable ledger/checkpoint digests as the
  pre-deploy Runtime. The persistent global MCP pointer was deliberately not
  rotated while four Codex writer locks were active; existing App sessions were
  neither stopped nor replaced.
- Post-deploy product run `b6d5f57b0d6a4d23be5499a132d482c5` used dynamic
  loopback port `50978` and again emitted
  `TASK106_STAGE_DEPENDENCY_FRESH_PROCESS_REPLAY_PASS`,
  `TASK105_STAGE_FOREMAN_DUAL_PROCESS_RACE_PASS`,
  `TASK105_TEARDOWN_OK root_absent=True listener_absent=True`, and
  `TASK105_DURABLE_FOREMAN_LIVE_GATE=PASS`.

TASK-106 is complete. Rollback, if later required, is a normal reviewed revert
of merge commit `6dc1e303`; reset, clean, force push and historical-worktree
mutation remain prohibited.
