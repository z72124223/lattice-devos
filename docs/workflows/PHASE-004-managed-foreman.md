# Workflow Ledger - Phase 4 managed foreman

## Request

- Classification: high-risk persistence, process lifecycle, and module-boundary change
- Repository: LATTICE DevOS
- Branch: `product/lattice-control-mvp`
- Baseline and current HEAD: `f2524cfa7d095febfc162892119c166259c13fbe`
- Delivery state: `NEEDS_REVIEW`; no commit, push, merge, deploy, release, or global install

## Stage Status

| Stage | Status | Evidence | Gate strength |
|---|---|---|---|
| Repository inspection | PASS | Baseline verified clean before implementation; unrelated/shared changes preserved | machine-enforced |
| Specification and ownership | PASS | `SPEC-011` v1.8, ADR-028, Task Ledger 3.2, Ports 2.6, PostgreSQL Foreman 1.8 | documented plus tests |
| Implementation | PASS | Existing Orchestrator, Task Ledger, writer lease, Approval Verifier, Artifact Store, Codex adapter, and PostgreSQL repository form one managed loop | machine-enforced |
| Focused verification | PASS | Runtime 415/415, Task Ledger 77/77, Orchestrator 40/40, Ports 11/11; approval 38/38; PostgreSQL foreman 46/46 with 10 live-only ignored | machine-enforced |
| PostgreSQL live replay | PASS | run `14846d3b8fd24addb354df472bfe1744`, 10/10 stages; closure run `837b382168a741f5838c41e31ca5d1cd`, 5/5 and provider dispatch 0 to 0 | live disposable profile |
| Scripted active restart | PASS | same task/attempt/thread/turn; one start; reconnect/resume; no duplicate Agent; exact process cleanup | live scripted App Server |
| Full verification | PASS | fmt, diff check, cargo check, cargo workspace test, release build, and `npm.cmd run verify` | machine-enforced |
| Independent review | PASS | final code/architecture/security reviews found no P0-P3 issue; Approval-owner review independence limitation disclosed | independent agents |
| Real Codex happy path | BLOCKED | exact real turn started and completed, but Windows sandbox denied the requested file edit; verifier correctly persisted BLOCKED instead of success | live real provider |
| Local commit | NOT RUN | prohibited by the request when any live gate fails | delivery gate |

## Implemented Product Contract

- A server-owned durable Foreman identity, generation, checkpoint, task/project
  identity, writer fence, and worker attempt are replay-bound in PostgreSQL.
- Dependency-ready work is atomically reserved and claimed with global capacity
  four, per-task limits, immutable budgets, duplicate prevention, and the
  GPT-5.6 Luna/Terra/Sol allowlist.
- RPC acceptance is not execution. Only the exact `turn/started` notification
  marks a worker active; timeout, EOF, disconnect, and close-timeout paths fail
  closed and reconcile before any replacement.
- Stalls, exact interrupt/terminal, no-provider-effect closure, retry lineage,
  model routing, worktree identity, and process-subtree cleanup are typed and
  tamper-evident. The default repair retry limit remains two.
- Verification uses captured project rules and closed command identities, not
  shell text from an objective. Artifact ingress recomputes content and
  descriptor digests and scans admitted bytes for sensitive material.
- Runtime cannot self-attest `VERIFIED_APPROVAL`. Only the reversible closed-
  policy local lane is enabled until a separate Approval-owner connector/role
  exists. Merge, push, deploy, publish, payment, messages, and permanent delete
  remain separate authorities.
- Public task status is a PostgreSQL-derived projection with phase, exact worker
  state, attempt, progress time, blocker, verification, evidence digest, next
  action, and cumulative resource observations.

## Verification Evidence

| Command or live check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS; only pre-existing line-ending warnings |
| `cargo check --workspace --all-targets --locked --offline` | PASS; two existing test-only dead-code warnings |
| `cargo test --workspace --all-targets --locked --offline` | PASS, exit 0 |
| `cargo build -p lattice-runtime --release --locked --offline` | PASS |
| `npm.cmd run verify` with pinned Rust 1.97.1 | PASS; Project Check PASS, Control 160/160, Node 117 pass / 0 fail / 1 skip |
| `test-phase4-managed-foreman.ps1 -StaticSelfTestOnly` | PASS |
| `test-phase4-managed-foreman.ps1 -ScriptedActiveRestart -KeepArtifacts -SkipBuild` | PASS in 62.948 s, zero real model calls |
| Disposable PostgreSQL live suite | PASS, run `14846d3b8fd24addb354df472bfe1744`, 10/10 |
| Repository/outbox crash-window suite | PASS, run `837b382168a741f5838c41e31ca5d1cd`, 5/5 |

The first unpinned `npm.cmd run verify` invocation stopped at Project Check
because the machine default stable toolchain lacked the required cargo
component. The pinned repository toolchain rerun passed; the initial environment
failure remains part of the evidence.

## Real Codex Gate

- task_ref: `83776f5e194c1b0babe4c86e4b80989321c71e14a4906278b6943032649b03a6`
- thread: `01a045cb-296f-70f1-a2ca-eca79bf28330`
- turn: `01a045cb-2d7c-7212-9fe4-0a9d666ecbf7`
- attempt: 1; product retry count: 0
- model: `gpt-5.6-terra`, medium reasoning
- result: exact provider completion observed, but `phase4-proof.txt` was not
  created because the Windows sandbox rejected the write; Git remained
  unchanged and verification persisted `BLOCKED`.
- evidence digest:
  `9dd8bd6ea0c24eeda71bbd54c215dd6eb726492561e3fa4263e18d8829c8f5b3`
- usage: 11,560 total tokens (11,353 input, 1,792 cached, 207 output,
  23 reasoning), one model call; monetary cost unavailable.
- retained evidence root:
  `C:\Users\f7212\AppData\Local\Temp\lattice-phase4-managed-foreman-e09e9f4b141643f41d3273e4397ab26a`

A third real-model attempt was intentionally not started: model-free probes
reproduced the same Windows sandbox failure across the bounded supported
configurations. Full-access execution was not used.

## Restart Evidence

- Scripted task_ref:
  `200a78c57bdd0f74068387fbf8af1eb44e12e14f3412e5a3998d1a1374960b44`
- attempt 1, retry 0, Terra; exact start and active reconciliation both true.
- provider thread starts: 1; provider turn starts: 1; dispatch counts after
  restart: 1/1; `no_duplicate_agent=true`.
- process handoff retained attempt/fence and replaced only process identity.
- checkpoint generation/digest remained exactly 1 /
  `d656288cdc7c06170bcf1e0b22bdfc20d36e5addf65155c28862b618230e386d`.
- retained evidence root:
  `C:\Users\f7212\AppData\Local\Temp\lattice-phase4-managed-foreman-28e488533b281f9ae0b4c921320c3c17`.

## Completion Boundary

- The worktree implementation and deterministic/live PostgreSQL gates are
  complete and independently reviewed.
- The mandatory real Codex edit-and-verify happy path is not complete; therefore
  Phase 4 is not accepted, the worktree remains intentionally uncommitted, and
  HEAD remains the Phase 3 baseline.
- The global LATTICE MCP/install was not switched, so this worktree functionality
  must not be described as globally active.
- Next action: restore a functioning supported Windows sandbox, rerun exactly one
  real Codex happy path with the existing bounded harness, then rerun the final
  verification and create the local commit only if that live gate passes.

## WSL2 continuation evidence - 2026-08-29

This section appends the WSL2 continuation without changing the immutable
Windows attempt above. The LATTICE MCP returned receipt-only runtime state and
`LATTICE_TASK_ADMISSION_MISSING`; this continuation therefore does not claim a
durable LATTICE task identity.

### Durable execution environment and same-domain verifier

- PostgreSQL typed execution-environment schema, exact replay, descriptor/ref
  substitution rejection, and fresh-process reconstruction passed in the
  marker-owned PostgreSQL 17 run `d5333100775d4288be03b940d658cdad`.
- Repository/outbox fresh-process run
  `da6ce4cc9e654c7eafca101ea1ab99aa` passed 6/6 with provider effects 0 before
  and after restart.
- Node verification passed 165 total (154 pass, 11 skip); the focused Control
  set passed 46/46. Root `npm.cmd run verify` passed Control 247 total
  (236 pass, 11 skip) and root 118 total (117 pass, 1 skip).
- Official Codex CLI 0.146.0 authentication was completed only inside the
  isolated Linux home/keyring. No Windows keyring, password, token, or secret
  was copied into the WSL execution domain.

### Bounded WSL2 full attempts and zero-effect truth

- Full run `8f663473fd6b0edbe4d3831961964446` stopped before provider dispatch at
  `LATTICE_MANAGED_GIT_OBSERVATION_REJECTED`.
- The second and final permitted full run
  `18413508445f396aa50ac2ecc1e925d4`, task
  `d1ded8a5db6c938494eb47bf363ba02755893e985702f9b52de77da24bc99bb5`,
  stopped at `WSL2_ACTIVE_ACCEPTED_START` with the same code. Its binary digest
  was `ee3502ab642fcf40c7168a53ce3945fe669e07195bebcef3c8df896fa35eecc2`.
- Fresh marker-owned PostgreSQL inspection after the second failure proved zero
  promotions, attempts, pending claims, execution environments, provider
  dispatches, observations, worker/reviewer thread or turn dispatches, outbox
  rows, closures, and writer heads. The real Codex/provider attempt was not
  started and the immutable Windows attempt above was not rewritten.
- Repository rule `AGENTS.md` stops retries after two failed attempts at the
  same acceptance. A third full run is therefore prohibited even though both
  WSL2 failures occurred before provider dispatch.

### Final Git observation repair and post-repair proof

- The closed managed Git command now supplies one exact task-owned
  `safe.directory`, keeps system/global configuration disabled, and retains the
  existing credential/helper/hook/environment closure.
- Windows canonicalization converts a WSL worktree into a verbatim UNC path.
  The managed Git child path now normalizes only
  `\\?\UNC\wsl.localhost\...` to `\\wsl.localhost\...`; arbitrary UNC hosts,
  lookalike hosts, and an empty distro remain fail closed.
- Exact focused tests passed 2/2. `cargo fmt --all -- --check` and
  `git diff --check` passed. Full `lattice-runtime` library tests passed 470,
  failed 0, ignored 11. The repaired release binary is 16,845,312 bytes with
  SHA-256 `82d4cca61eb09499dfb450ecd50a76a9a104371ca96b6bf20f08cfd847b8e2c0`.
- Fresh zero-model WSL2 technical preflight task
  `2b52ad794052cc7f0a7ec1eebdedaf67468819785a9e5c49d1702cc07e289bb6`
  passed. It exercised the Linux-home source and managed worktree, canonical
  Git observation, npm/Cargo/Rust/Git verifier toolchain, process fence,
  descriptor replay, and every substitution gate. Provider effects, provider
  thread/turn starts, outbox rows, and pending worker claims remained 0.
- An independent read-only code, architecture, and security review of the final
  managed Git patch reported `NO_P0_P2`, including its project-bridge
  consistency, exact UNC host restriction, `safe.directory`, and closed
  environment boundaries.
- The post-repair full real-provider acceptance is `NOT RUN`, not `FAIL`, due
  to the two-attempt circuit breaker. Phase 4 remains `NEEDS_REVIEW`; no local
  commit, push, merge, deploy, release, dashboard refresh, or archive claim is
  authorized.

### Retained post-repair evidence

- WSL evidence directory:
  `\\wsl.localhost\Ubuntu\home\zk\lattice-phase4-wsl2-acceptance-20260828\verifier-state\acceptance-2b52ad794052cc7f\evidence`
- Marker-owned Windows root:
  `C:\Users\f7212\AppData\Local\Temp\lattice-phase4-managed-foreman-9741cb859b27219bb11fcd3ce9019cc4`
- Exact next action: in a fresh authorized acceptance window, run one bounded
  full WSL2 live acceptance using the repaired binary or a byte-identical
  rebuild. Commit only if that live gate and the final full verification pass.

## Fresh WSL2 live-acceptance window - 2026-08-29

This section is append-only. It records the separately authorized fresh window
and does not rewrite the Windows attempt or either earlier WSL2 failure above.
The LATTICE MCP remained receipt-only and returned no durable admission for this
Codex continuation, so this section does not claim a LATTICE task identity for
the construction window itself.

### Pre-dispatch verification

- Windows free-memory, stopped-WSL, process-ownership, retained Linux-home root,
  isolated credential/keyring, and exact binary checks passed before Ubuntu was
  started. No Docker, global LATTICE service, or unrelated PostgreSQL process
  was stopped.
- The pinned Rust 1.97.1 suite passed 472 tests, failed 0, ignored 11. The full
  Node suite passed 241, failed 0, skipped 11. PowerShell parsing, the Phase 4
  hardening suite, exact Job Object argument/output probes, WSL command-unit
  probes, `cargo fmt`, and `git diff --check` passed.
- The byte-identical release binary used by both the technical preflight and
  the live run was 16,854,528 bytes with SHA-256
  `b5f0152870eea57c64530e243b8166028c11344cf67ed87b0b06476d400df675`.
- Fresh same-domain zero-model technical preflight task
  `b95ae3fb24b2fbb86be5363aa8cd9b61c9a1b4d37b86dedc7e249e0bd4b55643`
  passed in 117,134 ms. Provider, thread, turn, outbox, and pending-claim counts
  were all 0 and every command/process fence was closed. Its marker-owned root
  was
  `C:\Users\f7212\AppData\Local\Temp\lattice-phase4-managed-foreman-eac43336e2e502908639500e775a595f`.

### Sole authorized full live run

- Command: one bounded `test-phase4-managed-foreman.ps1 -Wsl2LinuxLive
  -SkipBuild -KeepArtifacts` invocation using the release binary above.
- Result: `FAIL`; `acceptance=false`; receipt stage
  `WSL2_ACTIVE_ACCEPTED_START`; receipt code
  `PHASE4_WSL2_ACTIVE_START_FAILED`; receipt line 7320. This is not a PASS and no
  commit or downstream acceptance gate was authorized.
- Fresh task_ref:
  `59e83b7be1eb169b730fc9d83c6db8c5789be1838f36d22911bc2f4c82ef6684`.
  Final execution-environment ref:
  `execution-environment:sha256:55ddec2d7f6d48190cdb1c5b7dfd0a7666190ffb0e789d55cf14ea472d2a4701`.
- The harness marked the real-dispatch boundary entered, but PostgreSQL and
  systemd evidence prove that no durable attempt or provider unit existed:
  provider effects 0, worker/reviewer thread and turn claims 0, attempts 0,
  observations 0, outbox rows 0, pending claims 0, and model calls 0. Model,
  reasoning, token usage, and monetary cost are therefore unavailable/null,
  not estimated.
- The failure cleanup was CLOSED and passed: task-owned WSL units 0, active
  command units 0, marker-owned PostgreSQL stopped, loopback listeners absent,
  and the foreman process tree stopped. No credential content was read, copied,
  printed, or placed in evidence.

### Durable race reconstruction and repair

- The successor stream
  `4e9b2643799e7597f301bf8c6a4df7373e5c4f441b813172f324d9131734a70a`
  contains exactly five events: task creation, autonomy receipt, transition to
  `AWAITING_EXECUTION_APPROVAL`, task-execution binding, and approval evidence.
  The task-execution binding was recorded at
  `2026-08-29T07:41:31.988966Z`; approval evidence followed at
  `2026-08-29T07:41:32.361565Z`, a 373 ms durable promotion/approval window.
- During that exact window, the zero-attempt promoted status path returns
  `BLOCKED` with `LATTICE_MANAGED_EXECUTION_APPROVAL_REQUIRED`. The former
  harness treated the first such ACTIVE poll as terminal and stopped the
  process before task-specific Writer acquire or attempt reservation. This
  explains the otherwise contradictory valid approval row with no task Writer,
  attempt, or provider effect. The original receipt did not retain the failing
  candidate, so the receipt code remains immutable; the product code plus exact
  PostgreSQL event ordering independently reproduce the lost status code.
- The harness now retains every ACTIVE candidate before evaluation. Only the
  exact `BLOCKED` + `AWAITING_EXECUTION_APPROVAL` + no attempt/worker/thread/turn
  shape is treated as transient inside the existing bounded start window.
  Every other blocker fails immediately; if the approval gate does not clear by
  the deadline, its exact product failure code is surfaced instead of a generic
  restart-window code. Poll timing is derived from the session's remaining
  fixed 56-call MCP budget and distributed across the full 120-second window,
  so the call ceiling cannot truncate the advertised deadline.
- `Invoke-Phase4Psql` now pins `PGCLIENTENCODING=UTF8`, `LANG=C`, and `LC_ALL=C`
  and uses strict UTF-8 output decoding. OEM decoding remains limited to the
  Windows-native `initdb`, `pg_ctl`, and `netstat` diagnostics.
- Post-repair verification passed: PowerShell AST, hardening 3/3, executable
  static self-check, a bounded failed-connection psql Job Object probe
  (`exit=2`, `active=0`), and `git diff --check`. The repaired harness SHA-256 is
  `3048f8d94e51e97950976d54483855a82bed23961b3a0d7ee2aaffea860eba10`;
  the hardening test SHA-256 is
  `047c1ea6fd33ca50f66969b4cea2361bdf93c6d3542bede942f7babe08de04ae`.
- No second real Codex attempt was started. The one-attempt fresh-window
  authority is exhausted; validating the repaired ACTIVE race requires another
  explicit fresh live-acceptance authority and new task/thread/turn/attempt
  identity.

### Retained evidence and delivery boundary

- Retained Windows evidence root:
  `C:\Users\f7212\AppData\Local\Temp\lattice-phase4-managed-foreman-335beccd1b9f019cc6971a8fe240f758`.
  Owner marker SHA-256:
  `fb8b1c7f1fef0ee43ab7eac77f5c56145099bb84e313242c7ca5d2b672dc1d52`.
- Retained WSL evidence:
  `\\wsl.localhost\Ubuntu\home\zk\lattice-phase4-wsl2-acceptance-20260828\verifier-state\acceptance-59e83b7be1eb169b\evidence`.
  Its zero-model preflight evidence SHA-256 is
  `b6c1983130a73484c8ad4c242cd89d47c3dd6ee710e88db06e18924b93782298`.
- The managed worktree remained clean at its retained task root when queried
  with the exact `safe.directory`; source and failed-attempt evidence remain
  immutable. A separate diagnostic copy of PostgreSQL was used for bounded
  single-user read-only reconstruction and is not an acceptance authority.
- Final Node/Rust/PostgreSQL/restart/reconnect/outbox live gates and the local
  feature commit are `NOT RUN` because the mandatory live acceptance failed.
  No reset, clean, commit, push, merge, deploy, release, dashboard/archive claim,
  global install, or credential mutation occurred.

### Evidence binding and post-review hardening addendum

- The exact retained Linux-side evidence for zero-model task
  `b95ae3fb24b2fbb86be5363aa8cd9b61c9a1b4d37b86dedc7e249e0bd4b55643`
  is
  `\\wsl.localhost\Ubuntu\home\zk\lattice-phase4-wsl2-acceptance-20260828\verifier-state\acceptance-b95ae3fb24b2fbb8\evidence\zero-model-preflight.json`.
  Its SHA-256 is
  `a8b53d203a9d0ed7e47eda79dc6ed9031a8b2a466a3a6464f2364fb89829a4ad`.
  This is the exact binding for the 117,134 ms PASS above; the later
  `b6c198...` file remains the separate internal preflight for full task
  `59e83b...` and is not substituted for it.
- Independent script and architecture reviews then found two bounded-polling
  defects in the first race repair: an ACTIVE deadline fractional-millisecond
  off-by-one could request a 57th MCP call, and reconnect could consume calls
  needed by the subsequent terminal poll. Neither defect was exercised by a
  second provider run.
- ACTIVE polling now has explicit deadline, per-call millisecond timeout,
  late-response, and captured-call-count guards. ACTIVE, reconnect, and
  terminal polling use absolute schedules that retain their final observation
  until one second before the deadline instead of exhausting calls and leaving
  an unobserved tail. Reconnect uses at most the eight calls not reserved from
  the 56-call session budget and preserves all 48 terminal-status calls. Static
  math checks and hardening assertions cover all three windows and the shared
  budget.
- Both initial ACTIVE acceptance and reconnect now run the complete strict
  managed-status assertion before consulting PostgreSQL. Reconnect additionally
  binds candidate and durable thread/turn identities, Writer ACTIVE status,
  attempt/fence, the new foreman PID, and the exact foreman checkpoint before
  it can satisfy the restart gate. Initial ACTIVE also binds its generation and
  checkpoint digest to the exact formal checkpoint before the hard restart.
- On the resulting freeze, PowerShell AST, hardening 3/3, executable static
  self-check, and `git diff --check` pass. Harness SHA-256:
  `34d037f32ab27edeb0dee9a17814fee9fc2aac4211aabb834160f8606dccd29e`.
  Hardening-test SHA-256:
  `afea835de6a2161a0bf59171ee175ba12d5625368c4b108fb6f936abf419c683`.
  These review-driven repairs remain `NOT RUN` against a real model because
  the sole authorized attempt was already consumed; no commit was created.
- Independent final script and code/architecture/security reviews against
  those exact hashes reported `NO_P0_P2`. They independently confirmed the
  deadline/call-budget math, late-response rejection, strict ACTIVE/reconnect
  status and durable Writer identity, new foreman PID, and exact checkpoint
  bindings. Heavy/live execution was intentionally `NOT RUN` by the reviewers.
- Final task-owned resource cleanup found no Phase 4 systemd units, owned
  processes, or listeners on ports 56328/56329. The disposable single-user
  PostgreSQL diagnostic copy was moved to the Windows Recycle Bin (recoverable),
  the original failed-run evidence root remains present, and Ubuntu was stopped
  after it contained only idle system services. Docker and global LATTICE or
  PostgreSQL services were not touched.

## Post-repair WSL2 full acceptance - 2026-08-30

This section is append-only. It records a new, explicitly authorized WSL2
attempt and does not rewrite or replace any prior Windows/WSL failure above.
The LATTICE MCP again supplied receipt-only runtime state and no durable task
admission for this Codex continuation; marker-owned PostgreSQL and retained
evidence are the durable live truth below.

### Login and zero-model gate

- Official Codex device login completed in the isolated Linux credential
  home/keyring. No Windows keyring or secret was copied or emitted.
- Fresh technical-preflight task
  `2f4d9ac8dbda0d0fdc81b0b4c870c43dbbb5ba3328b7fad0bd4f7731ea5506f0`
  passed. Provider, thread, turn, outbox, and pending-claim counts were all 0;
  process-fence and cleanup checks passed. Its WSL evidence is
  `\\wsl.localhost\Ubuntu\home\zk\lattice-phase4-wsl2-acceptance-20260830\verifier-state\acceptance-2f4d9ac8dbda0d0f\evidence`.
- The release binary remained 16,854,528 bytes with SHA-256
  `b5f0152870eea57c64530e243b8166028c11344cf67ed87b0b06476d400df675`.

### Sole authorized real attempt

- Exactly one bounded `-Wsl2LinuxLive -SkipBuild -KeepArtifacts` run started at
  `2026-08-29T16:32:37.364Z`. No second real attempt was started.
- Task
  `db30e0c8da8b2ff0b29040451ff837850bf7240332d2a06efdd257be123ced52`;
  execution-environment ref
  `execution-environment:sha256:246f189434240ad93da39ce6040d7489889bf77969a0baa34be7bd758beb40c1`.
- Result: `FAIL`, `acceptance=false`, stage
  `WSL2_ACTIVE_ACCEPTED_START`, code `PHASE4_MCP_RESPONSE_TIMEOUT`, and
  `real_codex_attempt_evidence=ACTIVE_RUNTIME_DISPATCH_BOUNDARY_ENTERED`.
- The last completed status was `AWAITING_EXECUTION_APPROVAL`/`SUBMITTED`, with
  null attempt/model/thread/turn and `worker_running=false`. Durable provider
  effects, dispatches, attempts, model calls, worker observations, execution
  environments, staged artifacts, thread/turn, pending claims, and outbox were
  all 0. Actual token usage and cost are null/unknown, not reported as zero.

### Durable ordering and diagnosis

- Promotion was recorded at `2026-08-29T16:34:52.925035Z`; exact bound approval
  evidence followed at `2026-08-29T16:34:53.329538Z`, 404.503 ms later.
  Approval remained durable for about 115.6 seconds before status timed out.
  This run therefore did not reproduce the prior approval race.
- The successor ledger contains only task creation, autonomy receipt,
  awaiting-approval transition, execution binding, and approval (sequences
  1-5); no PREPARING or EXECUTING event exists.
- The exact inner product status stall has no retained telemetry. It is not
  labeled a deadlock and is not claimed fixed. A contained diagnostic clone is
  retained at
  `C:\Users\f7212\AppData\Local\Temp\lattice-phase4-diagnostic-db-a80d966e0459aa947e6df4deaee5015f`;
  the original run root was not started or mutated during diagnosis.

### Post-failure zero-provider hardening

- ACTIVE now uses the product stage bound (480 seconds with current defaults);
  ACTIVE, reconnect, and terminal status RPCs use each stage's remaining time.
  Timeout diagnostics bind stage/request/poll/elapsed/remaining window, both
  configured and effective response time, and the last completed candidate.
- A timed-out stdio session is marked contaminated. Late response or reuse is
  rejected before another tool call.
- Future failure receipts are create-new atomic publishes inside the owned root
  with a SHA-256 sidecar. Overwrite is rejected; sidecar failure preserves the
  truthful receipt. This does not backfill the immutable `db30e0c8...` roots.
- Credential watching protects CODEX_HOME identity, `config.toml`, and
  `auth.json` while allowing ordinary Codex state. Provider `HOME` is the
  isolated CODEX_HOME; verification retains its separate home/runtime path.
- Focused Node: 64 total, 52 pass, 0 fail, 12 Linux-only skip. PowerShell
  static/self-check, delayed/hung/late MCP behavior, receipt persistence, and
  `git diff --check` passed. Linux supervisor tests passed 23/26, 0 failed,
  3 skipped.
- Exact current SHA-256: harness
  `7c26fab2564924896002a7e5080406d485581c58f859342bc69cdb49c85a5fda`;
  hardening test
  `214f208f0f25d4d8ea70ed42070d402ad4dfaf35d393981f75c67eecb76435ac`;
  supervisor
  `31036ffff5763479c2d64fb04d52efef0793564bf069b5c900d96e68ed88ccb3`;
  supervisor test
  `5ca8085259258e09d3868865191cf626040604998cb362fb29ce73d6bd82c0b3`;
  execution-domain source
  `de74d9471731461d377059c35c64aaa888d89ad8764f9353c682db89ace62ba1`;
  execution-preflight test
  `456285339218348c67f47aa856aa34012783eea366e4cb877260f3f00a84dae1`.
- Independent exact-hash code/architecture/security review: `NO_P0_P2`.
  This closes harness/evidence findings, not the product-side status stall.

### Evidence, cleanup, and delivery boundary

- Windows root
  `C:\Users\f7212\AppData\Local\Temp\lattice-phase4-managed-foreman-a80d966e0459aa947e6df4deaee5015f`;
  manifest `52a9576fd98f52ce6576d484824664b2d16b04a3cb59821b4f9580a2b1a67432`
  (1,517 files; 71,968,978 bytes).
- WSL evidence
  `\\wsl.localhost\Ubuntu\home\zk\lattice-phase4-wsl2-acceptance-20260830\verifier-state\acceptance-db30e0c8da8b2ff0\evidence`;
  manifest `7675c62c3532175d8652d192c9b21477feffd04d5f79ad406f6768786bd1c9fe`
  (5 files; 47,213 bytes).
- Those immutable roots contain no final receipt. The complete receipt is in
  `C:\Users\f7212\.codex\sessions\2026\08\29\rollout-2026-08-29T23-10-07-01a04e12-267e-7072-b5a3-13e536c0f84a.jsonl:1644`.
- Historical manifests remain unchanged: Windows `335beccd...` is
  `7bbf96512eb5d6e8bc132fc8a021832d47f5621bdbd22f0cbc97d7febfea7f7f`;
  WSL `59e83...` is
  `eb3e236d0127186d705242ab1dbea18eb13e6ae1bfa8809cf09211b45e41a716`.
- Cleanup passed: marker PostgreSQL stopped, pid file absent, 64222/64223
  listeners absent, owned process trees and task units/unit files absent, and
  Writer current lease empty. Ubuntu was not terminated because an interactive
  user shell was present; unrelated services/processes were not touched.
- Restart/reconnect/outbox and final full Node/Rust/PostgreSQL/live gates are
  `NOT RUN`; the mandatory live gate failed. No reset, clean, commit, push,
  merge, deploy, release, dashboard/archive claim, or global install occurred.
  Any further real attempt requires fresh explicit authority and a new
  task/thread/turn/attempt identity.
