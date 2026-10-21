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
10. Build one verified vertical slice at a time. Installation, authentication,
    payment, publication, deployment, and public network exposure remain
    separate human-owned gates.

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

## Open Questions

### Blocking architecture approval

Should the Rust core directly own the single writable Codex app-server process,
with OpenClaw acting only as the gateway?

Recommended answer: **yes**. This avoids two supervisors resuming or writing the
same Codex thread and makes writer leases, cancellation, schema negotiation,
and evidence capture enforceable in one place. The tradeoff is that the
OpenClaw integration becomes a thin LATTICE plugin rather than using its Codex
harness as the writable execution owner.

No Rust implementation, database creation, or constitution activation proceeds
until the user approves this topology and the V2 amendment proposal.

## Acceptance Ownership

| Acceptance | Direct evidence Codex can produce | Final owner |
|---|---|---|
| Plan/spec/ADR consistency and project isolation | Local file inspection and deterministic checks | Codex |
| Rust behavior and module contracts | Unit, property, integration, and characterization tests | Codex |
| PostgreSQL transaction, replay, lease, and migration behavior | Disposable local test database evidence | Codex after DB gate |
| Codex/Graphify/Hermes/OpenClaw adapter compatibility | Exact-version capability preflight with fake-to-live comparison | Codex plus user authorization |
| Quality of learned preferences or improvement proposals | Samples, provenance, counterexamples, rollback evidence | User |
| Self-upgrade promotion and recovery acceptance | A/B activation, health window, rollback drill | User for initial hardened versions |
| Installation, credentials, account actions, payment, public exposure | Not authorized by this plan | User |
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
- [ ] **CURRENT — Step 5:** Build Rust contracts/ports plus fake PostgreSQL, Artifact Store,
  Approval Verifier, and fake OpenClaw IPC adapters; port approved Task
  Domain/Policy/canonical-byte fixtures. The CLI is test/recovery-only and
  calls the same IPC.
  - TASK-008 bootstrap slice: implemented and locally verified on 2026-07-29;
    Step 5 remains current because production contracts, ports, fake adapters,
    and retained fixtures are not implemented yet.
- [ ] Step 6: Add PostgreSQL migrations, transactional event append/replay,
  terminal idempotent command receipts, at-least-once outbox, project registry,
  daemon leadership, and durable writer leases/fencing using a disposable test
  database.
- [ ] Step 7: Port Git workspace and exact Scope Check behavior; retain the
  local filesystem lock only as defense in depth.
- [ ] Step 8: Implement fake Codex and independent review adapters, then run a
  separately authorized Codex preflight that binds exact binary/digest,
  same-binary schema hash, explicit features, and dedicated `CODEX_HOME`.
- [ ] Step 9: Add Graphify code-only source snapshots with a separate
  LATTICE-owned artifact output; forbid install/hooks/live DB/semantic backend.
- [ ] Step 10: Add an OS-contained Hermes research lane that treats all raw
  output as arbitrary and accepts only validated, provenance-bearing
  candidates.
- [ ] Step 11: Implement Codebase Memory candidate, review, promotion,
  retrieval, project-isolation, and provenance behavior.
- [ ] Step 12: Implement the independent self-upgrade guardian with protected
  approval verification, atomic claim/nonce/admission transition, durable
  activation saga, daemon epochs, database-enforced drain/canary admission,
  system-stream write canary, stop control, restart reconciliation, and
  rollback. The first A/B MVP performs no schema migration.
- [ ] Step 13: Run the exact-version live OpenClaw plugin preflight and verify
  `/lattice` submission, status, task approval, and stop routing without direct
  database/Git access or protected-release authority.
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
  - the latest `npm.cmd run verify` attempt in this pass timed out and produced
    no valid success result.

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

## Drift Log

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
