# HANDOFF

## Status

`NEEDS_REVIEW`

The 2026-08-30 fresh WSL2 window completed its official isolated Codex login and
zero-model preflight, then consumed its sole authorized real attempt. That live
run failed before provider dispatch at `WSL2_ACTIVE_ACCEPTED_START` because the
status RPC exceeded the harness response deadline. PostgreSQL proves approval
was already durable for about 115.6 seconds, so this is not the earlier approval
race. The exact product-side status stall remains unlocalized. Post-failure
zero-provider hardening passes, but no additional real attempt is authorized and no
commit was created.

## Objective and scope

- Runtime 工作真相：LATTICE／PostgreSQL；Codex task 只承載推理與施工現場，
  不取代 durable task、attempt、receipt 或 execution-environment authority。
- 程式交付真相：GitHub 提交、PR 與 CI；本窗口未取得 push、merge、deploy 或
  release 授權。
- Connect general durable tasks to the existing LATTICE Foreman, Orchestrator,
  Task Ledger, writer lease, Approval Verifier, Codex connector, verifier,
  Artifact Store, and PostgreSQL truth.
- Provide bounded dispatch, exact-start observation, stall/reconcile/retry,
  independent verification, durable evidence, public status, and restart replay.
- Keep npm, Cargo/Rust, sandbox, Git, process fence, credentials, repository,
  and provider in one isolated Linux-home execution domain.
- Keep merge, push, deploy, publish, payment, external messages, permanent
  deletion, SaaS, accounts, billing, and UI work out of scope.

## Completed work

- Exact provider lifecycle and cwd identity; reconcile-required timeout/EOF;
  duplicate-child prevention after close timeout.
- Durable Foreman generation/checkpoint, atomic capacity-four claim, exact
  attempt/fence identity, model allowlist/routing, bounded budgets and retry.
- Typed no-provider-effect predecessor and exact terminal lineage for safe
  higher-fence repair without inventing provider completion.
- Approval-owner append-only execution binding and runtime self-attestation
  denial; reversible closed-policy local work remains the only enabled runtime
  execution lane until an owner connector exists.
- Canonical Artifact Store ingress, content/descriptor digest recomputation,
  sensitive-byte scan, quota/replay enforcement, and PostgreSQL-derived status.
- Exact process-root terminal receipt, subtree reap, independent project-rule
  verification, Git scope checks, and review/evidence projection.
- Durable typed WSL2 execution-environment persistence, exact replay,
  fresh-process reconstruction, and fail-closed descriptor/ref substitution.
- Official Codex CLI 0.146.0 login only in the isolated Linux home/keyring; no
  Windows keyring, password, token, or secret was copied or displayed.
- Exact task-owned Git `safe.directory` with system/global config, hooks,
  credential helpers, and ambient environment still disabled.
- Canonical WSL child paths normalize only `\\?\UNC\wsl.localhost\...` to
  `\\wsl.localhost\...`; arbitrary/lookalike UNC hosts and an empty distro
  remain fail closed.
- Windows child processes are created suspended inside an exact Job Object;
  arguments, environment, stdout/stderr bounds, stdin timeout, process identity,
  PostgreSQL ownership, and tree cleanup are independently checked.
- PostgreSQL JSON is decoded as strict UTF-8 with explicit client encoding and
  C locale; OEM decoding is limited to Windows-native diagnostic tools.
- ACTIVE polling retains every status and permits only the exact zero-attempt
  approval gate to remain transient. ACTIVE, reconnect, and terminal polling
  use explicit call-count, millisecond deadline/timeout, late-response, and
  absolute-schedule guards. The ACTIVE outer window now follows the product's
  bounded stage contract (480 seconds with the current defaults), and every
  status RPC uses that stage's remaining time. A timed-out stdio session is
  contaminated and cannot issue another tool call.
- Initial ACTIVE and reconnect success both require the complete strict managed
  status shape. Reconnect also binds candidate/durable thread and turn, Writer
  ACTIVE attempt/fence, the new foreman PID, and exact checkpoint identity;
  initial ACTIVE binds the exact formal generation/checkpoint before restart.
- Credential watchers now distinguish protected `config.toml`/`auth.json` and
  CODEX_HOME identity drift from ordinary Codex session-state activity. Provider
  `HOME` is the isolated Linux CODEX_HOME, while non-provider verification keeps
  its separate home and post-exit runtime directory.
- Future failure receipts are create-new, atomically published inside the owned
  run root, digest-sidecar bound, and overwrite resistant. The consumed run's
  immutable roots are not retroactively modified; its final receipt remains in
  the Codex session JSONL only.

## Verification

- `cargo fmt --all -- --check`: PASS, exit 0 with pinned Rust 1.97.1.
- `git diff --check`: PASS, exit 0 (existing line-ending warnings only).
- Exact managed Git regression tests: PASS, 2/2.
- `cargo test -p lattice-runtime --lib --locked`: PASS, 472 passed,
  0 failed, 11 ignored, exit 0.
- Release build: PASS, exit 0; `target/release/latticed.exe` is 16,854,528
  bytes, SHA-256
  `b5f0152870eea57c64530e243b8166028c11344cf67ed87b0b06476d400df675`.
- Marker-owned PostgreSQL typed-environment run
  `d5333100775d4288be03b940d658cdad`: PASS.
- Fresh-process repository/outbox run
  `da6ce4cc9e654c7eafca101ea1ab99aa`: PASS, 6/6, provider effects 0 before/after.
- Latest full Node verification: 252 total, 241 pass, 0 fail, 11 skip.
- Official Codex device login in the isolated Linux credential/keyring domain:
  PASS; no credential was copied to or from Windows and no secret was emitted.
- Fresh zero-model WSL2 technical preflight task
  `2f4d9ac8dbda0d0fdc81b0b4c870c43dbbb5ba3328b7fad0bd4f7731ea5506f0`:
  PASS; provider/thread/turn/outbox/pending-claim counts all 0 and process-fence
  cleanup PASS.
- This window's sole full live task
  `db30e0c8da8b2ff0b29040451ff837850bf7240332d2a06efdd257be123ced52`:
  FAIL before provider dispatch with stage `WSL2_ACTIVE_ACCEPTED_START` and code
  `PHASE4_MCP_RESPONSE_TIMEOUT`. Durable provider effects, attempts, model calls,
  thread/turn rows, pending claims, and outbox rows are all 0. Actual token and
  monetary cost fields are null/unknown, not asserted as zero.
- Post-failure focused Node verification: 64 total, 52 pass, 0 fail, 12 Linux-only
  skip. PowerShell static parser/self-check, MCP delayed/hung/late-response
  behavioral self-test, create-new receipt-persistence behavioral self-test, and
  `git diff --check`: PASS.
- Current harness SHA-256:
  `7c26fab2564924896002a7e5080406d485581c58f859342bc69cdb49c85a5fda`.
  Current hardening-test SHA-256:
  `214f208f0f25d4d8ea70ed42070d402ad4dfaf35d393981f75c67eecb76435ac`.
  Supervisor/source hashes are `31036ffff5763479c2d64fb04d52efef0793564bf069b5c900d96e68ed88ccb3`
  and `de74d9471731461d377059c35c64aaa888d89ad8764f9353c682db89ace62ba1`.
- Final owned-resource check: PASS; no task-owned WSL units/unit files or
  listeners on 64222/64223. Ubuntu was not forcibly terminated because an
  interactive user shell was present; no unrelated process was stopped.
- Independent final code/architecture/security review on the exact current
  hashes: `NO_P0_P2`; reviewer heavy/live execution: NOT RUN.
- CI, restart/reconnect/outbox final live gates, full post-live Rust/Node/
  PostgreSQL gates, and another real-provider acceptance: NOT RUN.

## Durable evidence

The complete append-only Phase 4 evidence ledger, including the immutable
blocked Windows attempt and its exact task/thread/turn, digest, usage, retained
artifact locations, and SQL identities, remains in
`docs/workflows/PHASE-004-managed-foreman.md`. This public handoff does not
rewrite or supersede that evidence.

## Files changed and workflow state

The shared Phase 4 worktree contains 137 tracked/untracked entries. They are
intentionally preserved; `git status --short` is the exhaustive manifest.

| Area | Purpose | Current gate |
|---|---|---|
| `apps/lattice-runtime/src/managed_foreman_service.rs` | Closed Git command, WSL path normalization, regression tests | PASS: focused/full Rust and zero-model WSL2 |
| Runtime/Control WSL2 execution-domain files | Toolchain, credentials, mapping, process fence, verifier and reconnect evidence | PASS: Node and zero-model WSL2 |
| PostgreSQL Foreman extension and adapters | Durable typed environment truth and fresh-process replay | PASS: marker-owned PostgreSQL 17 |
| Phase 4 acceptance scripts | Bounded technical/full acceptance, exact Job ownership, status timeout handling, retained evidence | current technical PASS; sole authorized full FAIL; post-hardening full NOT RUN |
| `docs/workflows/PHASE-004-managed-foreman.md` | Append-only detailed evidence | current |

The earlier runs `8f663473fd6b0edbe4d3831961964446` and
`18413508445f396aa50ac2ecc1e925d4` remain immutable pre-fix Git-observation
failures. Historical run
`59e83b7be1eb169b730fc9d83c6db8c5789be1838f36d22911bc2f4c82ef6684`
also remains immutable: its first ACTIVE status poll hit the 373 ms durable
promotion/approval window and the old harness treated the exact zero-attempt
approval gate as terminal. The new `db30e0c8...` run passed that race: approval
was durable only 404.503 ms after promotion and remained durable for about
115.6 seconds before the status RPC timed out. The product-side location of that
status stall has no retained telemetry and is not claimed as fixed.

## Review and risks

- The `db30e0c8...` immutable retained roots predate the new final-receipt writer
  and contain no final receipt. Its complete FAIL/stage/code/cleanup receipt is
  retained in the current Codex session JSONL; future runs use the new
  create-new, digest-bound receipt path.
- The harness timeout-contract and evidence defects are fixed and behaviorally
  verified without a provider effect. They do not localize or prove a fix for
  the inner product status stall.
- The sole real attempt authorized for this window is consumed. Any further real
  validation requires fresh explicit authority and a new
  task/thread/turn/attempt identity.
- The worktree contains extensive pre-existing Phase 4 changes. Do not reset,
  clean, move, overwrite, or selectively discard them.
- LATTICE MCP returned receipt-only runtime state with
  `LATTICE_TASK_ADMISSION_MISSING`; do not claim this continuation as a durable
  LATTICE task.

## Git and delivery state

- Branch: `product/lattice-control-mvp`, ahead of remote by four baseline commits.
- Shared Phase 4 worktree remains intentionally dirty and uncommitted because
  the mandatory current live gate failed. No reset, clean, commit,
  push, merge, deploy, release, dashboard/archive claim, or global
  installation/switch occurred.

## Next action

Obtain a new explicit WSL2 live-acceptance authority, use new
task/thread/turn/attempt identity, and run one bounded full `-Wsl2LinuxLive`
acceptance with the hardened harness and the byte-identical binary. Only if it
passes may the final Node/Rust/PostgreSQL/restart/reconnect/outbox gates be
rerun and one local feature commit created. Any failure stops before commit.

## Restart context

- Branch/HEAD: `product/lattice-control-mvp` at
  `f2524cfa7d095febfc162892119c166259c13fbe`.
- Detailed ledger: `docs/workflows/PHASE-004-managed-foreman.md`.
- First files: `apps/lattice-runtime/src/managed_foreman_service.rs` and
  `scripts/test-phase4-managed-foreman.ps1`.
- Current failed-run WSL evidence:
  `\\wsl.localhost\Ubuntu\home\zk\lattice-phase4-wsl2-acceptance-20260830\verifier-state\acceptance-db30e0c8da8b2ff0\evidence`.
- Current failed-run retained Windows root:
  `C:\Users\f7212\AppData\Local\Temp\lattice-phase4-managed-foreman-a80d966e0459aa947e6df4deaee5015f`.
- Current immutable root manifests: Windows
  `52a9576fd98f52ce6576d484824664b2d16b04a3cb59821b4f9580a2b1a67432`;
  WSL `7675c62c3532175d8652d192c9b21477feffd04d5f79ad406f6768786bd1c9fe`.
- Complete current failure receipt:
  `C:\Users\f7212\.codex\sessions\2026\08\29\rollout-2026-08-29T23-10-07-01a04e12-267e-7072-b5a3-13e536c0f84a.jsonl:1644`.
- Historical 2026-08-29 failed-run WSL evidence:
  `\\wsl.localhost\Ubuntu\home\zk\lattice-phase4-wsl2-acceptance-20260828\verifier-state\acceptance-59e83b7be1eb169b\evidence`.
- Historical 2026-08-29 failed-run retained Windows root:
  `C:\Users\f7212\AppData\Local\Temp\lattice-phase4-managed-foreman-335beccd1b9f019cc6971a8fe240f758`.
