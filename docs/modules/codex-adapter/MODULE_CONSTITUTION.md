---
module_id: codex-adapter
name: Codex App Server Adapter
version: 1.6
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-28
---

## Mission

Own one supervised Codex app-server child process and translate the typed
`DeliveryCodexPort` contract to its version-pinned stdio protocol for the
bounded delivery composition. For managed workers, own the sealed connector
boundary that verifies credential readiness without exposing credential or
account identity to LATTICE or the task shell.

## Non-Goals

- Decide task policy, workflow order, approval, scope, test, Git, or release
  outcomes.
- Access PostgreSQL, OpenClaw, Graphify, Hermes, product credentials, or a
  second writable Codex thread.
- Report success before the app-server emits an unambiguous terminal result.
- Read, copy, persist, log, or project Codex credential bytes, `auth.json`,
  account identity, provider payloads, login artifacts, or raw auth failures.
- Perform login or credential enrollment. Codex owns its OS-keyring-backed
  credential storage; LATTICE consumes only sanitized readiness.

## Owned Data

- Executable path, version, digest, generated protocol-schema digest, process
  identity, native thread/turn identity, normalized events, and completion or
  reconciliation evidence.
- Canonical server-owned Codex-home identity digest, exact managed-config
  digest, per-child opaque App Server session identity, generation, their
  canonical combined digest, and sanitized `chatgpt` readiness boolean.
- No task truth, product files, Git authority, approval, or durable database
  state.
- For WSL2, the Windows gateway identity and the Linux launcher/home/config,
  boot/PID-start/process-group/cgroup fence, and exact zero-member subtree exit
  receipt. The gateway is never projected as the Codex launcher.

## Public Contracts

- Implement `DeliveryCodexPort::run_delivery` and
  `DeliveryCodexPort::interrupt_delivery` with request-bound typed evidence.
- Do not implement or activate the frozen generic `CodexPort` as an alternate
  production writer path.
- Spawn the configured Codex binary with `app-server --listen stdio://`,
  initialize it, and bind every run to one exact working directory and request.
- Accept a controlled task run only when its request includes the exact Task
  Spec digest and Orchestrator-verified live Writer Lease identity, fencing
  token, current-head commitment, holder/worktree claim, and durable intent.
- Bind the child/thread/turn evidence to that same spec/lease/fence/worktree
  identity; never accept a caller-supplied lease, fence, thread, prompt, path,
  or permission mode.
- Normalize protocol events without treating unknown notifications as task
  authority.
- Treat `thread/start` and `turn/start` responses as acceptance only. Report a
  turn active only after the exact matching `turn/started` notification says
  `inProgress`; a terminal arriving without that evidence fails closed.
- On managed restart, reconcile only the retained exact thread/turn and never
  create a replacement while that attempt is active or uncertain.
- Admit a managed connector only for one stable server-owned `CODEX_HOME`
  outside every worktree, with exact `cli_auth_credentials_store = "keyring"`,
  the closed shell-environment allowlist, and no `auth.json` entry of any kind.
- Call the public App Server `account/read` method with token refresh disabled,
  discard its raw response, and expose only auth mode/readiness plus the exact
  App Server generation/session identity and opaque Codex-home/config digests.
- Immediately before every managed provider effect, repeat the sanitized
  readiness read and pass its expected generation/session into the connector;
  the connector rejects drift before writing the provider RPC.
- For `WSL2_LINUX`, start only through the typed descriptor-bound Linux
  supervisor and isolated Linux `CODEX_HOME`; independently verify every
  declared effect-bearing executable/config/repository identity before spawn,
  frame child diagnostics away from supervisor control evidence, and require a
  matching zero-member subtree receipt before retry or reconnect.

## Invariants

1. One adapter instance owns at most one writable app-server child and native
   task thread at a time.
2. Executable version, file digest, and same-binary generated schema digest are
   captured before a run is accepted.
3. EOF, timeout, malformed protocol, or ambiguous completion fails closed as
   reconciliation-required evidence.
4. The adapter never calls Git, tests, PostgreSQL, or another component adapter.
5. Fake/synthetic, expired, suspect, stale, receipt-only, cross-spec,
   cross-worktree, or cross-fence authority cannot start a production turn.
6. Loss of lease-currentness or heartbeat requires bounded interruption and a
   reconciliation result unless terminal non-mutation is proved.
7. The adapter owns no lease repository and cannot acquire, renew, release, or
   project current authority; it consumes only the exact typed writer request
   ordered by Orchestrator.
8. Exact thread and turn identities are immutable for an attempt. Foreign,
   duplicate, non-running start notifications and mismatched restart replay
   cannot establish execution.
9. Codex alone reads credentials from its configured OS keyring. LATTICE does
   not read, migrate, cache, persist, or pass credential bytes.
10. Any `auth.json`, non-keyring config, config drift, home/worktree overlap,
    missing readiness, non-ChatGPT auth mode, generation substitution, or
    home/config digest mismatch fails closed before claim or provider effect.
11. The managed task shell receives only the exact non-secret environment
    allowlist. `CODEX_HOME`, user-profile/app-data homes, database variables,
    and token/key/secret variables are excluded from child shell commands.
12. Readiness evidence never contains email, account ID, plan, token, prompt,
    provider error text/data, filesystem locator, or raw remote URL.
13. The owned marker and exact keyring config remain file-identity sealed
    against write/delete/link substitution for the app-server process lifetime;
    each Rust provider-effect boundary re-verifies the same seal.
14. Managed shell `PATH` is rebuilt from a server-owned exact directory
    allowlist, never ambient `PATH`; any admitted directory containing a Codex
    launcher is rejected. Only the outer absolute `LATTICE_CODEX_BIN` selects
    the supervised executable.
15. Durable worker observations bind the canonical digest of App Server
    session, Codex-home digest, and config digest. The pair with generation is
    immutable within an attempt except at an exact `RECONCILED` boundary; an
    old pair cannot resume progress or close the rotated attempt.

## Allowed Dependencies

- `lattice-contracts` and `lattice-ports` public APIs.
- Rust process, async I/O, JSON, hashing, timeout, and path libraries needed for
  the versioned stdio client.
- The configured official `codex` executable.

## Forbidden Dependencies

- Orchestrator internals, direct model/provider APIs, database clients, Git
  mutation libraries, OpenClaw SDK, Graphify, Hermes, Guardian, or product
  credential stores.

## Failure, Compatibility, And Migration

Unknown executable identity, unsupported protocol/schema, initialization
failure, process loss, cancellation uncertainty, or terminal ambiguity blocks
success and returns typed failure or reconciliation evidence. Protocol changes
require a compatibility update; no in-place silent fallback is allowed.

Version 1.5 strengthens the version 1.4 credential boundary with per-child
session identity, connector-side pre-dispatch fencing, process-lifetime home
seals, a non-ambient launcher-free shell path, and durable identity replay.

Version 1.4 preserves the same sole process/protocol lane and adds a managed
credential boundary: Codex owns OS-keyring credential access; LATTICE accepts
only exact, sanitized, generation/home/config-bound `account/read` readiness.
No login, token migration, plaintext auth fallback, or account projection is
introduced. Missing enrollment remains a typed blocker.

Version 1.2 preserves the same sole `DeliveryCodexPort` process/protocol lane
and adds mandatory Task-Spec/Writer-Lease/fence/worktree binding. It adds no
lease repository, PostgreSQL dependency, caller-selected prompt/path, generic
Codex port activation, or second writable child.

## Acceptance Gates

| Gate | Evidence | Owner | Required for merge |
|---|---|---|---|
| Binary identity | exact path/version/digest and same-binary schema digest | Engineering | yes |
| Protocol lifecycle | initialize, thread, turn, terminal, interrupt tests | Engineering | yes |
| Real bounded run | app-server modifies only an isolated acceptance repo | Engineering | yes |
| Failure closure | EOF/timeout/malformed/ambiguous cases never report success | Engineering | yes |
| Writer authority | fake/synthetic/stale/cross-spec/cross-fence/current-head substitution matrix blocks spawn | Security review | yes |
| Lease loss | currentness-loss interruption/reconciliation and zero-later-success evidence | Engineering | yes |
| Exact active start and restart | accepted-versus-started distinction, exact IDs/status, active resume, terminal replay, and duplicate-start denial | Engineering | yes |
| Credential isolation | exact keyring config, no `auth.json`, server-owned non-overlapping home, process-lifetime marker/config seal, non-ambient launcher-free shell path, per-effect sanitized `account/read`, generation/session fence, tamper/missing readiness denial | Security review | yes |

## Change Policy

Writable-thread ownership, executable trust, protocol methods, permission
mapping, cancellation, or success semantics require a versioned constitution
amendment and architecture review.

## Amendment History

| Version | Date | Decision reference | Summary | Approver |
|---|---|---|---|---|
| 1.0 | 2026-08-05 | SPEC-002 v24, TASK-032 | First supervised Codex app-server boundary | Current user delivery-first directive |
| 1.1 | 2026-08-05 | SPEC-002 v26, ADR-021 clarification, TASK-032 | Bind the production adapter explicitly to the approved typed `DeliveryCodexPort`; generic writer port remains frozen | User approval of typed delivery contracts/ports in preceding implementation window |
| 1.2 | 2026-08-09 | SPEC-003 v3, ADR-023, TASK-038 | Bind every controlled Codex turn to one Task Spec, live Writer Lease current head, fencing token, and worktree claim | User TASK-038-first direction |
| 1.3 | 2026-08-26 | SPEC-011, ADR-028 | Require exact `turn/started` before execution and exact retained-ID reconciliation after restart | Delegated product owner |
| 1.4 | 2026-08-28 | SPEC-011 v1.5, ADR-028 amendment | Make Codex the sole credential reader; require keyring-only state and sanitized identity-bound readiness before managed provider effects | Delegated product owner |
| 1.5 | 2026-08-28 | ADR-028 security amendment | Bind every provider effect and durable worker observation to the exact App Server session/home/config identity; seal config lifetime and remove ambient shell PATH | Delegated product owner |
