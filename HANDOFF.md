# TASK-033 Terminal Delivery Repair Handoff — 2026-08-21

## Status

`NEEDS_REVIEW` / `BLOCKED` before live execution.

## Objective And Scope

- Repair TASK-033 itself on `feature/task-033-terminal-delivery`, preserving
  implementation checkpoint `52389375cd7dde552ceec9319120d3659dd7bb2f`
  and clean base `fd9561c2f488c30365135ab94b392f212fe68afc`.
- Protect the dirty `feature/v2-rust-postgres-bootstrap` worktree. Its ten
  uncommitted paths remain unchanged and excluded.
- Do not create a TASK-090 ticket, touch TASK-051, use system PostgreSQL 5432,
  terminate foreign processes, merge, deploy, release, or archive.

## Completed Work

- Confirmed `feature/task-033-terminal-delivery` was unoccupied locally and
  remotely, then used `git branch -m` and `git worktree move` to register the
  existing clean isolated candidate under the legal TASK-033 identity.
- Updated the one existing TASK-033 ticket with branch, delivery policies,
  Traditional-Chinese display metadata, and exact implementation/base
  provenance. It remains `in_progress`.
- Regenerated the four exact ticket-allowed review artifacts from current
  evidence. They state that independent reviewer identity is not proven.
- Passed both focused Rust groups, format, strict workspace Clippy, locked full
  workspace tests, repository `npm.cmd run verify` (44/44 Node tests), and
  current diff/allowlist/secret checks.
- Integrated exact TASK-096 delivery `5c13a9232ffc59afed4a172ccd860a47c899e196`
  through the authorized history-preserving merge
  `d5f238908e9d909a3954241a63623175d66e3478` (parents
  `ef1c3741a862493a7edeea815ef5a7a101aecfcd` and TASK-096). The reviewed
  change preserves a closed, receipt-equality-checked terminal-cause envelope
  for the existing CLI/MCP boundary; it neither adds a tool nor changes live
  component behavior. TASK-095 is deliberately absent from this product line.
- The combined candidate passed offline `cargo test -p lattice-runtime`
  (56 tests), `cargo fmt --check`, workspace strict Clippy,
  `npm.cmd run check`, `npm.cmd run verify` (48/48 Node tests), merge-range
  `git diff --check`, and Gitleaks. The separate read-only code and
  architecture review found no P0--P3; reviewer independence remains
  unproven.

## Exact Blocker

The exact external c6 validator
`c6be912fae8e261527a07324833d68632aba8f3e` exits 1 against this branch only
because TASK-033 is deliberately not terminal:

```text
docs/tickets/TASK-033-graphify-postgres-codebase-memory.md: parallel ticket must be terminal.
```

This is the expected staged terminal blocker. No contract, branch metadata, or
allowed-path error remains. It does not authorize changing TASK-033 to
`completed` before fresh live acceptance.

## Verification And Runtime State

- Non-live validation: PASS, including the TASK-096 combined-candidate checks
  recorded above.
- External c6 validator: only the expected TASK-033 terminal-status blocker.
- Graphify/PostgreSQL combined live after the ABI-3 repair: `NOT_RUN`.
- TASK-033 live run root/port/marker: not allocated; TASK-033 live processes:
  none launched. System PostgreSQL 5432 and TASK-079 resources remain untouched.
- This handoff is a pre-push checkpoint. It does not run a completion finisher
  and does not claim terminal remote delivery or dashboard readiness.

## Exact Next Action

Keep TASK-033 `in_progress`/`BLOCKED`. After this checkpoint's authorized
non-force feature push and non-terminal dashboard refresh, the foreman must
assign a new, independent acceptance worker. That worker must take a fresh
resource-conflict snapshot and obtain explicit authorization before one
disposable PostgreSQL/Graphify restart/replay attempt; this worktree must not
self-accept or start that live run.

---

# Archived TASK-033 Implementation Handoff

## Status And Alignment

`DONE` for the bounded TASK-033 Graphify -> same-database PostgreSQL Codebase
Memory -> database restart -> exact query/receipt replay -> `latticed`
composition checkpoint. Hermes and OpenClaw were not started. TASK-032 official
Codex live remains `FAILED_DIAGNOSTIC`; no official live retry, sandbox setup,
unelevated/no-sandbox mode, deployment, push, or merge was attempted.

## Combined Production Checkpoint — 2026-08-05

- `lattice_delivery_run` now executes the pinned Graphify adapter against the
  exact committed scripted fixture, binds the durable delivery receipt to
  project/TASK/commit, and persists only through `PostgresCodebaseMemory` fixed
  functions. `lattice_delivery_status` reconstructs the same typed request and
  calls PostgreSQL receipt replay through a fresh runtime-role connection; it
  does not invoke Graphify.
- `powershell.exe -NoProfile -ExecutionPolicy Bypass -File
  .\scripts\run-lattice-delivery.ps1` exited 0 in 254 seconds. It emitted
  `TASK019_POSTGRES_HARNESS=PASS` and `LATTICE_DELIVERY_HARNESS=PASS` on
  PostgreSQL 17.10 at an ephemeral non-5432 endpoint.
- The combined run proved Store+Memory initial/restart, exact V3+Memory
  catalog/ACL admission, real non-empty Graphify persistence/retrieval, a
  second PostgreSQL stop/start, exact run/status graph-field equality, and an
  unchanged snapshot/staging footprint during fresh-process status. The final
  graph receipt digest was
  `b118e01deeab76c65c2a19a2dfb44c2f67723c7b39652fa2414cdaaeaa021a88`.
- Focused production verification passed: runtime all-target tests (30 unit,
  7 composition, 5 dispatch, 11 MCP), strict runtime Clippy, format, and the
  combined live gate. Broader non-P0/P1 review and redundant full matrices are
  intentionally deferred to the integration window.

## PostgreSQL Memory Checkpoint — 2026-08-05

- Added the independent `postgres-codebase-memory` crate and exact embedded
  `db/extensions/codebase-memory/v1.sql` profile: six owned tables, identity
  ledger, explicit admin runner, and three fixed `SECURITY DEFINER` functions.
- Added typed database/extension identity to durable contracts, validated
  restart replay constructors, and the production `PostgresCodebaseMemory`
  adapter. It accepts no SQL, path, schema, DSN, credential, or MCP input.
- RED evidence: the adapter and three replay constructors were absent and the
  focused tests failed to compile. GREEN evidence: adapter API 1/1, normalized
  contract replay 5/5, package all-target tests, and strict focused Clippy pass.
- Live evidence:
  `powershell.exe -NoProfile -ExecutionPolicy Bypass -File
  .\scripts\run-task019-postgres.ps1 -MemoryOnly` exited 0 in 19.1 s on
  PostgreSQL 17.10 at an ephemeral non-5432 endpoint. It proved partial and
  collision rejection, transactional rollback, exact install/no-op,
  catalog/ACL verification, runtime direct-table denial, real analysis and
  retrieval persistence, exact retries, changed-request rejection, stop/start,
  and fresh-process exact receipt replay.
- The full Store restart matrix and `lattice_delivery_status` composition now
  pass in the combined checkpoint above.

## Delivered

- Added typed graph-memory requests/evidence, ports, pure Codebase Memory
  normalization/retrieval, and pure orchestrator ordering with fail-closed
  zero-later-effect behavior.
- Added an exact tracked-Git snapshot materializer that excludes untracked and
  secret-shaped paths, binds the commit/tree/manifest, and rejects drift.
- Pinned `graphifyy` 0.9.33, upstream commit
  `4e7e6b1f7e0df10ed07d5f28f9189bbde42940f1`, Apache-2.0, wheel SHA-256
  `c32b5792c783a6e66b1100b35bc65df3538e3f69b9df45fb098c9634c1b8eb01`,
  and the complete reviewed dependency payload.
- Added the Windows/WSL Graphify adapter: fixed headless arguments, cleared
  provider environment, private tmpfs runtime/source/output, exact copy
  verification, strict seven-field framed copy-out, exclusive parent capture
  handles, bounded teardown, and Landlock ABI 3 truncate enforcement before
  any Graphify command.
- Preserved Store/ADR-020 Project Registry ownership and the global migration
  bytes/manifest. The independent Memory extension and runtime composition add
  no third MCP tool; Hermes and OpenClaw remain absent from this checkpoint.

## Material Files

| Files | Reason |
|---|---|
| `crates/lattice-contracts`, `crates/lattice-ports` | typed graph, memory, analysis, persistence, and retrieval boundaries |
| `crates/lattice-codebase-memory` | deterministic untrusted structural record normalization and retrieval |
| `crates/lattice-orchestrator` | pure snapshot -> analyze -> validate -> persist -> retrieve effect order |
| `crates/lattice-graphify-adapter` | exact Git snapshot plus pinned, contained, fail-closed Graphify execution |
| `crates/lattice-postgres-codebase-memory`, `db/extensions/codebase-memory/v1.sql` | sole same-database Memory persistence owner and exact extension bytes |
| `apps/lattice-runtime`, `scripts/run-lattice-delivery.ps1`, `scripts/run-task019-postgres.ps1` | production run/status composition and restart acceptance |
| `Cargo.toml`, `Cargo.lock` | register the new Rust crates and locked dependencies |
| SPEC-002, ADR-022, affected constitutions, TASK-033, `PLANS.md` | approved boundary and completed database/composition checkpoint |
| `HANDOFF.md`, `docs/workflow/WORKFLOW_LEDGER.md` | durable checkpoint truth and continuation evidence |

## Verification And Review

- Private typed Graphify live: exit 0, 1 passed/0 failed, test time 112.92 s
  (115.8 s wall); exact Git fixture produced deterministic typed evidence. This
  run proved the private-copy/framed-capture revision before the final ABI-3
  gate was added; the full Graphify live was not rerun after that review repair.
- Final containment repair: embedded runner SHA-256
  `98d0411709927a5687315f64efc6673a77f2241e2db6df8bd17c34886e3c2ad9`;
  execution identity
  `f270004749c7f4fc260dfc09925b52f3b7071bcc64ba5f7cbd9bd37ae1400dd5`.
  A bounded bubblewrap tmpfs probe exited 0 with
  `abi=7 truncate_denied=1 allowed_write=1`; no target Windows or WSL process
  remained.
- `cargo test -p lattice-graphify-adapter --lib --tests --locked`: exit 0;
  18 unit tests passed/2 ignored, 2 Git tests passed/2 ignored, and one static
  containment test passed/3 ignored.
- `cargo fmt --all -- --check`, strict adapter Clippy, locked full workspace
  tests, strict full workspace Clippy, and `npm.cmd run verify`: exit 0; Node
  tests 44/44. The first full run observed one pre-existing scripted Codex EOF
  timing mismatch; its exact test and the complete workspace rerun both passed.
- Three independent final read-only reviews: code P0=0/P1=0, security
  P0=0/P1=0, architecture P0=0/P1=0. Graphify Adapter 1.1 now matches the
  user-directed private tmpfs/Landlock/framed-capture implementation.

## Remaining Boundary

- The two declined-cleanup diagnostic directories still exist and are
  non-blocking; cleanup was not retried:
  `C:\Users\f7212\AppData\Local\Temp\lattice-graphify-live-typed-ports-7788-1`
  and
  `C:\Users\f7212\AppData\Local\Temp\lattice-graphify-live-typed-ports-16628-1`.
- The next executable node is a separately bounded Hermes candidate/reflection
  integration; it receives no authority from this checkpoint and was not
  started here.
- Branch: `feature/v2-rust-postgres-bootstrap`; checkpoint parent:
  `79096b6b5f184a47d44bbbd20a575bad79a5e393`; checkpoint is this commit. The
  repository has no remote, so remote synchronization, CI, branch protection,
  push, and merge remain unavailable/not performed.

---

# LATTICE DevOS TASK-032 Executable Delivery Handoff

## Status

`NEEDS_REVIEW` for TASK-032 completion. The trusted scripted delivery node is
implemented and fully verified, but official Codex live remains
`FAILED_DIAGNOSTIC`; TASK-032 and MVP-1 must remain open.

## Objective And Alignment

This checkpoint implements the approved bounded chain:

> fixed delivery intent -> Rust `latticed` -> PostgreSQL -> Codex app-server
> protocol -> isolated workspace -> one fixed test -> local Git commit ->
> PostgreSQL outcome/receipt -> restart status replay

It remains a general autonomous AI development platform. No playmate website,
deployment, publication, payment, public exposure, or protected release work
was added. OpenClaw, Graphify, Hermes, and Codebase Memory were not started
because the official-Codex prerequisite is still blocked.

## Completed And Confirmed

- Recorded the user-approved Contracts 1.10, Ports 1.6, pure Orchestrator 2.1,
  `latticed` 1.0, two-tool MCP, compatibility, and allowlist amendment in the
  existing spec, ADR, module constitutions, ticket, and plan.
- Added typed delivery request/evidence/receipt contracts and ports. The pure
  orchestrator depends only on `lattice-contracts` and `lattice-ports` and
  orders intent -> workspace -> Codex -> scope -> fixed test -> Git -> outcome
  -> receipt, stopping on every known or ambiguous failure.
- Added the canonical `latticed` composition root, concrete PostgreSQL,
  workspace/test/Git, and Codex adapters, plus the compatibility command that
  delegates to the same composition.
- Exposed exactly `lattice_delivery_run` and `lattice_delivery_status` over
  bounded MCP stdio. Both tools have closed zero-argument schemas; arbitrary
  shell, SQL, path, credential, provider, and task inputs are absent.
- Added typed PostgreSQL v2 restart reconstruction, complete non-secret
  configuration binding, post-mutation ambiguity handling, shared delivery
  deadlines, one fixed verification before commit, and safe empty `.agents`
  handling.
- Closed the final review blocker by checking the absolute deadline after Git
  child completion/output reads and before commit evidence returns. After a
  durable intent, every outcome-persistence failure now remains
  reconciliation-required, including ambiguous commit plus known DB timeout.
- Bound `SCRIPTED_ACCEPTANCE` to the checked-in fixture bytes, canonical paths,
  launcher/server hashes, exact wrapper, and marker before any database or
  process effect. A self-consistent tampered server cannot claim that mode.
- Added PowerShell and Rust fail-closed incident gates that reject official mode
  before build, database, or child-process effects.

## Material Files Changed

| Files | Reason |
|---|---|
| `Cargo.lock`, `apps/lattice-runtime/Cargo.toml`, `crates/lattice-codex-adapter/Cargo.toml` | lock and declare the concrete composition dependencies |
| `crates/lattice-ports/src/lib.rs` | add typed delivery effect ports |
| `crates/lattice-orchestrator/src/lib.rs`, `crates/lattice-orchestrator/tests/delivery_order.rs` | pure effect ordering and failure/ambiguity regressions |
| `crates/lattice-codex-adapter/src/{lib,delivery,process}.rs`, adapter tests | bounded identity/protocol/process adapter with one absolute deadline |
| `apps/lattice-runtime/src/{lib,composition,delivery_ledger,git_delivery,mcp}.rs` | canonical composition, durable replay, fixed test/Git adapter, and two-tool MCP |
| `apps/lattice-runtime/src/bin/latticed.rs`, runtime tests | canonical executable and composition/CLI/MCP regressions |
| `apps/lattice-runtime/src/fixtures/task032-scripted-codex.ps1` | immutable trusted scripted app-server fixture |
| `scripts/run-lattice-delivery.ps1`, `package.json` | bounded acceptance harness, official incident gate, and project-only Node test discovery |
| `PLANS.md`, SPEC-002, ADR-021, affected module constitutions, TASK-032 | minimal approved architecture and current incident/acceptance state |
| `docs/workflow/WORKFLOW_LEDGER.md`, `HANDOFF.md` | durable evidence and continuation boundary |

## Verification Evidence

- `cargo fmt --all -- --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`:
  exit 0, zero warnings.
- `cargo test --workspace --all-targets --all-features --locked`: exit 0; all
  workspace test binaries pass, including 7 composition and 11 MCP tests.
- `npm.cmd run verify`: exit 0; project check passes and Node tests are 44/44.
- `git diff --check`: exit 0; one non-failing LF-to-CRLF notice only.
- PowerShell AST parse for the harness and checked-in fixture: pass.
- `cargo tree -p lattice-orchestrator --depth 1`: only Contracts and Ports.
- Trusted scripted fixture
  `target/lattice-delivery/c9bf2939ad5844e9973ee0af0a84b756`:
  PostgreSQL 17.10 initial/restart pass, status `COMPLETED`, runtime
  `SCRIPTED_ACCEPTANCE`, clean fixture repository, and commit
  `ed408cc4373519f57950a66660148df39f9d5f82` changing only `answer.txt`.
- The run/status evidence agrees on request, profile, configuration, intent,
  outcome, and receipt digests; final evidence is
  `target/lattice-delivery/c9bf2939ad5844e9973ee0af0a84b756/evidence/final.json`.

## Official Live Failed Diagnostic

- The official attempt is not acceptance evidence. Windows displayed
  `codex-windows-sandbox-setup.exe` with "The specified module could not be
  found" while the isolated fixture contained an uncommitted `answer.txt`.
- The OpenAI-signed x64 helper SHA-256 is
  `7191d24f6fb4a26cbbce0d2aecd6deb71fa074a8cb5f24a45d2fa2164473885f`;
  its direct system imports were resolvable. OpenAI issues
  [#29952](https://github.com/openai/codex/issues/29952) and
  [#29200](https://github.com/openai/codex/issues/29200) report the same open
  Windows sandbox-helper regression/compatibility failure.
- Read-only package evidence identifies `@openai/codex` 0.144.6 and signed
  native `bin/codex.exe` SHA-256
  `4b76ded066d0239115ca97473d010c92072bc5c5550a45dd7cbebe1e9eb956a7`.
  The helper's exact path is
  `C:\Users\f7212\AppData\Roaming\npm\node_modules\@openai\codex\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\codex-resources\codex-windows-sandbox-setup.exe`.
- No exact missing DLL, helper stderr, or helper exit code was captured; none is
  claimed. The preserved diagnostic fixture is
  `target/lattice-delivery/1b1e1661d9e843e2b9e4774b93bf0dc9` at initial
  commit `94ba7385b81dd607c8a271a3c988e0f9bc82fac1`, with untracked
  `answer.txt` and no delivery commit.
- No official live or sandbox setup was retried after the user stop, no
  unelevated/no-sandbox mode was selected, and no system component was changed.

## Workflow And Review Ledger

| Stage | Status | Evidence |
|---|---|---|
| Repository/Git inspection | valid | feature branch, base `4cf98cf`, dirty candidate preserved, no remote |
| Clarification/spec/ticket | valid | prior explicit approval reused; SPEC-002 v26 and TASK-032 remain current |
| Module governance | valid | approved versioned constitutions and ADR-021; no silent boundary change |
| TDD implementation | valid | observable red/green coverage for ordering, deadlines, replay, protocol, Git, and fixture trust |
| Focused/full verification | pass | Rust format/Clippy/tests, Node 44/44, AST, diff and dependency checks |
| Scripted runtime acceptance | pass | real PostgreSQL restart plus real isolated Git commit; explicitly not official live |
| Independent code review | pass | final P0=0/P1=0/P2=4/P3=1; non-blocking findings recorded below |
| Architecture review | pass with debt | pure ports boundary and two-tool surface preserved; P0=0/P1=0 |
| Integration/CI/merge | partial/blocked | local combined checks pass; no remote, CI/branch protection, push, or merge authorization |
| Handoff | complete for checkpoint | this file plus `docs/workflow/WORKFLOW_LEDGER.md` |

## Remaining Gaps And Next Step

- P2: PostgreSQL sets remaining session timeouts at connection, but lower-level
  transactions can replace them with fixed 5s/30s values. A return after the
  absolute deadline is safely `Ambiguous`, yet latency can exceed the caller
  deadline.
- P2: Windows Codex cleanup uses unbounded `taskkill.exe.status()`/`child.wait()`
  after protocol completion; it cannot falsely report success but could delay
  terminal persistence.
- P2: Codex stdout uses an unbounded channel and `read_line` allocates before
  the 8 MiB line check. The trusted fixture cannot exploit this, but bounded
  framing/backpressure is required before removing the official incident gate.
- P2: the child-environment denylist misses exact generic names such as
  `API_KEY`, `TOKEN`, `SECRET`, and `PASSWORD`. No leak was observed and the
  pinned fixture does not inspect them, but official mode should use a narrow
  environment allowlist before its gate is removed.
- P3: MCP `initialize` accepts an empty `clientInfo` object instead of requiring
  non-empty `name` and `version`.
- The global project-memory router path was missing (`MODULE_NOT_FOUND`), so
  that documented global routing gate is unavailable in this environment.
- Wait for an upstream helper correction or an explicit user decision on a new
  safety posture before any official live retry. Only after official
  modify/test/commit/restart evidence passes may TASK-032 close and the bounded
  OpenClaw, Graphify, Hermes, and Codebase Memory nodes begin.
- No push, merge, publication, deployment, payment, account/credential change,
  or production mutation was performed.

## Restart Context

- Branch: `feature/v2-rust-postgres-bootstrap`; checkpoint base:
  `4cf98cf3f9e3b53d0e819139cdfd96ff457e587a`; no remote.
- Active ticket: TASK-032 (`in-progress`); active plan marker remains CURRENT.
- Begin with `git status --short --branch`, then read this section, PLANS.md,
  TASK-032, and the active incident gate. Do not retry official/sandbox setup.
- Overall goal remains active; do not mark TASK-032 or MVP-1 complete.

---

# Archived LATTICE DevOS TASK-021 Handoff

## Outcome

TASK-021 is complete. LATTICE now has its first durable domain repository:
Task Ledger 2.1 remains the pure Rust semantic owner, while Postgres Store 1.3
atomically persists each terminal command, optional event and outbox admission,
projection/checkpoint, and applied physical Store receipt in PostgreSQL.
SPEC-002 AC-03, AC-04, and AC-35 are complete.

This does not complete MVP-1, MVP-2, MVP-3, or the full platform. MVP-1 is
12/22 tickets (54.5%); AC-05 and AC-19 remain open. Outbox claim/delivery,
live resource observations, the other durable repositories, daemon activation,
OpenClaw/Codex/Graphify/Hermes/Codebase Memory live integration, Guardian
autonomy, production, release, and deployment remain later work.

LATTICE remains the user's general local autonomous AI development platform:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated companion/playmate website remains excluded from the product,
architecture, roadmap, implementation, tests, and project evidence.

## Completed In TASK-021

- Task Ledger 2.1 exposes one Fake/Live pure vacant/plan/apply/checkpoint
  boundary, retains verified appended and denied commands, and derives exactly
  one outbox admission only for an appended `EFFECT_INTENT` with outcome
  `RECORDED`.
- Postgres Store 1.3 adds exact transaction-control-free schema v3 while
  keeping `0001` through `0003` byte-identical and preserving the immutable
  Store-v2 receipt profile for historical exact replay.
- Runtime receives exactly three Store-v3 and five Task-Ledger-v1 fixed
  functions, with zero direct protected-table SELECT/DML and no generic SQL,
  DSN, credential, environment-discovery, or raw-client surface.
- Each new command runs in one bounded `SERIALIZABLE` transaction. Store and
  Ledger finalization are two ordered fixed calls in that same transaction;
  the Ledger finalizer accepts only the Store terminal created by the current
  transaction and any later failure rolls both back.
- Every read/write re-observes dynamic global schema/full-manifest evidence in
  the transaction and compares it with constructor-frozen evidence. Store-v2
  receipt evidence remains separate from global-v3 evidence.
- Outbox replay verifies event digest, command ID, and request digest linkage.
  Duplicate, missing, cross-project/snapshot, checkpoint, terminal, event,
  command, or outbox corruption fails closed and is never auto-repaired.
- Explicit database responses remain known retryable or terminal outcomes;
  only no database response at commit yields `CommitOutcomeUnknown` and
  poisons the adapter. Transactions/functions use 5-second lock and 30-second
  statement timeouts.
- Fresh Store genesis is correctly distinct from the vacant Ledger checkpoint
  until the first atomic mutation. A vacant stream with a same-ID wrong-scope
  physical orphan now fails closed through the three-argument fixed read-head
  function and a direct live regression.

## Review History And Repairs

The initial independent code review blocked closure with P1=4 and P2=2:
constructor-only global evidence, overly broad unknown-commit classification,
incomplete outbox linkage checks, acceptance of a prior Store terminal,
wrong-scope physical-load gaps, and missing bounded timeouts. Architecture
review also blocked on transaction provenance and bounded failure semantics.

All findings received direct repairs/regressions. Live acceptance then exposed
and repaired the fresh Store-genesis/vacant-checkpoint finalizer mismatch and a
test-only ACTIVE-admission restart cleanup gap. Final re-review found one more
P2 vacant wrong-scope orphan read gap; the SQL/Rust/live compatibility unit was
repaired and the full PostgreSQL matrix rerun. Final code/security and
architecture reviews report P0=0, P1=0, P2=0, P3=0; local integration passes.

## Verification Evidence

- PostgreSQL 17.10 marker-owned harness: latest frozen initial and restart
  phases pass, including migration/upgrade, old Store replay, new/exact/
  changed/stale/outbox commands, same/cross-stream concurrency, rollback,
  bounded retry, commit-ack loss, coherent manifest drift, lock timeout,
  current-transaction `xmin`, ACL, wrong-scope orphan, corruption, and cleanup.
- Rust workspace: 432/432 tests across 52 binaries.
- Postgres Store package: 57/57 tests; migration contract: 15/15.
- Preserved Node verification: 44/44 tests.
- `cargo fmt` and strict workspace/all-targets Clippy: pass, zero warnings.
- `cargo audit`: 109 locked dependencies checked against 1,178 advisories;
  zero known vulnerabilities.
- `0004_task_ledger_repository.sql`: 111,742 bytes, SHA-256
  `cd658ed2f4624cd0a829c818c1cf96d8ac3829264046976bdde3b2fc7feea6e5`.
- Full four-entry manifest:
  `09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407`.
- `0001`/`0002`/`0003` retain their TASK-020 bytes and hashes.
- No unmerged path, conflict marker, tracked whitespace error, PowerShell parse
  error, reverse adapter dependency, or temporary focus switch remains.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_021_2026-08-02.md`
- `docs/reviews/GOVERNANCE_REVIEW_TASK_021_2026-08-02.md`
- `docs/reviews/CODE_REVIEW_TASK_021_2026-08-02.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_021_2026-08-02.md`
- `docs/reviews/INTEGRATION_TASK_021_2026-08-02.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

TASK-021 stayed aligned with PLANS Step 6, SPEC-002 v23, ADR-019, Task Ledger
2.1, and Postgres Store 1.3. It adds durable Ledger/outbox-admission truth
without giving PostgreSQL domain meaning or adding outbox delivery, another
writer/truth/gateway, provider/product work, or the unrelated website.

| Gate | Current classification |
|---|---|
| Local Rust/Node tests, format, lint, audit, migration hashes | machine-enforced for this run |
| Disposable PostgreSQL transaction/concurrency/fault/restart behavior | machine-enforced for the exact marker-owned target |
| Fixed functions, direct-table denial, roles, ACLs, catalog and scope | machine-enforced locally plus independent review |
| Module ownership and dependency direction | independently reviewed plus local scans |
| Ticket allowlist | documented plus local scan; no clean per-ticket commit |
| Remote Rust/PostgreSQL CI and branch protection | missing/unverified |
| Primary merge readiness | blocked; no committed candidate, remote, or merge authorization |

## Git, Runtime, And Cleanup State

- Branch: `feature/v2-rust-postgres-bootstrap`; HEAD:
  `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`;
  feature is four committed commits ahead and not behind.
- Remote/upstream: none. MVP-0 through TASK-021 remain one cumulative
  uncommitted dirty result; no merge was performed.
- No disposable PostgreSQL/Cargo/test process remains. The installed Windows
  PostgreSQL 17 service was not replaced or stopped.
- Two stopped ignored diagnostic roots remain under `target/`, both without
  `postmaster.pid` or listeners. Exact removal was blocked by local policy and
  was not bypassed; this is a disclosed hygiene note, not live database state.

## Next Bounded Slice

TASK-022 governance must freeze the durable Project Registry repository before
implementation. It must reuse Project Registry's pure identity/reconciliation
semantics and Postgres Store's schema-v3 transaction/authority boundary,
preserve one-way dependency and project isolation, and prove restart,
concurrency, drift/collision, corruption, and exact replay. It must not start
Writer Lease, Approval, Artifact, OpenClaw/Codex/Graphify/Hermes/Memory,
Guardian, production, release/deploy, or unrelated website work.

---

# Archived TASK-020 Handoff

## Outcome

TASK-020 is complete. LATTICE now has one exact live PostgreSQL 17 physical
`ControlStore`: Contracts 1.9 and Ports 1.4 preserve the fake while Postgres
Store 1.2 supplies schema-v2 migration, fixed-function runtime access,
durable apply/stale terminal receipts, exact replay, bounded pre-commit retry,
unknown-commit reconciliation, and restart evidence. SPEC-002 AC-34 is
complete.

This is a physical durability boundary, not a domain repository. AC-03,
AC-04, AC-05, and AC-19 remain open, and MVP-1, MVP-2, MVP-3, and the full
platform are not yet complete.

LATTICE remains the user's general local autonomous AI development platform:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated companion/playmate website is excluded from the product,
architecture, roadmap, implementation, tests, and project evidence.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-020 complete, TASK-021 next.
- MVP-2 exact-version local components and Codebase Memory: planned after the
  MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2 gate.

## Completed In TASK-020

- Preserved Store contract v1 as visibly fake/non-durable and introduced exact
  v2 live PostgreSQL persistence evidence without changing unrelated contract
  behavior.
- Changed only the synchronous `ControlStore::current_head` observation to
  explicit mutable access; Ports remains Contracts-only and driver-free.
- Kept `0001` and `0002` byte-identical and added one exact
  `0003_live_control_store.sql` expansion. Fresh targets and verified empty
  exact-v1 prefixes upgrade; drifted, partial, reordered, edited, unknown, or
  non-empty sources fail closed.
- Added exactly three fixed `SECURITY DEFINER` runtime functions with safe
  ownership/search-path/ACL properties. Runtime has no direct physical or
  terminal table access and cannot migrate or self-activate.
- Added `PostgresControlStore` over a caller-supplied authenticated client. It
  rechecks exact ACTIVE daemon authority and the locked physical head inside a
  bounded `SERIALIZABLE` transaction, and returns durable evidence only after
  successful commit.
- Exact retry remains byte-identical after admission, epoch, or head changes;
  changed transaction-ID reuse reveals no retained receipt. Unknown commit
  response returns no receipt, poisons the instance, and requires reconnect
  plus the exact request for reconciliation.
- Expanded the marker-owned PostgreSQL harness for fresh/upgrade/rollback,
  permissions, apply/stale/replay/substitution, concurrency, serialization
  exhaustion, overflow, corruption, response loss, restart, and exact cleanup.

## Verification Evidence

- Full Rust workspace: 409/409 tests.
- Preserved Node suite: 44/44 tests.
- `cargo fmt` and strict all-target/all-feature Clippy: pass with zero warnings.
- `cargo audit`: 109 locked dependencies checked against 1,178 advisories;
  zero known vulnerabilities.
- Duplicate dependency roots: zero.
- PostgreSQL 17.10 marker-owned initial/restart harness: self-test and complete
  TASK-020 live/fault/upgrade matrix pass.
- `0003_live_control_store.sql`: 29,518 bytes, SHA-256
  `00ae3eedd76704f26b1df58955d9d594c98f0ba525be93b15d8c9ebb1f2115c1`.
- PowerShell AST, source-scope, secret/DSN/raw-client/dynamic-SQL, conflict,
  temporary-marker, whitespace, dependency, and governance checks pass.
- Independent code/security and architecture reviews pass with P0-P3 all zero;
  local combined integration passes.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_020_2026-08-02.md`
- `docs/reviews/CODE_REVIEW_TASK_020_2026-08-02.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_020_2026-08-02.md`
- `docs/reviews/INTEGRATION_TASK_020_2026-08-02.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

TASK-020 stayed aligned with PLANS Step 6, SPEC-002 v22, ADR-018, and the
versioned Contracts/Ports/Postgres Store constitutions. It makes physical
PostgreSQL durability real without confusing it with Ledger, Registry, Lease,
Approval, Artifact, Guardian, provider/product, release, or production
authority.

| Gate | Current classification |
|---|---|
| Local Rust/Node format, tests, strict lint, audit, and migration hashes | machine-enforced for this run |
| Disposable PostgreSQL transaction/concurrency/fault/restart evidence | machine-enforced for the exact marker-owned target |
| Fixed runtime functions, direct-table denial, roles, ACLs, and catalog shape | machine-enforced locally plus independent review |
| One Gateway/Truth/Writer and dependency/domain boundaries | independently reviewed and locally scanned |
| TASK-020 ticket allowlist | documented plus local scan; no per-ticket committed diff |
| Remote Rust/PostgreSQL CI, branch protection, remote synchronization | missing/unverified |
| Primary-branch merge readiness | blocked; no committed candidate, remote, or authorization |

## Git And Scope State

- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Primary/V2 committed ahead/behind: `0/4`; remote/upstream: none.
- MVP-0 through TASK-020 remain one inspectable cumulative uncommitted result.
- No reset, clean, branch switch, commit, push, merge, production database or
  credential mutation, publication, deployment, deletion, or protected action
  occurred.

## Next Bounded Slice

TASK-021 must first freeze the durable Task Ledger adapter boundary before any
code change. It will add one domain-owned composition layer that persists the
canonical Task Ledger request, event/head, terminal command receipt, and
outbox admission atomically in PostgreSQL, reuses the pure Ledger verifier as
the semantic owner, and proves idempotency, concurrency, corruption denial,
unknown-commit reconciliation, and restart replay. It must not give the pure
domain crate I/O, make Postgres Store depend on a domain, grant runtime generic
SQL, or introduce OpenClaw/Codex/Graphify/Hermes/provider/product/release work.

---

# Archived TASK-019 Handoff

## Outcome

TASK-019 Postgres Store 1.1.5 is complete for its exact-manifest PostgreSQL
17 schema, permission, compatibility, and STOPPED-admission foundation.
SPEC-002 AC-33 is complete. This does not complete a live `ControlStore`,
durable domain repositories, AC-03/04/05/19, MVP-1, MVP-2, MVP-3, or the whole
platform.

LATTICE remains a general local AI development platform for the user's
computer:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated companion/playmate website is not part of this product,
architecture, roadmap, implementation, or test target.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-019 is complete.
- MVP-2 exact-version local components and Codebase Memory: planned after the
  MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2 gate.

## Completed In TASK-019

- Pinned the synchronous `postgres` 0.19.14 driver with default features off
  and SHA-256 0.11.0 while retaining the Contracts-only Ports boundary.
- Added a compile-time exact migration manifest. The fixed `0001` remains
  byte-identical and `SUPERSEDED`; transaction-control-free `0002` creates only
  database identity/history/compatibility, physical transaction foundations,
  and STOPPED/no-leader admission.
- Added the explicit administrative runner and read-only repeatable-read
  verifier with exact target sentinel, transaction-scoped concurrency lock,
  no-op retry, uncertain/committed-unverified reconciliation, and full catalog
  drift closure.
- Enforced real LOGIN-to-NOLOGIN capability separation, CONNECT-only bootstrap,
  cluster-wide ACL/default-ACL/ownership closure, exact protected-function
  denial, `max_prepared_transactions = 0`, and no notification authority.
- Added a marker-owned PostgreSQL 17.10 harness that uses a fresh non-5432
  loopback cluster, leaves the installed service untouched, restarts its own
  cluster, proves real LOGIN permissions, and deletes only its exact root after
  stopped-state and marker verification.
- Kept the deterministic fake and `ControlStore` unchanged; no live/durable
  receipt, domain write, self-activation, production credential, provider,
  product, or website behavior was added.

## Verification Evidence

- Postgres Store focused tests: 35/35.
- Full Rust workspace: 401/401.
- Preserved Node suite: 44/44.
- Format and strict all-target/all-feature Clippy pass with zero warnings.
- Two fresh PostgreSQL 17.10 trials report
  `TASK019_HARNESS_SELF_TEST=PASS` and `TASK019_POSTGRES_HARNESS=PASS` for both
  initial and restart phases.
- PowerShell AST, dependency tree, duplicate dependency, migration hash,
  debug-marker, temporary-artifact, diff, and governance checks pass.
- Independent code/security and architecture reviews pass with P0-P3 all zero;
  local combined integration passes.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_019_2026-08-01.md`
- `docs/reviews/CODE_REVIEW_TASK_019_2026-08-01.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_019_2026-08-01.md`
- `docs/reviews/INTEGRATION_TASK_019_2026-08-01.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

TASK-019 stayed aligned with PLANS, ADR-017, and Postgres Store 1.1.5. It made
PostgreSQL evidence real without confusing a schema foundation with a live
Store, domain truth, leader activation, or production release.

| Gate | Current classification |
|---|---|
| Manifest, runner, verifier, role/catalog/settings and harness behavior | machine-enforced locally |
| Disposable PostgreSQL transaction/concurrency/restart/permission evidence | machine-enforced locally twice |
| One Gateway/Truth/Writer and dependency boundaries | independently reviewed and locally scanned |
| Live physical Store and durable/domain receipts | missing/deferred to TASK-020+ |
| `cargo-audit`, remote Rust CI, branch protection | unavailable or missing/unverified |
| Merge readiness | blocked; no committed candidate, remote, or authorization |

## Git And Scope State

- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Primary/V2 committed ahead/behind: `0/4`; remote/upstream: none.
- MVP-0 through TASK-019 remain one inspectable uncommitted dirty result.
- No reset, clean, branch switch, commit, push, merge, production database or
  credential mutation, publication, deployment, deletion, or protected action
  occurred.

## Next Bounded Slice

TASK-020 will version the affected contracts before implementing the live
physical PostgreSQL `ControlStore`. It must revalidate exact ACTIVE daemon
authority and physical head in the same transaction, retain exact terminal
receipts for reconciliation, expose only narrow runtime operations, and keep
all Registry/Ledger/Lease/Approval/Artifact legality and Guardian activation
outside this slice.

---

# Archived TASK-018 Handoff

## Outcome

TASK-018 Postgres Store 1.0 is complete for its typed zero-I/O MVP-1 boundary.
SPEC-002 AC-32 is complete. This does not complete durable PostgreSQL
AC-03/04/05/19, MVP-1, MVP-2, MVP-3, or the whole platform.

LATTICE remains a general local AI development platform for the user's
computer:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated playmate website is not part of this product, architecture, or
roadmap. Ten old `ACCESS_PLAYMATE` strings remain only as explicit V1
compatibility/denial fixtures; active V2 and TASK-018 paths have zero coupling.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-018 is complete.
- MVP-2 exact-version local components and Codebase Memory: planned after the
  MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2 gate.

## Completed In TASK-018

- Contracts 1.8 adds bounded canonical Store transaction, daemon, scope,
  authority, physical-head, commitment, disposition, and receipt values.
- Ports 1.3 exposes only typed `ControlStore::transact` and `current_head`
  through Store-specific failures and remains Contracts-only.
- Postgres Store 1.0 implements a deterministic in-memory fake with Store-owned
  genesis/head hashes, complete request hashing, exact replay, changed-ID
  substitution denial, project/snapshot/owner/aggregate isolation, atomic head
  and receipt apply, stable stale denial, bounded capacity/serialization, and
  explicit before/after-apply fault reconciliation.
- Every receipt is fixed to `RuntimeKind::Fake` and `NonDurableFake`; no driver,
  SQL, connection, migration runner, or durable constructor exists.
- Project governance now rejects duplicate ticket IDs, invalid current-marker
  cardinality, a marker without its unique ticket, a current ticket without its
  module constitution, non-canonical constitution paths, and duplicate module
  IDs without forcing future modules active early.

## Review Repairs

Independent review found and closed bounded/canonical snapshot identity,
replay-integrity ordering, changed-ID substituted-scope probing, arbitrary
physical-head injection, revision-zero genesis override, canonical constitution
path, and stale-disposition documentation drift. Each behavioral finding
received a RED/GREEN regression. Final code/security and architecture reviews
pass with P0=0, P1=0, P2=0, P3=0.

## Verification Evidence

- Focused locked package tests: Contracts 42, Ports 5, Store 14 (61 total).
- Full locked Rust workspace: 380/380.
- Preserved Node verification: 44/44.
- Format, strict workspace Clippy, dependency tree, forbidden driver, scoped
  I/O/SQL/credential/provider/product/website, migration inactivity,
  governance, and diff/untracked hygiene checks pass.
- Migration unchanged: SHA-256
  `7BFF021FC17F738551309C906578C8015B2DD0307D27D239C21DF1697C4D09C8`,
  Git blob `5c1bb61e220980b2087d4ec7a3c61a50a9d23ec5`.
- Independent local combined integration: `PASS`.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_018_2026-08-01.md`
- `docs/reviews/CODE_REVIEW_TASK_018_2026-08-01.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_018_2026-08-01.md`
- `docs/reviews/INTEGRATION_TASK_018_2026-08-01.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

TASK-018 stayed aligned with PLANS and ADR-016: it froze the physical
transaction boundary before real database work and did not duplicate domain
legality or durable truth.

| Gate | Current classification |
|---|---|
| Store contract/fake scope, hashing, atomicity, replay, faults | machine-enforced locally |
| Dependency/no-I/O/driver/migration inactivity | locally tested and scanned |
| Current-ticket/constitution governance | machine-enforced locally |
| PostgreSQL durability, restart/concurrency, roles/time/admission | missing/deferred to TASK-019+ |
| Remote Rust CI and branch protection | missing/unverified |
| Merge readiness | blocked; no committed candidate, remote, or authorization |

## Git And Scope State

- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Primary/V2 committed ahead/behind: `0/4`; remote/upstream: none.
- MVP-0 through TASK-018 remain one inspectable uncommitted dirty result.
- No reset, clean, branch switch, commit, push, merge, database mutation,
  publication, deployment, credential/account/payment change, deletion, or
  protected action occurred.

## Next Bounded Slice

`PLANS.md` marks `CURRENT TASK-019 GOVERNANCE`. SPEC-002 v16, ADR-017,
Postgres Store 1.1, and TASK-019 now freeze the exact-manifest PostgreSQL 17
schema/admission foundation. The bounded implementation adds a pinned
synchronous driver, an explicit administrative runner, read-only runtime
verifier, STOPPED/no-leader bootstrap, role separation, and a marker-owned
disposable 17.10 cluster. It does not add a live `ControlStore`, durable
receipt, production credential/role/database change, daemon activation,
public exposure, or unrelated website work.

## Restart Context

- Overall goal remains active through MVP-3; do not mark it complete.
- Current MVP: MVP-1 offline control core.
- Completed ticket: `docs/tickets/TASK-018-postgres-store-boundary.md`.
- Current ticket:
  `docs/tickets/TASK-019-postgres-manifest-admission-foundation.md`.
- Continue bounded reversible local work automatically; protected actions and
  primary merge remain fail-closed.

---

# Archived LATTICE DevOS TASK-017 Handoff

## Outcome

TASK-017 Gateway IPC 1.1 / wire protocol 1.0 is complete for its bounded
pure/fake MVP-1 scope. SPEC-002 AC-31 is complete. This does not complete the
live portion of AC-07, MVP-1, MVP-2, MVP-3, or the whole platform.

LATTICE remains a general local AI development platform for the user's
computer:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated playmate website is not part of this repository, architecture,
tests, or roadmap.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-017 is complete.
- MVP-2 exact-version local OpenClaw/Codex/Graphify/Hermes plus Codebase Memory:
  planned after the MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2
  authority/containment gate.

No routine human decision blocks TASK-018. Credentials, account/payment
changes, public exposure/publication/deployment, irreversible real effects,
security-control changes, protected release activation, and primary-branch
merge remain separately protected.

## Completed In TASK-017

### Contracts 1.7 And Ports 1.2

- Added neutral bounded peer/request/reply values for exactly Submit, Plan,
  Status, normal Approve/Reject, and task Stop. Actions derive from closed typed
  bodies; no arbitrary action, SQL, shell, path, provider, daemon, or release
  escape hatch exists.
- Bounded gateway-reused task/snapshot/attempt identifiers to 256 bytes and
  rejected all-zero authority/freshness/receipt/observation/terminal digests.
- Bound reply action, request, command, correlation, subject, page size,
  disposition, and evidence before canonical hashing.
- Changed `GatewayService` to accept server-derived peer context and a complete
  request, returning a bound reply through component-free
  `GatewayServiceError`; Rust-core failures can no longer be mislabeled as
  OpenClaw failures.

### Gateway IPC 1.1 / Wire Protocol 1.0

- Added a strict canonical JSON codec with a raw 1 MiB frame cap, depth 32,
  node 8,192, array 256, no numbers, exact fields, duplicate/unknown/version/
  action rejection, complete trailing-data checks, and redacted bounded errors.
- Added allocation-free NFC preflight for values and keys. Non-NFC request and
  reply identities, including normalization-expanding exact-bound inputs, fail
  before canonical hashing/allocation, replay insertion, or service dispatch.
- Added domain-separated request/reply digests and mechanical Task Spec 2.1
  canonical-document digest/binding verification without copying Task Domain
  semantics or exposing raw source.
- Added a pure in-memory fake client/server. Role authorization precedes replay;
  exact `(project, actor, command)` retry returns identical terminal bytes;
  changed content denies without another service call. Replay storage is capped
  at 1,024 entries while retained exact retries remain readable.
- Recovery can route only bounded Status and task Stop. ProtectedChange routes
  normally; raw `PROTECTED_RELEASE` is unrepresentable and rejected by the
  codec before service dispatch.
- Project status observes at most both the request page size and global 100-
  item cap. Stop preserves REQUESTED, ALREADY_TERMINAL, and
  RECONCILIATION_REQUIRED without claiming process interruption or completion.

### Repository Governance Repair

- `scripts/check-project.mjs` now rejects duplicate `ticket_id` values and any
  `PLANS.md` state with other than one `CURRENT TASK-nnn` marker.
- Three disposable Node regressions prove duplicate denial, multiple-marker
  denial, and the valid unique/single-marker case.
- Contracts owns neutral in-process representations and constructor-level
  identifier/cursor/page bounds. Gateway IPC owns wire layout, parser/encoded-
  frame limits, NFC enforcement, hash subjects, and replay. SPEC v14, ADR-015,
  module constitutions, routing index, ticket, and plan agree on that split.

## Review Repairs

The initial independent review found nine blockers: replay before role
authorization, zero digest sentinels, reply hashing before bounds, oversized
reused IDs, ignored request page size, unbounded replay storage, contradictory
protected semantics, incomplete reply/substitution matrices, and duplicate
TASK-017 tickets.

Final review additionally found and closed non-NFC identity/size expansion,
typed-encoder fast-fail ordering, false external component attribution for
Rust-core errors, Contracts/wire ownership drift, dependency/version drift,
and missing machine enforcement for ticket/current-marker uniqueness.

Every accepted behavioral finding received a failing regression before repair.
Final independent code/security and architecture reviews report `PASS`, with
P0=0, P1=0, P2=0, and P3=0.

## Files Added Or Materially Changed

- Gateway code/tests: `crates/lattice-gateway-ipc/**`.
- Shared interfaces/tests: `crates/lattice-contracts/{src,tests}` and
  `crates/lattice-ports/{src,tests}`.
- Workspace dependency graph: `Cargo.toml`, `Cargo.lock`, and Gateway manifest.
- Machine governance: `scripts/check-project.mjs` and
  `test/project-governance-check.test.js`.
- Governance/delivery: SPEC-002 v14, ADR-015, Gateway/Contracts/Ports and
  related module documents, TASK-017, PLANS, workflow audit, final review and
  integration reports, workflow ledger, and this handoff.

## Verification Evidence

All final local commands completed successfully:

- focused locked suites: Contracts 36, Gateway IPC 31, Ports 3 (70 total).
- `cargo test --workspace --locked`: 358 Rust tests.
- strict workspace Clippy, all targets/features, locked, `-D warnings`.
- `cargo fmt --all -- --check`.
- `npm.cmd run verify`: 41 Node tests; project check reports 221 files,
  17 constitutions, 17 unique tickets, and one current task.
- Gateway Cargo tree: only Contracts, Ports, cjson, exact serde/serde_json, and
  exact Unicode normalization plus approved transitives.
- forbidden filesystem/network/process/database/Git/provider/product and
  unrelated-website scans: zero scoped implementation matches.
- `git diff --check`: pass.
- independent code/security and architecture review: `PASS`, zero P0-P3.

Review artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_017_2026-08-01.md`
- `docs/reviews/CODE_REVIEW_TASK_017_2026-08-01.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_017_2026-08-01.md`
- `docs/reviews/INTEGRATION_TASK_017_2026-08-01.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Alignment And Enforcement Truth

The work stayed aligned with PLANS Step 5 and the MVP-1 goal: the gateway
protocol is frozen before live transport, PostgreSQL, Codex, Graphify, Hermes,
or Codebase Memory composition.

| Gate | Current classification |
|---|---|
| Pure codec/fake action, binding, role, retry, limit, and fault behavior | machine-enforced locally |
| Project/actor/command isolation and bounded replay | machine-enforced locally |
| Unique ticket IDs and exactly one current task | machine-enforced locally |
| Dependency/no-I/O direction | locally linted and inspected |
| Contracts/wire ownership and fake/live boundary | documented plus structurally checked |
| Live OpenClaw transport, ACL, peer identity, restart, compatibility | missing/deferred under AC-07 |
| PostgreSQL durability and composed One Truth | missing/deferred |
| Remote Rust CI and branch protection | missing/unverified |
| Merge readiness | blocked; no committed candidate, remote, or authorization |

## Git And Scope State

- Branch: `feature/v2-rust-postgres-bootstrap`.
- HEAD/base: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`.
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`.
- Primary/V2 committed ahead/behind: `0/4`; remote/upstream: none.
- MVP-0 through TASK-017 remain one inspectable uncommitted dirty result.
- TASK-017-identifiable paths fit its final allowlist; exact shared-file
  per-ticket diff is partial/documented-only because prior V2 work is also
  uncommitted.
- No reset, clean, branch switch, commit, push, merge, installation, live
  database mutation, publication, deployment, credential/account/payment
  change, real delete, or protected action occurred.

## Incomplete And Explicitly Open

- AC-07 remains open for real OpenClaw package/schema/binary, OS-local
  transport/ACL/peer authentication, session restart/disconnect, and durable
  terminal receipt evidence.
- TASK-018 through TASK-031 remain for PostgreSQL stores, filesystem/Git/scope,
  review and Codex fakes, offline end-to-end composition, compatibility, and
  the MVP-1 exit gate.
- Exact-version live OpenClaw, Codex, Graphify, Hermes, and Codebase Memory
  remain MVP-2.
- Guardian-protected improvement, A/B activation, canary, and rollback remain
  MVP-3.

## Next Bounded Slice

`PLANS.md` marks `CURRENT TASK-018 GOVERNANCE`.

TASK-018 must freeze a typed, zero-I/O Postgres Store 1.0 boundary and
deterministic fake before any database connection:

1. re-audit TASK-017 closure and current repository/Git state;
2. define transaction request/result/error and exact project/authority/
   idempotency binding without copying domain legality;
3. keep PostgreSQL as the future sole durable writer/truth while the TASK-018
   fake remains visibly non-durable and performs no database I/O;
4. freeze migration ownership and compatibility boundaries for TASK-019;
5. update SPEC/ADR/module constitution/ticket before RED tests;
6. repeat TDD, focused/full verification, independent reviews, integration,
   ledger, and handoff.

## Restart Context

- Overall goal remains active through MVP-3; do not mark it complete.
- Current MVP: MVP-1 offline control core.
- Current marker: `CURRENT TASK-018 GOVERNANCE` in `PLANS.md`.
- Completed ticket: `docs/tickets/TASK-017-gateway-ipc.md`.
- Continue bounded, reversible local work automatically; do not introduce the
  unrelated playmate website.

---

# Archived LATTICE DevOS TASK-016 Handoff

## Outcome

TASK-016 Artifact Store 1.0 is complete for its bounded pure/fake MVP-1 scope.
SPEC-002 AC-30 is complete. This is not completion of AC-19, MVP-1, MVP-2,
MVP-3, or the full platform.

The product remains a general local AI development platform for the user's
computer:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The unrelated playmate website is not part of this repository, architecture,
tests, or roadmap.

Current milestone state:

- MVP-0 Rust foundation: complete.
- MVP-1 offline control core: in progress; TASK-016 is complete.
- MVP-2 exact-version local OpenClaw/Codex/Graphify/Hermes plus Codebase Memory:
  planned after the MVP-1 exit gate.
- MVP-3 Guardian-protected autonomy and upgrade: planned after the MVP-2
  authority/containment gate.

No routine human decision blocks the next bounded local ticket. Credentials,
account/payment changes, public exposure/publication/deployment, irreversible
real effects, security-control changes, protected release activation, and
primary-branch merge remain separately protected.

## Completed In TASK-016

### Contracts 1.6

- Added neutral immutable project-scoped object/generation, provenance,
  reference, read, sweep, availability/delete, fixed owner receipt, and full
  current-head representations.
- Enforced positive signed-BIGINT-safe counters, exact SHA-256 identity,
  canonical time, runtime/producer/status/action closure, and complete binding
  validation.

### Artifact Store 1.0

- Added one public `FakeArtifactStore` owner. Lifecycle, history, quotas,
  staging, bytes, and terminal maps are composed atomically behind it; lower
  mechanisms cannot be a public second writer.
- Verified length and SHA-256 before publication. Raw bytes remain confined to
  a separately redacted in-memory fake backend and never enter retained
  requests, receipts, snapshots, checkpoints, errors, or `Debug`.
- Implemented project-isolated content generations, immutable per-use
  references, complete provenance, typed fixed-owner current authority,
  release terminality, active/suspect read lifecycle, safe delete planning,
  exact claim token, unknown-outcome reconciliation, and higher-generation
  reintroduction.
- Implemented exact applied and denied command retry before stale/time checks;
  changed content under one scoped command key rejects permanently.
- Enforced hard and configured byte/manifest/field/bundle/object/reference/
  read/staging/command/history limits at object, task, project, and store
  scopes. Task object/active-byte attribution is active-reference-only;
  retained project/store capacity and worst-case claimed/reconciliation/orphan
  capacity remain held until verified terminal evidence.
- Included holder IDs, complete persisted lifecycle strings, and the 64-byte
  domain-separated delete claim token in `FieldBytes` accounting.
- Added strict raw snapshots containing complete sanitized terminal lifecycle
  receipts. Context-free replay reconstructs lifecycle, history, quotas,
  staging, command tasks, retired scopes, and terminals, validates all
  digests/joins, and then compares an independent compact checkpoint.
- The checkpoint retains only store/limit/snapshot/replay-bound/trust-anchor
  commitments; it contains neither an owner clone, metadata row set, nor
  payload. Untrusted canonical size is preflighted before allocation, including
  control-character escape expansion.

### Review Repairs

Independent review found and closed:

- checkpoint construction that temporarily copied payload bytes;
- canonical-byte bounds checked after output allocation;
- replay that returned a trusted owner clone instead of rebuilding raw input;
- missing full lifecycle receipts needed for context-free exact retry;
- missing holder/lifecycle/claim-token `FieldBytes` projection;
- direct applied-after-replay retry evidence.

Each accepted behavioral finding received a failing regression before repair.
Final code/security and architecture re-reviews report `PASS`, with P0=0,
P1=0, P2=0, and P3=0.

## Files Added Or Materially Changed

- Workspace/contracts: `Cargo.toml`, `Cargo.lock`,
  `crates/lattice-contracts/src/lib.rs`, and Contracts tests.
- Artifact Store owner and mechanics:
  `crates/lattice-artifact-store/src/{lib,aggregate,history,quota,quota_owner,semantics,snapshot}.rs`.
- Strict context-free restore:
  `src/aggregate/snapshot_restore.rs`, `src/semantics/snapshot_restore.rs`,
  `src/snapshot_parse.rs`, `src/snapshot_contract.rs`, and
  `src/snapshot_quota.rs` under the Artifact Store crate.
- Artifact Store behavior suites under
  `crates/lattice-artifact-store/tests/`, including owner, delete, read, quota,
  staging, history, lifecycle, and replay matrices.
- Governance/delivery: SPEC-002 v12, ADR-014, Artifact Store and Contracts
  constitutions, TASK-016, PLANS, workflow audit, final code/security review,
  final architecture review, integration report, workflow ledger, and this
  handoff.

## Verification Evidence

All commands below completed successfully:

- `cargo test -p lattice-contracts --locked`: 32 tests.
- `cargo test -p lattice-artifact-store --locked`: 97 tests.
- `cargo test -p lattice-artifact-store --test artifact_owner_replay --locked`:
  8 tests.
- `cargo test --workspace --all-targets --all-features --locked`: 322 Rust
  tests.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.
- `cargo fmt --all -- --check`.
- `npm.cmd run verify`: `check=ok`, 38 preserved Node tests.
- `cargo tree -p lattice-artifact-store --edges normal --locked`: only
  Contracts, cjson, SHA-256, time, and approved transitives.
- forbidden-I/O, provider/product dependency, and unrelated-website scans:
  zero scoped implementation/dependency matches.
- payload fixture containment assertions: raw snapshot, checkpoint, and owner
  debug contain no fixture bytes; replayed verified read reports `MissingBytes`.
- `git diff --check`: pass.

Verification artifacts:

- `docs/reviews/WORKFLOW_AUDIT_TASK_016_2026-07-30.md`
- `docs/reviews/CODE_REVIEW_TASK_016_2026-07-30.md`
- `docs/reviews/ARCHITECTURE_REVIEW_TASK_016_2026-07-30.md`
- `docs/reviews/INTEGRATION_TASK_016_2026-07-30.md`
- `docs/workflow/WORKFLOW_LEDGER.md`

## Workflow Ledger

| Stage | Status | Evidence |
|---|---|---|
| Inspect repository/Git | valid | workflow audit, branch/base/status |
| Clarify decisions | valid | SPEC v12 and ADR-014 |
| Specification | valid | AC-30 complete; durable/live AC-19 remains open |
| Module governance | valid | Artifact Store 1.0 and Contracts 1.6 |
| Ticket decomposition | valid | bounded non-parallel TASK-016 |
| Branch/worktree plan | valid | dirty V2 worktree preserved; V1 untouched |
| TDD implementation | valid | RED/GREEN behavior and review regressions |
| Focused/full verification | valid | 32 Contracts, 97 Artifact, 322 Rust, 38 Node |
| Code/security review | pass | zero remaining P0-P3 |
| Architecture review | pass | zero P0-P3; no amendment |
| Local integration | pass/partial | combined result passes; no committed candidate |
| Remote CI/merge | missing/blocked | no remote, CI, branch protection, candidate, or authorization |

## Alignment And Enforcement Truth

The work stayed aligned with PLANS Step 5 and the overall MVP-1 goal: one pure
artifact authority was frozen before PostgreSQL, filesystem effects, OpenClaw,
Codex, Graphify, Hermes, or Codebase Memory consumes it.

| Gate | Current classification |
|---|---|
| Pure artifact authority/quota/retry/replay/checkpoint behavior | machine-enforced locally |
| Project isolation and fixed owner/runtime contracts | machine-enforced locally |
| Dependency/no-I/O direction | locally linted and inspected |
| Governance semantics and ownership | documented plus structurally checked |
| PostgreSQL transactions/durability/restart | missing/deferred under AC-19 |
| Real filesystem containment/staging/delete | missing/deferred |
| Live provider/authority authentication | missing/deferred |
| One Gateway/One Truth/One Writer at composed runtime | documented-only in this slice |
| Remote Rust CI and branch protection | missing/unverified |
| Merge readiness | blocked; no committed candidate or authorization |

## Git And Scope State

- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- HEAD/base: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Primary branch: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`
- Primary/V2 committed ahead/behind: `0/4`
- Remote/upstream: none.
- The shared MVP-0 through TASK-016 result remains uncommitted and dirty.
- Identifiable TASK-016 paths fit its allowlist; exact per-ticket shared-file
  scope is `partial/documented-only` because prior V2 work is also uncommitted.
- No reset, clean, branch switch, commit, push, merge, installation,
  publication, deployment, credential/account/payment change, PostgreSQL
  mutation, real delete, or protected action occurred.

## Incomplete And Explicitly Open

- AC-19 remains open for PostgreSQL metadata/reference transactions,
  serialization, durability, restart, and same-transaction effect admission.
- Real filesystem staging/flush/rename/link containment/read/delete and orphan
  reconciliation remain open.
- Fake OpenClaw IPC, remaining PostgreSQL stores, workspace/scope enforcement,
  fake Codex/reviewer, offline end-to-end orchestration, and the MVP-1 exit gate
  remain incomplete.
- Exact-version OpenClaw, Codex, Graphify, Hermes, and Codebase Memory remain
  MVP-2.
- Guardian-protected improvement/activation/rollback remains MVP-3.

## Next Bounded Slice

At TASK-016 closure, `PLANS.md` then marked `CURRENT TASK-017 GOVERNANCE`;
TASK-017 is now complete as recorded at the top of this file.

TASK-017 should freeze and implement a pure/fake OpenClaw IPC boundary without
installing or invoking OpenClaw:

1. re-audit TASK-016 closure and confirm the slice still serves MVP-1;
2. define the only normal typed gateway actions for task submission, status,
   approval routing, and stop;
3. keep the CLI as a recovery/test client over the same contract, not another
   normal gateway;
4. ensure IPC grants no direct PostgreSQL/Git/provider/credential/protected-
   release authority and cannot own a Codex writer thread;
5. update SPEC/ADR/module constitution/ticket before implementation;
6. repeat TDD, focused/full verification, independent reviews, integration
   report, ledger, and handoff.

## Restart Context

- Overall goal remains active through MVP-3; do not mark it complete.
- Current MVP: MVP-1 offline control core.
- Historical next marker at TASK-016 closure: `CURRENT TASK-017 GOVERNANCE`.
- Completed ticket: `docs/tickets/TASK-016-artifact-store.md`.
- First checks:
  - `git status --short --branch`
  - `git rev-parse HEAD`
  - `cargo test --workspace --all-targets --all-features --locked`
  - `npm.cmd run verify`
- Continue bounded, reversible local work automatically.
- Do not introduce the unrelated playmate website.
