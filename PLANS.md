# LATTICE DevOS V2 Plan

## Goal

Build **LATTICE DevOS — 織網 AI 開發中樞** as a general-purpose,
local-first autonomous AI development platform that can improve its own
development workflows safely over time.

The target composition is:

> OpenClaw + Rust LATTICE Core + PostgreSQL + Codex + Graphify + Hermes +
> Codebase Memory

The governing invariant remains:

> **One Gateway. One Truth. One Writer.**

## Current GitHub Handoff — 2026-08-10

Repository: `z72124223/lattice-devos`<br>
GitHub URL: `https://github.com/z72124223/lattice-devos`<br>
Remote: `origin` -> `https://github.com/z72124223/lattice-devos.git`<br>
Default branch: not established; remote HEAD is currently
`feature/task-037-full-chain-integration`<br>
Current branch: `feature/task-038-chatgpt-mcp`<br>
Upstream: none; this branch has not been pushed<br>
Base checkpoint: local Phase 1 commit `512732d`<br>
Latest fetched TASK-037 remote head: `8828d2b`<br>

Current Goal:
- Preserve the accepted canonical-local TASK-038 Phase 2 checkpoint, publish
  its exact feature commit, then refresh the real Secure MCP Tunnel so ChatGPT
  can discover/invoke the two bounded task tools. Only after that separate live
  ChatGPT gate resumes TASK-037 production-chain diagnosis/repair.

CURRENT TASK-038 — add `lattice_task_submit` and `lattice_task_status` through
the same `latticed` One Gateway. The first public intent is exactly
`CONTROLLED_CODEX_CANARY`; LATTICE builds the complete Task Spec 2.1, preserves
one spec digest, uses PostgreSQL Task Ledger truth and a real PostgreSQL Writer
Lease/fence, governs the sole Codex writer, verification, Git, and fresh-process
status replay. Canonical-local acceptance is `PASS`; actual ChatGPT tunnel
discovery/invoke remains open and is not inferred from local NDJSON evidence.

User Order Correction — 2026-08-09:
1. TASK-038 bounded GPT -> LATTICE -> Codex Submit/Status first.
2. TASK-037 production-chain repair second.
3. Final combined production acceptance only after both independent gates pass.

Completed Evidence Retained:
- TASK-038 Phase 1 private tunnel health/readiness and ChatGPT discovery proved
  exactly `lattice_delivery_run` and `lattice_delivery_status` across legacy and
  stateless MCP generations.
- Both delivery tools retain closed zero-argument schemas and a composition-
  owned binding; hostile caller properties fail before service dispatch.
- This is transport/discovery evidence only. It does not prove Task Submit,
  production database connectivity, Writer Lease/fencing, controlled Codex
  dispatch, or full-chain completion.
- TASK-038 canonical-local acceptance ID
  `8c21e96b9bc44b1d87de0dea884b9678` ran the production `latticed` executable
  and official Codex `0.146.0` against a disposable real PostgreSQL 17.10
  cluster and Git repository. Its canonical `latticed` SHA-256 is
  `130ef9f92f2582055d9828828c95526a58f01aa7772e43c4db31062219d278b2`;
  `final.json` is under that acceptance directory.
- Submit and fresh-process Status returned the same six-field terminal result:
  task ref `ab8724dd51419cf190ad491f1f8973894bca56dc0c3aed55ebc3723f6214177d`,
  ledger head `f3d3b84625f80f26e90b6ed06514bb4f7e8a65f112c089fb6d8a3c82b7d6cdc2`,
  result `457bab1f71b5bd69e99f3240ca170a25fb88895b94474efd7d747916b2c86bcd`.
- PostgreSQL physical restart, Writer Lease live/fault suite, exact retry,
  different-key denial, one Codex invocation, fixed verification, one governed
  Git commit, and zero-rerun status all passed. Git, database, and timestamp-
  sensitive Codex Home footprints were byte-identical across fresh Status.
- Graphify/Hermes/Memory effect delta was zero, current writer was absent, the
  execution home was removed, and the read-only credential source was unchanged.
- Completed and `Merging + result` recovery now independently replay Writer
  Lease snapshot/checkpoint plus physical command/transition history before
  accepting terminal state. The fixed canary admits only active `1/1/1` or
  released `1/2/2` history; absent, active-at-completion, or drifted history
  fails before a new Task Ledger mutation.

Current Phase 2 Decision:
- ADR-023 and SPEC-003 v4 select process-start-owned closed ingress evidence.
  Production Secure MCP Tunnel and local canonical acceptance use distinct
  non-substitutable commitments. MCP `clientInfo` and caller-supplied identity
  never grant authority.
- Submit accepts only `CONTROLLED_CODEX_CANARY` plus a bounded
  `client_request_id`; Status accepts only the returned lowercase SHA-256
  `task_ref`. No arbitrary task text, shell,
  SQL, path, Git/test command, credential, lease/fence, or writer thread exists
  in the public schema.
- Task Domain 2.2 exports the complete validated Task Spec 2.1 canonical
  subject/document; no gateway/adapter may rebuild a reduced digest carrier.
- Writer Lease 1.1 owns canonical snapshot/checkpoint bytes and the repository
  trait. New `postgres-writer-lease` 1.0 owns the independent exact
  `db/extensions/writer-lease/v1.sql` persistence adapter. Postgres Store 1.6
  recognizes the exact combined catalog/ACL profile and may invoke only the
  fixed same-transaction current-authority predicate for fenced Task Ledger
  mutation; it does not own or mutate lease state.
- Global migrations `0001` through `0004` remain unchanged; TASK-038 creates no
  `0005` and does not place lease state in Codebase Memory.
- Task Submit/Status is `WriterOnly`; it must leave Graphify/Hermes/Memory at
  zero. The `lattice_delivery_run` MCP tool enters the same governed writer path
  and may start downstream only after durable Task completion and Writer Lease
  release. The non-MCP compatibility command remains scripted-only. Alternate
  `lattice-full-chain` is a read-only delivery observer: its retained Run name
  is fixed-denied before dispatch and it cannot discover or invoke task tools.
- Fresh fixed-canary execution is bounded to 300 seconds with a 30-second
  finalization reserve and 600-second Writer Lease TTL. Longer/general task
  profiles stay closed pending heartbeat, interrupt, and orphan recovery.

In Progress / Not Yet Accepted:
- Refresh the real Secure MCP Tunnel after the stable feature checkpoint, then
  prove ChatGPT discovers/invokes `lattice_task_submit` and a separate new
  request/session invokes `lattice_task_status` for the same durable result.
- Publish only the exact feature branch after clean-commit review and GitHub
  authentication; no PR, merge, tag, release, deployment, or force push.
- Full workspace strict Clippy remains blocked by eleven lints in unchanged
  `lattice-hermes-adapter`. Workspace tests and changed-slice strict Clippy pass;
  the TASK-038 branch does not absorb the separate TASK-041/042 Hermes slice.
- Broader task templates remain closed until durable Project Registry/live
  Policy, heartbeat, governed interruption, and orphan recovery are composed.

Deferred Until TASK-038 Phase 2 Completes:
- The latest ChatGPT delivery-status observation is
  `LATTICED_DATABASE_CONNECT_REJECTED`. It is the current first visible
  production-chain failure and must not be assumed to share the older Hermes
  root cause.
- Older TASK-037 evidence remains
  `HERMES_PRODUCTION_CHILD_EXITED / LATTICE_HERMES_REFLECTION_REJECTED` at the
  later Hermes boundary. It remains useful incident history, not the current
  first blocker.
- TASK-037 will resume by diagnosing the actual first production failure, then
  proceeding gate-by-gate through PostgreSQL -> Codex -> Graphify -> Hermes ->
  Memory -> Status. No production E2E PASS exists today.

Known Boundaries:
- The fixed profile actor is deliberately not a per-human ChatGPT identity.
  Broader identity and task templates require later versioned decisions.
- No remote default/main branch, CI, push, merge, deployment, or release is
  established/performed by this local phase.
- Historical sections below are retained for evidence; this top section is the
  authoritative current order and completion truth.

Relevant Issue:
- GitHub Issue #4: `TASK-038 — GPT Web UI ↔ LATTICE MCP Gateway Interface`.
- Issue #6 / TASK-039 will consume the stable provider-neutral Task, Writer
  Lease, process-start, and Codex thread/turn evidence produced by this
  checkpoint; it must not become a second Task truth or writer controller.
- Issue #2 Passive Supervisor may read those durable projections only after
  TASK-039 defines freshness/observation semantics; it must not bypass the
  existing Orchestrator.

## User Execution Preference — 2026-07-29

The user changed the project workflow preference to direct execution without
routine human review prompts: project changes may be submitted automatically,
and the user will report problems for correction. The same preference applies
to installation, credential setup, database connections, and external-component
activation from the user's project perspective.

This preference changes the interaction policy, not the platform safety
boundary. System-enforced restrictions on credentials, accounts, payment,
public exposure, irreversible deletion, legal commitments, and other protected
operations remain fail-closed. Such actions must be reported as blocked or
gated rather than silently performed.

## Delivery-First Target Mode — 2026-08-05

The active objective is a runnable local product path, not completion of the
remaining governance backlog. The required product path is:

> Codex App or OpenClaw -> Rust LATTICE -> PostgreSQL -> Codex app-server
> modifies/tests/commits -> Graphify -> Hermes -> Codebase Memory -> queryable
> result

Work proceeds in executable nodes. At every node boundary, verify that the
node produced runnable progress toward this path, used real components where
claimed, avoided unrelated documentation/review expansion, passed its bounded
checks, and was committed. A node that does not advance this path is stopped
and replanned. Non-blocking polish and exhaustive edge-case work are deferred;
startup failure, a broken core path, corrupt durable state, credential leakage,
or inability to create a recoverable local commit remains blocking.

## Approved TASK-032 Architecture Amendment — 2026-08-05

The user approved this versioned amendment in the preceding task window; it is
an implementation input, not a pending review gate:

1. `lattice-contracts` and `lattice-ports` gain typed delivery requests,
   evidence, and PostgreSQL-ledger/workspace/verification/Git ports.
2. `orchestrator-runtime` remains pure Rust effect ordering and may reach
   Codex, PostgreSQL, verification, or Git only through those ports.
3. `latticed` 1.0 is the composition root implemented by the existing
   `apps/lattice-runtime` package; the `lattice-runtime` binary remains a
   compatibility wrapper, not a second orchestrator.
4. The bounded MCP stdio surface exposes exactly two zero-argument tools:
   `lattice_delivery_run` and `lattice_delivery_status`. Tool callers cannot
   supply shell commands, SQL, filesystem paths, or credentials. OpenClaw
   remains the normal human gateway.
5. TASK-032's exact-path allowlist is expanded only for this architecture and
   its executable acceptance path. Deployment, payment, publication, and the
   unrelated website remain excluded.

## Current TASK-032 Incident Gate — 2026-08-05

The first official-Codex live attempt is `FAILED_DIAGNOSTIC`, not acceptance
evidence. Windows displayed `codex-windows-sandbox-setup.exe` with "The
specified module could not be found" while the official turn was editing the
isolated fixture. The exact npm-package helper is OpenAI-signed, x64, SHA-256
`7191d24f6fb4a26cbbce0d2aecd6deb71fa074a8cb5f24a45d2fa2164473885f`;
its direct imports resolve to installed Windows system DLLs or an API-set.
OpenAI issues [#29952](https://github.com/openai/codex/issues/29952) and
[#29200](https://github.com/openai/codex/issues/29200) remain open with the same
modal failure and no linked repair branch or pull request.

Until that upstream Windows sandbox-helper regression/compatibility failure is
resolved or the user explicitly authorizes a new safety posture, all official
Codex live and sandbox-setup launches are fail-closed before process or database
effects. Do not retry, install/modify system components, or switch to
unelevated/no-sandbox. Scripted acceptance, code fixes, tests, review, and
durable handoff work may continue, but cannot close TASK-032's official-live
acceptance criterion.

TASK-032 therefore remains `FAILED_DIAGNOSTIC` for official live, but it no
longer blocks independently safe downstream executable nodes. The user
explicitly directed the next bounded node to attach real Graphify and
PostgreSQL Codebase Memory to the already verified scripted delivery fixture.
This does not waive or reinterpret the official-live criterion.

## Current TASK-033 Graphify/Memory Slice — 2026-08-05

The current executable node is intentionally narrow:

1. Pin the current official Graphify stable release and invoke only its
   headless, local, code-only extraction against a LATTICE-materialized exact
   Git commit snapshot. The source snapshot is read-only and output is confined
   to separate LATTICE staging. On this Windows host, the fixed production
   boundary is direct `wsl.exe --exec` plus bubblewrap user/mount/network
   namespaces; it exposes no shell or unbound host path.
2. Preserve project/commit/tree/source-manifest provenance and content digests
   in typed Contracts 1.11 evidence. Graphify output is derived evidence, not
   task, policy, scope, or release authority.
3. Codebase Memory 1.0 normalizes structural graph facts, deterministically
   ranks a fixed process-owned query, and persists candidate observations only
   through the independent same-database PostgreSQL Memory adapter. The exact
   extension profile is not part of the global Store migration manifest.
4. Orchestrator 2.2 owns `snapshot -> Graphify -> validate -> persist ->
   retrieve` effect order through Ports 1.7. The first failure stops later
   effects; partial or malformed output is never committed.
5. `latticed` 1.1 keeps exactly the existing two zero-parameter MCP tools. The
   run tool extends the preconfigured scripted acceptance chain; status reads
   the exact project/commit analysis and memory evidence from PostgreSQL. No
   shell, SQL, path, credential, query, or provider input is added to MCP.
6. ADR-020, Postgres Store 1.4, Project Registry's reserved global `0005`, and
   global schema v4 remain authoritative and unchanged. The proposed Memory
   extension path is `db/extensions/codebase-memory/v1.sql`, with its own exact
   hash, identity, ledger, explicit admin runner, and V3+Memory verifier.
   The user has now approved the precise bounded amendment: new
   `postgres-codebase-memory` 1.0 is the sole persistence owner. A newly
   discovered compatibility constraint requires Postgres Store's read-only
   verifier to recognize one exact V3+Memory catalog/ACL profile because its
   current closure hashes the `control`, `memory`, and `readmodel` schemas.
   That compatibility amendment may neither depend on Codebase Memory nor
   install/write Memory state, and it may not change `migrations.rs`,
   migrations `0001` through `0004`, the global manifest, or Registry-reserved
   `0005`/schema-v4.

The trusted scripted checkpoint is now complete: fixture
`c9bf2939ad5844e9973ee0af0a84b756` persisted typed intent/outcome/receipt
evidence through a PostgreSQL 17.10 restart and produced clean fixture commit
`ed408cc4373519f57950a66660148df39f9d5f82`, changing only `answer.txt`.
Focused/full verification and final architecture review pass with only deferred
non-blocking deadline/initialization hardening. This evidence does not replace
the blocked official-live criterion.

The TASK-033 production checkpoint is also complete. The fixed
`lattice_delivery_run` composition now binds the durable delivery receipt to
project `task032-delivery`, TASK-033, and the exact fixture commit before
Graphify persistence; `lattice_delivery_status` opens a fresh runtime-role
connection and replays only the exact PostgreSQL Memory receipt. The single
combined command `powershell.exe -NoProfile -ExecutionPolicy Bypass -File
.\scripts\run-lattice-delivery.ps1` exited 0 in 254 seconds on PostgreSQL
17.10, including Store+Memory initial/restart, Graphify persistence, a second
database stop/start, exact run/status equality, and an unchanged Graphify
execution footprint during fresh-process status. No global migration,
Registry reservation, MCP input/tool, official Codex, Hermes, or OpenClaw
boundary changed.

## Global Strategy

1. Use OpenClaw as the only normal human command, status, task-approval, and
   stop gateway. Keep protected core-release approval on a guardian-owned
   OS-authenticated administrative surface.
2. Put orchestration, policy, task state transitions, capability verification,
   scope enforcement, and adapter supervision in a Rust control core.
3. Use PostgreSQL as the only durable control-plane truth for task events,
   approvals, writer leases, evidence, capability observations, memory
   promotion, and release state.
4. Let Codex app-server be the only product-code Implementer, operating in one
   LATTICE-owned Git worktree while holding a current lease and fencing token.
5. Keep Graphify's product-source input read-only and route its required output
   to a separate LATTICE artifact root. Treat graph output as a versioned,
   content-addressed derived snapshot whose `EXTRACTED` edges are evidence and
   whose inferred edges remain leads until confirmed.
6. Keep Hermes' product input read-only and enforce whole-process OS
   containment. It may research, reflect, summarize failures, and propose
   memory/skill/improvement candidates, but arbitrary output crosses the
   boundary only through schema/provenance validation and never becomes
   authoritative by itself.
7. Make Codebase Memory a LATTICE-owned PostgreSQL subsystem with provenance,
   fact/observation/inference labels, review status, revision history, project
   isolation, and conservative retrieval.
8. Implement self-improvement as an evidence loop: observe -> propose -> test
   in isolation -> review -> stage -> activate through an independent guardian
   -> monitor -> rollback. LATTICE may never silently approve and overwrite
   itself.
9. Preserve the existing Node.js implementation as a prototype and
   characterization suite. Port proven behavior incrementally; do not reset,
   delete, or bulk-rewrite the current dirty worktree.
10. Build one verified vertical slice at a time. Do not pause for routine
    project review prompts; preserve verification evidence and automatically
    submit safe local changes. Protected system actions remain fail-closed.

## Non-Goals

- Build or modify any particular website or unrelated user project.
- Make a website, managed cloud service, messaging bot, or cloud deployment a
  prerequisite for the local platform.
- Create an unconstrained self-modifying agent.
- Allow OpenClaw, Hermes, Graphify, generated files, or a second database to
  become an independent truth source.
- Allow Hermes, Graphify, reviewers, the Integrator, or the upgrade controller
  to write product code.
- Let OpenClaw and the Rust core concurrently own or resume the same writable
  Codex native thread.
- Install OpenClaw, Graphify, Hermes, `uv`, crates, or database extensions
  during the planning gate.
- Create PostgreSQL roles/databases, use credentials, call a model, pay, publish,
  push, merge, or deploy before its explicit execution gate.
- Claim full autonomy, safety, or live compatibility from plans, static files,
  local unit tests, or fake adapters alone.

## Scope

Current V2 replan scope:

- Record the user-authoritative direction change.
- Replace the active Node-first plan and product charter.
- Draft a behavior-focused V2 specification.
- Draft architecture decisions for Rust/polyglot boundaries, PostgreSQL truth,
  adapter topology, and self-upgrade safety.
- Propose versioned amendments for existing modules and constitutions for new
  modules without activating them.
- Classify old tickets and the Node prototype as preserved legacy evidence.
- Update the workflow ledger and durable handoff.

Implementation after approval:

- Rust workspace and local service/CLI.
- Typed fake OpenClaw IPC contract in the first vertical slice; CLI remains a
  test/recovery client over the same IPC rather than a second normal gateway.
- PostgreSQL schema migrations and transactional repositories.
- V2 Task Packet and compatibility reader for V1 evidence.
- Policy, project registry, event ledger, writer leases/daemon epochs, approval
  verification, artifact store, Git worktrees, and exact scope checks.
- Fake adapters followed by capability-gated Codex, Graphify, Hermes, and
  OpenClaw adapters.
- Codebase Memory and a guarded self-upgrade controller.

Governance/source worktree:

`C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos`

Active V2 implementation worktree:

`C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`

## MVP Definitions And Current Status

An MVP is complete only when its named exit evidence is recorded in the
workflow ledger and handoff. Static design, a fake adapter, a passing unit test,
and an exact-version live preflight are separate evidence levels and are never
substituted for one another.

| MVP | Status | Required outcome | Exit evidence | Explicitly not claimed |
|---|---|---|---|---|
| MVP-0 — Rust foundation | **COMPLETE — 2026-07-29** | TASK-008 workspace/bootstrap plus TASK-009 versioned contracts and ports; four active V2 module constitutions; no provider I/O | local format, Clippy, 14 Rust tests, 38 preserved Node tests, independent code/architecture review | no PostgreSQL durability, live provider, autonomous execution, release, merge, or deployment |
| MVP-1 — Deliverable local alpha | **CURRENT — TASK-033; 12 foundation tickets plus scripted delivery checkpoint complete** | one thin real chain from Codex App or OpenClaw through Rust/PostgreSQL to Codex modification, test and commit, followed by real Graphify, Hermes, and project-isolated Codebase Memory retrieval | a repeatable local acceptance run records exact component identities, durable task/result state, changed paths, test result, commit, graph/reflection artifacts, and memory query | no production hardening, public service, silent self-release, or claim from fake adapters |
| MVP-2 — Isolation and recovery | **PLANNED** | harden the MVP-1 chain with durable leases, exact scope enforcement, cancellation, reconciliation, component isolation, restart recovery, and measured retrieval quality | fault/cancel/restart/reconciliation tests, OS-boundary evidence, scope isolation, compatibility matrix, memory benchmark | no unconstrained agent, second writer/truth, public service, or self-release authority |
| MVP-3 — Guardian-protected autonomy | **PLANNED** | outcomes can propose normal improvement tasks; immutable A/B candidates pass independent review, protected guardian claim, drain/canary/health, restart reconciliation, and rollback | fault-injected activation saga, nonce/epoch/admission enforcement, complete-drain proof, write canary, power-loss and rollback drill | no silent policy/constitution/credential/public-exposure change and no in-place self-overwrite |

Delivery dependency sequence:

- MVP-1 now follows runnable vertical nodes rather than waiting for every old
  foundation ticket. TASK-032 first proves real Codex app-server, PostgreSQL,
  bounded Git modification, tests, and a local commit. Subsequent nodes add
  Graphify/Codebase Memory, then Hermes, then OpenClaw to that same executable
  path. TASK-032 official live stays diagnostic-blocked independently.
- Unfinished TASK-022 through TASK-031 work is preserved as hardening backlog;
  it is pulled forward only when a missing control blocks the runnable path.
- MVP-2 closes isolation, recovery, and edge-case gaps observed in the working
  MVP-1 chain. MVP-3 begins only after that hardened chain has current evidence.

MVP-0 completion is a foundation milestone, not a claim that the full platform
already runs. MVP-1 through MVP-3 remain incomplete until their direct exit
evidence exists.

## Confirmed Facts

- The user explicitly defined LATTICE as a separate, general AI development
  platform for their computer and explicitly excluded the unrelated website
  project from the product.
- The user explicitly requested Rust, PostgreSQL, OpenClaw, Codex, Graphify,
  Hermes, Codebase Memory, and continuous improvement/iteration.
- The existing repository is a Node.js Phase 1 prototype on
  `feature/phase1-controlled-swarm`.
- The current worktree contains eight pre-existing modified paths from TASK-004
  security work; no reset, clean, branch switch, or broad move is safe.
- Existing V1 contracts preserve valuable invariants: immutable task specs,
  digest-bound approvals, event replay, default deny, one Implementer, fencing,
  isolated worktrees, fail-closed conflicts, detection-only scope checks, and a
  deterministic Fake Runtime.
- Existing V1 contracts also conflict with the new direction by making Node.js
  and a file ledger the core while excluding Graphify, Hermes, long-term
  memory, PostgreSQL, and real Codex execution.
- No SQLite implementation exists in this repository. This is not a
  SQLite-to-PostgreSQL migration.
- Local evidence on 2026-07-29 confirms `rustc 1.97.1`, `cargo 1.97.1`,
  PostgreSQL client 17.10, a running `postgresql-x64-17` service, and
  `127.0.0.1:5432` accepting connections.
- `psql.exe` exists under `C:\Program Files\PostgreSQL\17\bin` but is not on the
  current PATH. Database identity, credentials, roles, and migration permission
  have not been tested.
- `codex-cli 0.144.6` is available. OpenClaw, Graphify, Hermes, and `uv` are not
  currently found on PATH.
- TASK-012 baseline verification on 2026-07-29 passes 94 Rust tests and 38
  preserved Node tests. No Project Registry crate or active Project Registry
  constitution existed before the TASK-012 governance pass.
- TASK-012 completion verification on 2026-07-29 passes Contracts 11, Project
  Registry 16, Policy 70, full Rust workspace 118, and preserved Node 38
  tests; independent code, security, architecture, and governance reviews all
  pass.
- Official OpenClaw documents a Codex app-server harness and a
  TypeScript/ESM plugin boundary. The current official Codex manual describes
  app-server as experimental and documents bidirectional JSON-RPC-shaped
  messages over stdio JSONL.
- Official Graphify documentation describes code extraction and distinguishes
  extracted from inferred graph relationships; it also writes graph artifacts,
  so only its source boundary—not the whole process—is read-only.
- Official Hermes documentation exposes programmatic integration and an
  optional Codex app-server runtime.
- The dated, tag-pinned research snapshot and adapter caveats are recorded in
  `docs/reviews/COMPONENT_CONTRACT_EVIDENCE_2026-07-29.md`.

## Assumptions

- "For my computer" means the first supported deployment is one Windows machine
  with loopback-only services; WSL/Linux packaging may follow after the local
  vertical slice.
- Rust-first does not mean every dependency is rewritten in Rust. OpenClaw's
  thin plugin remains TypeScript/ESM, while Graphify and Hermes remain
  separately supervised Python processes.
- Hermes' sessions, memory, skills, tools, and optional Codex runtime overlap
  LATTICE responsibilities; the proposed adapter therefore isolates them and
  accepts only candidate output. This is an architecture inference, not an
  upstream guarantee.
- PostgreSQL full-text search is the zero-extension retrieval baseline to
  benchmark. It is not assumed sufficient for Traditional Chinese,
  mixed-language, symbol/path, error-code, or exact-filename queries. Vector or
  other indexing remains optional and requires measured need plus approval.
- The Node prototype remains executable only as characterization evidence until
  equivalent Rust behavior is verified. It is not merge-ready or a production
  fallback.
- Self-improvement may automatically collect local evidence and prepare
  proposals. Promotion of a LATTICE core release remains human-approved in the
  first hardened version.

## Resolved Architecture Decision

The Rust core owns the single writable Codex app-server process/thread and
OpenClaw remains the thin normal gateway. The user approved this topology and
the V2 module direction on 2026-07-29. This removes the former planning blocker;
each implementation ticket must still prove its own scope, capability, and
safety gates.

TASK-012 resolves Registry/Policy ownership without a dependency cycle:
`lattice-contracts` 1.2 owns only neutral Project ID/class/lifecycle,
physical-Git-ref, fixed-producer receipt, and full authority-head
representations; Project Registry 1.1 owns the complete identity/lifecycle
aggregate, accepted/pending reservations, defensive blocking, and receipt
issuance; Policy 2.3 owns only Task-Spec-bound receipt sufficiency and compares
an independent current owner head. Registry and Policy do not depend on one
another. Future Orchestrator/PostgreSQL performs authenticated, serialized
current-head composition.

TASK-013 resolves Ledger/Policy resource ownership by the same acyclic pattern:
Contracts 1.3 owns neutral immutable full Ledger head/resource receipt
representation; Task Ledger 2.0 owns event/request/receipt hashes, replay,
resource projection, and fake issuance; Policy 2.4 owns only Task-Spec-bound
receipt/current-head sufficiency. Task Ledger and Policy do not depend on one
another, and Task Ledger does not duplicate Task Domain transition legality.
Authenticated durable currentness plus the atomic resource/effect/outbox claim
remain future Orchestrator/PostgreSQL responsibilities.

## Acceptance Ownership

| Acceptance | Direct evidence Codex can produce | Final owner |
|---|---|---|
| Plan/spec/ADR consistency and project isolation | Local file inspection and deterministic checks | Codex |
| Rust behavior and module contracts | Unit, property, integration, and characterization tests | Codex |
| PostgreSQL transaction, replay, lease, and migration behavior | Disposable local test database evidence | Codex under an exact bounded ticket |
| Codex/Graphify/Hermes/OpenClaw adapter compatibility | Exact-version capability preflight with fake-to-live comparison | Codex when the required local capability is available |
| Quality of learned preferences or improvement proposals | Samples, provenance, counterexamples, rollback evidence | Evidence gate; user may override or report corrections |
| Self-upgrade promotion and recovery acceptance | A/B activation, health window, rollback drill | Independent guardian plus protected OS-authenticated release gate |
| Installation and local component setup | Exact package/source/version plus post-install verification | Codex under a bounded ticket |
| Credentials, account/payment actions, public exposure, irreversible actions | Authenticated protected-action evidence | User/protected system surface |
| Primary-branch merge | Current checks, review, synchronization, explicit authorization | User |

## Implementation Steps

- [x] Step 1: Audit the repository, dirty Git state, original attachment,
  current local toolchain, and official component contracts.
- [x] Step 2: Complete the V2 plan, charter, draft specification,
  proposed ADRs, module-amendment proposal, workflow ledger, architecture
  review, and handoff.
- [x] Step 3: Obtain explicit user approval for the proposed Codex ownership
  topology and versioned module amendments. Approved by the user on
  2026-07-29 with `好 開始執行`.
- [x] Step 4: Create dependency-aware V2 tickets and a branch/worktree plan that
  preserves the dirty Node prototype.
- [x] Step 5: Build Rust contracts/ports plus fake PostgreSQL, Artifact Store,
  Approval Verifier, and fake OpenClaw IPC adapters; port approved Task
  Domain/Policy/canonical-byte fixtures. The CLI is test/recovery-only and
  calls the same IPC.
  - TASK-008 bootstrap slice: implemented and locally verified on 2026-07-29;
    Step 5 remains current because production contracts, ports, fake adapters,
    and retained fixtures are not implemented yet.
  - TASK-009 established dependency-only `lattice-contracts` and
    `lattice-ports` crates. OpenClaw is modeled as the inbound
    `GatewayService`, while PostgreSQL, Codex, Graphify, and Hermes remain
    abstract outbound boundaries. This slice defines no orchestration or
    domain transition, activates no fake/live provider module, and performs no
    I/O. Provider fakes remain separate sequential tickets aligned with Steps
    5, 8, 9, and 10.
    - Completed and locally verified on 2026-07-29. Independent code review's
      component/boundary cross-label P1 was fixed with lane-specific evidence
      return types; final code review has no findings and architecture review
      has no blocker.
    - Step 5 remains current because Task Domain/Policy/canonical bytes and the
      fake PostgreSQL, Artifact Store, Approval Verifier, and OpenClaw IPC
      slices were not all implemented at that point.
  - TASK-010 activated Task Domain V2 and the pure `lattice-cjson-1` mechanism.
    It freezes Task Spec hashing, strict scalar/path validation, the V1
    transition characterization, and deterministic DAG evidence.
    - Completed and locally verified on 2026-07-29: canonical 8, Task Domain 6,
      Rust workspace 28, and preserved Node 38 tests pass; independent code
      review has no findings and architecture review has no blocker.
    - No database, filesystem, process, network, provider, credential, or
      product-repository I/O was added.
  - **COMPLETED TASK-011:** activate Policy Engine V2 and bind the immutable Task
    Spec, role/action/state, project/snapshot, runtime admission,
    capability/provider identity, network/deployment/cost, risk/approval,
    writer/fencing, resources, memory, and upgrade facts to deterministic
    default-deny decisions.
    - ADR-009 makes the approved pure dependency edge explicit:
      `lattice-policy -> lattice-task-domain + lattice-contracts`.
    - Policy evaluates typed authority facts but neither produces nor persists
      them. TASK-011 performs no database, filesystem, process, network,
      provider, credential, payment, publication, deployment, or
      product-repository I/O.
    - Independent review RED (2026-07-29): generic-gate bypass and unbound
      merge/cost/writer/memory/protected-release subjects were reproduced.
    - Independent re-review was initially `BLOCKED` on seven exact-subject
      gaps, all closed with RED/GREEN evidence: protected-release approval
      binds the Guardian runtime identity; merge conflict/readiness is a fresh
      `workspace-git` fact bound to the reviewed subject and target head;
      resource usage must be a fresh Task-Ledger-owned fact bound to the same
      Task Spec and one accounting currency; runtime reconciliation must use a
      dedicated typed normal/guardian recovery subject; primary-branch Git refs
      must be fully qualified local `refs/heads/*` Registry identities;
      rollback reverses the failed activation slots with a strictly newer
      epoch; and writer
      release must bind the requesting actor.
    - Security/code re-review RED (2026-07-29): Git revision pseudo-ref `HEAD`
      could masquerade as a feature branch; a same-Task-Spec Resource Fact
      could be replayed for another effect claim; rollback carried an unused
      activation digest instead of the exact typed protected-release receipt;
      and Task Domain admitted decimal budgets longer than Policy can parse.
      The active repair rejects ambiguous Git namespaces, gives each resource
      gate an independent exact Ledger observation subject, uses the typed
      protected-release receipt for rollback, and shares one 256-byte canonical
      decimal contract with 127 integer digits and 128 fractional digits, so
      mixed-scale checked arithmetic also remains representable.
    - Final Windows adversarial RED (2026-07-29): case-insensitive Git storage
      can resolve `refs/heads/Main` and `refs/heads/main` to the same physical
      ref while Policy string comparison classifies them differently. Registry
      and Workspace Git must therefore supply the same owner-produced physical
      ref-identity digest; Policy classifies primary by that identity, never by
      platform case guessing.
    - Architecture re-review RED (2026-07-29): a typed recovery target alone
      did not prove that an unknown effect, dead holder, replaced leadership,
      or interrupted guardian saga had actually been reconciled. Normal
      supervisor recovery may only move to `STOPPED`; restoring `ACTIVE`
      requires the exact Guardian lane plus owner-produced durable
      DB/boot/saga resolution evidence.
    - The fixed decision precedence also applies to Worker Admission and Merge:
      a requested external-cost effect is rejected before approval or resource
      budget failures are reported.
    - Completed and locally verified on 2026-07-29: Policy 66, Task Domain 6,
      Rust workspace 94, and preserved Node 38 tests pass. Code, security, and
      architecture re-reviews all return `PASS`; local combined integration
      passes. Remote CI and merge readiness remain separately unavailable.
  - **COMPLETED TASK-012:** activate the Project Registry owner contract and its
    deterministic fake evidence boundary. Freeze registered project/snapshot
    identity, fully qualified primary ref, physical `GitRefIdentity`,
    repository drift/reconciliation, and exact owner receipt semantics before
    PostgreSQL persistence or real Workspace Git consumes them.
    - This directly supplies the owner side of TASK-011 project/merge facts and
      records the future exact Scope Check receipt composition gate.
    - It remains pure/fake and local: no live repository mutation, database,
      credential, network, provider, publication, deployment, or protected
      action.
    - **Evidence/design pass complete:** baseline Rust 94 and Node 38 tests
      pass; two independent read-only audits reject either
      `policy -> registry` or `registry -> policy`.
    - **Governance activated before code and review-amended:** ADR-010,
      SPEC-002 v8, Project Registry 1.1, Contracts 1.2, Policy 2.3, and
      TASK-012 freeze shared values, fixed-producer full-head receipts,
      accepted/pending identity reservation, idempotent command receipts,
      snapshot rotation, defensive blocking, and exact reconciliation.
    - **Implementation RED/GREEN complete:** the pure fake Registry registers,
      resolves, observes drift, suspends, reconciles, reserves pending
      identities, rejects NFC/hash aliases, distinguishes zero-mutation
      `Denied` from state-changing `Blocked`, exposes a fake current-head
      lookup, and composes with Policy through the shared full-head contract.
    - **Review RED/GREEN complete:** producer substitution, receipt security
      field substitution, pending-identity front-running, authoritative
      cross-project collision, Unicode normalization collision, and overbroad
      uppercase pseudo-ref rejection were reproduced and repaired. Governance
      was synchronized before final full verification and re-review.
    - **Completion evidence:** Contracts 11, Registry 16, Policy 70, Rust
      workspace 118, and Node 38 tests pass; format, Clippy, dependency,
      forbidden-I/O, constitution, project, and diff checks pass. Final code,
      security, architecture, and governance reviews return `PASS`; local
      combined integration passes. Remote CI and merge readiness remain
      separately unavailable.
  - **COMPLETED TASK-013:** freeze the Rust Task Ledger V2 owner
    boundary and deterministic fake append/replay/command-receipt/resource
    behavior before PostgreSQL supplies durable event, outbox, and
    resource-claim transactions.
    - TASK-012 completion, V1 characterization, active Rust contracts, dirty
      worktree, and Rust 118/Node 38 baselines were re-audited.
    - SPEC-002 v9, ADR-011, Task Ledger 2.0, Contracts 1.3, Policy 2.4, and one
      bounded TASK-013 ticket freeze the owner/dependency/current-head
      contract before code.
    - Task Ledger owns chain and resource replay only; Task Domain retains legal
      task transitions and future Orchestrator composes both.
    - Preserve One Truth: the fake is characterization/composition evidence,
      never a second durable truth, restart proof, or permission to bypass the
      PostgreSQL atomic resource/effect claim.
    - Implementation and review RED/GREEN close complete raw snapshot/replay,
      appended and denied receipt verification, cross-identity poisoning,
      corrupt exact retry, uncreated-stream terminal-denial export, Task ID
      parity, diagnostic bounds/secret leakage, full Policy substitution
      matrices, and actual fake-owner current-head composition.
    - Completion evidence: Contracts 13, Task Ledger 20, Policy 75, Rust
      workspace 145, and preserved Node 38 tests pass. Format, Clippy,
      dependency, no-I/O, constitution, project, and diff checks pass. Final
      code/security and architecture reviews return `PASS`; local combined
      integration passes.
    - AC-27 is complete. PostgreSQL atomicity/durability/restart, authenticated
      current heads, and live append planning remain explicitly open.
  - **COMPLETED TASK-014:** freeze Writer Lease 1.0 as the next pure semantic
    owner before PostgreSQL persists leases and fencing.
    - Audit complete: V1 retains exact one-writer/holder/lease/fence intent but
      its writable file counter can roll back/reuse a fence and issue a value
      beyond JavaScript's safe integer range while validation still passes.
    - Governance frozen before code: SPEC-002 v10, ADR-012, Writer Lease 1.0,
      Contracts 1.4, Policy 2.5, TASK-014, and the workflow audit define one
      reusable public planner/verifier plus deterministic fake.
    - Remove Policy's remaining caller-owned lease active/current/role/epoch/
      fence/count fields through a fixed-producer receipt plus independent
      current owner head, following the Registry and Ledger pattern.
    - Bound holder, worktree, process-start, task/spec, daemon epoch, positive
      signed-BIGINT fencing/revision, heartbeat/expiry, suspect/revoke
      evidence, exact command idempotency, and runtime-admission behavior.
    - `DRAINING` denies heartbeats but permits exact release/recovery;
      `CANARY` and `STOPPED` deny all user-project lease transitions;
      `RECONCILIATION_REQUIRED` permits only typed recovery.
    - Keep the fake I/O-free with injected time/process/admission evidence.
      Concurrent acquisition, restart, database time, authenticated process
      death, and old-connection fencing remain Step 6 PostgreSQL evidence, not
      TASK-014 claims. AC-05 therefore stays open while new pure AC-28 is the
      TASK-014 closure criterion.
    - Implementation and review RED/GREEN close raw ingress, complete semantic
      replay, rollback/overwrite, fake/live history, heartbeat expiry
      regression, daemon-bound holder death, suspect/admission behavior, exact
      Policy release composition, and denial-only receipt-tail truncation.
    - Completion evidence: Writer Lease 24, Policy 81, Rust workspace 180, and
      preserved Node 38 tests pass. Strict Clippy, format, dependency, no-I/O,
      project/governance, V1 lock characterization, and diff checks pass.
      Independent final code/security and architecture reviews return `PASS`
      with zero remaining P0 through P3 finding; local combined integration
      passes.
    - AC-28 is complete. AC-05 remains open for PostgreSQL concurrency,
      database time, atomic trusted-checkpoint persistence, restart,
      authenticated recovery evidence, and stale live connection fencing.
  - **COMPLETED TASK-015:** freeze Approval Verifier 1.0 before
    PostgreSQL, OpenClaw approval IPC, or Guardian activation consumes approval
    authority.
    - The next slice removes Policy's remaining caller-owned approval
      verification/currentness booleans using a fixed-producer receipt plus an
      independently queried owner head, following Registry, Ledger, and Writer
      Lease.
    - The bounded first implementation is pure/fake: exact approval subject,
      actor/authority/channel/session, target/spec, nonce, issue/expiry,
      challenge/receipt, exact retry/reuse denial, and protected-versus-normal
      authority separation.
    - True OS authentication, trust-root/key access, database persistence,
      clock/randomness, durable nonce consumption, IPC, activation, and product
      effects remain later owner/adaptor tickets. In particular, protected
      release nonce consumption remains exclusive to the future
      Guardian/PostgreSQL atomic activation claim.
    - Audit found a P1: Policy 2.5 accepts an unused caller subject digest,
      five caller verification/currentness/self-approval Booleans, and two R3
      review Booleans while only checking time strings for non-empty text.
    - Governance is frozen before code in SPEC-002 v11, ADR-013, Approval
      Verifier 1.0, Contracts 1.5, Policy 2.6, TASK-015, and the workflow audit.
      Complete typed approval subjects move to Contracts representation so
      Policy can compare the owner receipt without depending on Verifier/cjson.
    - Approval Verifier cannot manufacture Review Runtime authority. R3 and
      every independent-review-required allow path must fail closed until a
      later bounded Review Runtime owner ticket supplies receipt/current-head
      evidence.
    - Implementation and review RED/GREEN closed Policy's independent-review
      early-return and fact-memory bypasses, public challenge substitution,
      incomplete golden/subject/trust/retry/rollback matrices, missing typed
      revocation, and revocation-governance drift.
    - Completion evidence: Contracts 25, Approval Verifier 28, Policy 84, Rust
      workspace 218, and preserved Node 38 tests pass. Strict Clippy, format,
      dependency, no-I/O, legacy-Boolean, project/governance, and diff checks
      pass. Independent final code/security and architecture reviews return
      `PASS` with zero remaining P0 through P3 finding; local combined
      integration passes.
    - AC-29 is complete. Live authentication, PostgreSQL uniqueness/database
      time/durability/restart/atomic claim, OpenClaw approval IPC, Review
      Runtime, Guardian activation, and product effects remain explicitly
      open.
  - **COMPLETED TASK-016:** freeze and implement Artifact Store 1.0 before
    Graphify, Hermes, Codebase Memory, PostgreSQL, or provider adapters can
    retain or reference generated evidence.
    - Repository/task-boundary audit confirms TASK-015 still serves the
      platform-wide MVP-1 goal; the configured local project-router entry point
      is absent, so PLANS/HANDOFF/Git state provide the direct route.
    - Governance is frozen before code through SPEC-002 v12, ADR-014,
      Artifact Store 1.0, Contracts 1.6, TASK-016, and its workflow audit.
    - Contracts 1.6 and one atomic public `FakeArtifactStore` owner now bind
      lifecycle mutation, complete typed authority, command/history quota,
      terminal receipts, exact applied/denied retry, current heads, byte
      verification, and delete/reconciliation behavior. The lower mechanisms
      remain crate-private and cannot become a second writer.
    - Object identity is project-scoped `(project_id, sha256)` with positive
      generation; immutable per-use references bind complete task/provenance
      metadata, Registry/effect/daemon/admission/capability owner evidence, and
      limit snapshot without granting provider trust.
    - The pure owner verifies byte/manifest/object/reference/task/project/
      staging bounds, atomic quota projections, typed fixed-owner reference
      authority, exact retry, currentness, replay/checkpoint, and safe
      delete-claim preconditions through a visibly non-durable fake.
    - Task object/active-byte attribution is active-reference-only. Holder IDs,
      complete persisted lifecycle strings, and the domain-separated 64-byte
      delete claim token are included in `FieldBytes` quota; exact boundary and
      plus-one cases deny without partial mutation.
    - The compact checkpoint contains identity, limit, snapshot, replay-bound,
      and rollback-sensitive trust-anchor commitments only. It retains neither
      payload bytes nor a second metadata owner. Replay preflights exact encoded
      canonical size, reconstructs lifecycle/history/quota/staging/command/
      terminal rows from untrusted raw data without context, validates every
      join/digest, and then compares the independent trusted commitments.
    - Independent governance review caught and closed pre-code P1 gaps:
      caller opaque digests cannot authorize initial/retain/release/read/sweep;
      provenance binds live owner/effect/daemon evidence; deletion uses an
      exact durable claim token plus unknown-outcome reconciliation; aggregate
      quotas bound small-object/reference/metadata exhaustion; and
      delete-claimed/reconciliation/orphan state retains worst-case capacity
      until verified terminal evidence.
    - Implementation review additionally found and closed payload-copying
      checkpoint construction, post-allocation canonical byte limits,
      checkpoint-clone replay, incomplete terminal receipt restoration, and
      holder/claim-token `FieldBytes` gaps. Each accepted finding received a
      direct regression before repair.
    - Completion evidence: Contracts 32, Artifact Store 97, locked Rust
      workspace 322, and preserved Node 38 tests pass. Format, strict workspace
      Clippy, dependency, forbidden-I/O/provider/product/unrelated-website,
      raw-byte containment, project/governance, and diff checks pass.
      Independent final code/security and architecture reviews return `PASS`
      with zero remaining P0 through P3 findings; local combined integration
      passes.
    - AC-30 is complete. PostgreSQL reference transactions/durability/restart,
      real filesystem containment/staging/delete, and live owner authority
      remain explicitly open under AC-19 and later tickets.
    - PostgreSQL remains future durable metadata/reference truth; a separate
      owned-root filesystem adapter later performs staging/flush/atomic rename/
      verified read/link containment/exact unlink. The pure crate exposes no
      real deletion and no provider/product alternate authority.
  - TASK-017 is complete. Gateway IPC 1.1/wire protocol 1.0, Contracts 1.7,
    and Ports 1.2 implement the bounded canonical pure/fake boundary. Seventy
    focused tests, 358 full Rust tests, 41 Node tests, dependency/I/O scans,
    machine-enforced unique-ticket/current-marker governance, independent
    code/security and architecture reviews, and local integration pass. AC-31
    is complete; live OpenClaw/transport/authentication AC-07 remains open.
  - TASK-018 is complete. Postgres Store 1.0, Contracts 1.8, and Ports 1.3
    are implemented and locally complete for the typed zero-I/O fake.
    Focused package suites pass 61 tests, the full Rust workspace passes 380,
    and preserved Node verification passes 44. Strict format/Clippy,
    dependency/I/O/SQL/driver/migration/provider/product scans, independent
    code/security and architecture reviews, and local integration pass. AC-32
    is complete; no PostgreSQL connection, migration execution, driver, or
    durability claim occurred.
  - TASK-019 is complete. Postgres Store 1.1.5 adds the exact manifest,
    administrative runner, repeatable-read verifier, STOPPED/no-leader schema,
    real LOGIN capability separation, catalog/ACL/ownership/protected-function
    closure, and marker-owned PostgreSQL 17.10 restart harness. Thirty-five
    Store tests, 401 full Rust tests, 44 Node tests, two clean live harness
    trials, strict format/Clippy, independent code/security and architecture
    reviews, and local integration pass. AC-33 is complete; no live
    `ControlStore`, domain repository, production target, or activation path
    was claimed.
  - TASK-020 is complete. Contracts 1.9, Ports 1.4, and Postgres Store 1.2 add
    the exact schema-v2 expansion and the narrow live durable physical
    `ControlStore`. The marker-owned PostgreSQL 17.10 harness passes fresh and
    exact-v1-prefix upgrades, live apply/stale/replay/substitution,
    concurrency, bounded retries, overflow, corruption, commit-response-loss
    reconciliation, and restart. The full Rust workspace passes 409 tests,
    preserved Node verification passes 44 tests, strict format/Clippy and
    `cargo audit` pass, and independent code/security, architecture, and local
    integration reviews pass. AC-34 is complete; no domain repository,
    activation, production target, provider/product, or release path was
    claimed.
- [ ] **PAUSED — Step 6:** Complete TASK-018 through TASK-025: first freeze a typed,
  zero-I/O Postgres Store fake, then add checksum migrations/runtime admission,
  durable Ledger/outbox, Registry, Lease, Approval, and Artifact repositories
  using only a disposable PostgreSQL database; finally add the disposable
  owned-root Artifact filesystem adapter.
  - TASK-021 is complete. SPEC-002 v23 AC-03, AC-04, and AC-35 have direct
    durable evidence; final code/security and architecture findings are all
    zero, and the marker-owned PostgreSQL 17.10 initial/restart harness plus
    432 Rust and 44 Node tests pass. MVP-1 remains 12/22 tickets (54.5%).
  - **PAUSED TASK-022:** Implement the first durable Project Registry
    repository one verified behavior at a time. Project Registry 1.2 remains pure
    and add one runtime-aware verified global state, command plan/apply,
    immutable global checkpoint, ordered command replay, and projection/
    reservation verification shared by Fake and PostgreSQL. Postgres Store 1.4
    will add exact schema v4 through migration `0005` and a Registry-specific
    global transaction because
    registration denial can have no authority snapshot and cross-project
    accepted/pending identity collisions require one serialization point.
    TASK-022 must not forge a `ProjectSnapshotId`, reinterpret a per-project
    `StoreScope`, change Store-v2 receipt hashes, or move identity legality into
    SQL. Contracts and Ports remain unchanged. SPEC-002 v24, ADR-020, both
    versioned constitutions, TASK-022, and its workflow audit now agree.
    The first independent governance review correctly returned CHANGES
    REQUIRED (P0=0, P1=5, P2=4): the corrected set now freezes an acyclic hash
    construction order, canonical logical-byte accounting, an independent
    retained-checkpoint comparison, vacant high-water 0 with a seeded Live
    singleton, exact `15/28/17/11-ungranted` catalog totals, scalar Registry
    signatures capped at 73 inputs, and bounded idle/total transaction time.
    The fresh independent governance re-review passed with P0=P1=P2=P3=0 and
    released only the governance blocker. The current bounded code action is
    Registry 1.1 golden characterization followed by focused Registry 1.2 TDD;
    the first RED/GREEN is complete: both Fake/Live vacant checkpoints,
    independently recomputed 103-byte logical state, retained-checkpoint
    reconstruction, and all four Registry 1.1 literal vectors pass in the
    19-test Project Registry package. The second RED/GREEN is also complete:
    opaque vacant snapshot export, plain self-consistency verification, and
    independent retained-checkpoint currentness are distinct and the package
    now passes 20 tests plus strict crate Clippy. The next bounded RED covers
    first-registration planning without mutating its verified vacant base; no
    implementation-complete claim is implied.
    Read-only schema reconnaissance also found that ADR-020 freezes the leading
    global-profile pair only for the nine Registry functions, not for the eight
    Store-v4/Task-Ledger-v2 successors. Pure Registry TDD may continue, but
    schema-v4 RED/SQL work is blocked until ADR-020, the Postgres Store
    constitution, and TASK-022 freeze that pair as positions 1-2 plus the exact
    successor input counts and pass a bounded governance check.
    AC-06 remains open for real Windows/Git inspection, Workspace Git, and
    Scope Check. Writer Lease, Approval, Artifact, external components,
    production/release/deployment, and the unrelated website remain excluded.
- [ ] **BLOCKED TASK-032 — executable Codex/PostgreSQL delivery node:** first
  record the approved versioned contract/port/orchestrator/`latticed` boundary,
  then implement it with TDD and prove an official Codex app-server repository
  modification, fixed verification, Git commit, durable PostgreSQL result, and
  restart status replay. The existing scripted checkpoint remains a test
  harness and is not live-Codex evidence. This first real segment does not
  claim OpenClaw, Graphify, Hermes, or Codebase Memory until each real component
  is attached and verified by a following executable node. Current bounded
  scripted subpath, fail-closed repair, full verification, and durable incident
  handoff are complete. The official-live acceptance substep remains blocked by
  the incident gate above and must not be retried in this window; TASK-032 stays
  `in-progress` with official acceptance `FAILED_DIAGNOSTIC`.
- [ ] **TASK-033 historical full-chain Steps 9/10 continuation:** the exact
  Graphify/PostgreSQL Codebase Memory checkpoint is complete and remains the
  control. The current step persists a typed Hermes `INFERENCE/CANDIDATE`
  reflection through the same `postgres-codebase-memory` owner and proves fresh
  status performs zero Hermes calls. Production Hermes additionally requires
  verified OS containment, empty isolated cwd/no product mount, memory off,
  redacted bounded context, exact identity, and one absolute deadline. After
  that passes, authenticated loopback OpenClaw may expose only
  submit/status/stop through the same fixed GatewayService and PostgreSQL
  terminal idempotency. Baseline commits `b682ca4` and `e648529` are integration
  inputs, not live acceptance evidence; the two zero-parameter MCP tools remain
  unchanged.
- [ ] Step 7: Complete TASK-026 and TASK-027 for Workspace Git 2.0 and exact
  Scope Check 1.1 behavior in disposable repositories; retain the local
  filesystem lock only as defense in depth.
- [ ] Step 8: Preserve unfinished TASK-028 through TASK-031 controls as
  hardening backlog and pull forward only a control that blocks the executable
  delivery path; do not let the older fake-adapter sequence override TASK-032.
- [ ] **CURRENT STEP 9:** Attach the OS-contained Hermes reflection lane to the
  same exact graph/memory receipt and persist/replay it only through PostgreSQL.
- [ ] Step 10: After Hermes passes, attach and verify the bounded OpenClaw
  gateway against the same durable delivery request and receipt.
- [ ] Step 11: Harden Codebase Memory retrieval quality/project isolation and
  run the full local component fault/restart/reconciliation exit gate.
- [ ] Step 12: Implement the MVP-3 independent self-upgrade Guardian with protected
  approval verification, atomic claim/nonce/admission transition, durable
  activation saga, daemon epochs, database-enforced drain/canary admission,
  system-stream write canary, stop control, restart reconciliation, and
  rollback. The first A/B MVP performs no schema migration.
- [ ] Step 13: Run the disposable A/B process/filesystem, power-loss, canary,
  health-disagreement, higher-epoch rollback, and complete improvement-to-
  protected-activation integration drills. A real active-slot promotion or
  service replacement remains a protected action.
- [ ] Step 14: Run full local verification, independent code review,
  architecture review, integration readiness, and user acceptance; do not merge
  or deploy without authorization.

## Delivery Forecast

These are engineering estimates after V2 architecture approval, not promises:

- First Rust/PostgreSQL vertical slice with fake adapters: **3-5 working days**.
- Verifiable offline Rust/PostgreSQL control core: **8-14 working days**.
- Integrated local MVP with all five component adapters: **3-6 weeks**.
- Hardened self-improvement and A/B self-upgrade loop with rollback drills:
  **5-8 weeks**.

The fastest safe route is to make the first vertical slice small, then add one
external adapter at a time behind the same contracts.

## Verification

- Planning gate:
  - every active product document names the general platform scope;
  - V1 is clearly historical/characterization evidence;
  - no new document claims an install, account, database, live adapter, or
    deployment exists;
  - document links, frontmatter, module references, and exactly one current step
    are checked locally.
- Rust gates after approval:
  - `cargo fmt --check`;
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
  - focused crate tests followed by `cargo test --workspace`;
  - compatibility tests replaying V1 characterization fixtures.
- PostgreSQL gates after approval:
  - reversible migrations test up/down; forward-only migrations test
    restore/compatibility rather than pretending destructive downgrade;
  - concurrent append, receipt races, unknown commit, at-least-once outbox,
    hash-chain, transaction-failure, daemon epoch/admission across every
    durable repository mutation, lease, fencing, suspect holder, and restart
    tests;
  - no credential or raw secret persistence.
- Adapter gates:
  - fake contract tests first;
  - exact binary/version/schema/capability evidence;
  - timeouts, cancellation, malformed output, duplicate events, and unavailable
    dependency fail closed;
  - Graphify/Hermes cannot mutate a product worktree.
- Self-upgrade gates:
  - candidate build and verification never run in the active installation;
  - staged activation has exact-subject approval, a separate guardian, durable
    saga states, atomic nonce claim, database-enforced drain/canary admission,
    epoch enforcement, complete drain proof, health/write canary, rollback
    target, and power-loss evidence;
  - promotion policy and user-owned acceptance remain distinct.
- Existing Node verification:
  - the V2 planning pass does not claim the dirty prototype is newly verified;
  - an earlier V2 planning attempt timed out and produced no valid success
    result; the newer TASK-016 closure run on 2026-08-01 completed successfully
    with `check=ok` and all 38 preserved Node tests passing.

## Risks

- Running OpenClaw's Codex harness and a Rust-owned Codex client against the
  same native thread could violate One Writer. Mitigation: approve one writable
  execution owner and use fork/read-only lanes for any secondary view.
- PostgreSQL can become a nominal truth while files or external memory silently
  act as another truth. Mitigation: label generated files and external stores
  as caches/candidates and require PostgreSQL event references plus hashes.
- Autonomous improvement can optimize for its own tests or promote unsafe
  changes. Mitigation: immutable acceptance sets, independent review, staged
  activation, health windows, rollback, and no self-approval.
- Hermes and Graphify are external Python supply-chain/runtime surfaces.
  Mitigation: pin exact versions only after approval, run them with least
  privilege, capture capabilities, and reject unknown schemas.
- PostgreSQL credentials and role configuration are not verified. Mitigation:
  use a disposable least-privilege database in a later explicit gate and never
  print or persist credentials.
- The current Node worktree contains unfinished security changes and a known
  fencing-counter safe-integer risk. Mitigation: preserve it untouched and use
  its behavior/tests only as fallible characterization evidence.
- Plans and local tests cannot prove production containment or live adapter
  behavior. Mitigation: keep static, fake, live preflight, human acceptance,
  and machine enforcement statuses separate.
- A fake physical Git-ref digest could accidentally be mistaken for a real
  loose/packed-ref identity algorithm. Mitigation: receipts carry
  `RuntimeKind::Fake`; Policy requires runtime agreement; Workspace Git must
  separately prove a stable physical algorithm with disposable repositories.

## Drift Log

- 2026-08-05: TASK-033's independent extension initially assumed that keeping
  Postgres Store source byte-identical was sufficient. Current code evidence
  disproved that assumption: the Store verifier closes catalog and ACL hashes
  across `control`, `memory`, and `readmodel`, so independent Memory objects
  would make existing Store/Ledger construction fail. The plan now keeps
  `postgres-codebase-memory` as the sole Memory persistence owner while allowing
  only an exact read-only Postgres Store V3+Memory verifier profile. Global
  migration bytes/manifest and Registry `0005` remain forbidden. The first
  compile-time RED exited 1 for the intentionally missing typed database/
  extension identity and exact embedded-manifest APIs.

- 2026-07-29: The original attachment used one unrelated website as an example
  and proposed a Node-first, managed-cloud-oriented sequence. Direct user
  clarification superseded that interpretation: LATTICE is a separate,
  general-purpose platform for the user's computer.
- 2026-07-29: The requested core changed from dependency-light Node.js plus a
  file ledger to Rust plus PostgreSQL. Existing code is preserved as a
  prototype rather than rewritten in place.
- 2026-07-29: Graphify, Hermes, real Codex execution, and long-term Codebase
  Memory moved from deferred/non-goals to required V2 lanes.
- 2026-07-29: The old TASK-005 is no longer current. The user approved the V2
  topology and module direction; TASK-008 established an inert Rust workspace
  in a separate worktree without activating live components.
- 2026-07-29: Existing TASK-004 adversarial changes remain uncommitted. This
  replan does not discard them or repeat their prior verification claims.
- 2026-07-29: TASK-012 moved neutral Project ID/class/lifecycle and physical
  Git-ref/authority-receipt representation from Policy-local shapes into
  contracts 1.1, while keeping mutable project truth solely in Project
  Registry and Task-Spec-specific sufficiency solely in Policy 2.2. This
  prevents a Registry/Policy dependency cycle and removes contradictory
  project-status booleans.
- 2026-07-29: TASK-012 independent review required a versioned hardening to
  SPEC-002 v8, Project Registry 1.1, Contracts 1.2, and Policy 2.3. Ordinary
  duplicate registration/reconciliation remains `Denied` with no mutation,
  while an authoritative observation collision returns `Blocked` and rotates
  the observed project to `SUSPENDED`; pending identities are reserved, hash
  subjects must already be NFC, authority heads mirror every security field,
  and currentness requires an independent Registry-owner lookup.
