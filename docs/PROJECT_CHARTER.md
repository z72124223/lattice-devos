> 歷史架構與模組索引：保留演進證據；其中 OpenClaw 入口、逐工單治理及舊工具安裝核准流程不再作為現行要求。
> 目前產品邊界與驗證方式以 [AGENTS.md](../AGENTS.md) 及其工程契約為準。

# LATTICE DevOS Product Charter

## Identity

- Formal name: **LATTICE DevOS**
- Chinese name: **織網 AI 開發中樞**
- Expansion: **Layered Agent Task, Tool, Intelligence, Context & Execution**
- Direction: **V2 — Local Autonomous Development Platform**
- Repository: `lattice-devos`
- Normal gateway: OpenClaw
- User command: `/lattice`

## Product Mission

LATTICE DevOS is a general-purpose, local-first platform that turns a user's
software-development intent into planned, bounded, implemented, verified,
remembered, and continuously improvable work.

It belongs to the user's computer and can serve registered software projects.
It is not part of any one website or application.

## Governing Principle

> **One Gateway. One Truth. One Writer.**
>
> 一個正常入口、一個耐久事實來源、每個寫入領域同時間一個授權寫入者。

### One Gateway

OpenClaw is the normal human entry for task submission, status, approval,
rejection, and stop. A supervisor recovery surface may expose only
status/stop/rollback; it cannot create ordinary development tasks. Protected
core-release approval uses a separate guardian-owned OS-authenticated
administrative surface. This is a security boundary, not a second normal task
gateway.

### One Truth

PostgreSQL event streams are the durable authority for task control, approvals,
leases, evidence references, memory promotion, and release activation.
Projections, Graphify graphs, Hermes memory, transcripts, generated reports,
filesystem locks, and process queues are derived evidence or candidates.

### One Writer

- Product code: one Codex Implementer with a current project lease.
- Task/control events: one valid `latticed` daemon epoch appends through the
  transactional store; except for guardian release/epoch procedures, the
  database rejects stale instance/epoch or disallowed runtime-admission mode on
  every daemon-authorized durable mutation/effect.
- Release activation stream and active slot: only the independent guardian may
  append through narrow release/epoch procedures and activate or roll back a
  verified bundle through a recoverable saga.

No one of these writers may assume another writer's authority.

## Required Components And Roles

| Component or role | Mission | Product-code write |
|---|---|---:|
| OpenClaw gateway | Authenticated commands, status, approvals, stop | no |
| Rust LATTICE core | Orchestration, policy, task state, scope, adapter supervision | only by granting the Codex lease |
| PostgreSQL | Durable event, approval, lease, evidence, memory, and release truth | not product code |
| Project Registry | Canonical project/repository identity and lifecycle | no |
| Writer Lease | Lease/fencing/epoch domain rules | grants exclusive authority only |
| Approval Verifier | Exact-subject identity, nonce, expiry, and protected release approval | no |
| Artifact Store | Content-addressed identity/reference/quota/delete semantics; physical bytes remain in a separate owned-root filesystem adapter | no |
| Codex app-server | Exclusive Implementer in one owned worktree | yes, exclusive |
| Graphify adapter | Read-only source analysis; writes only derived artifact staging | no |
| Hermes adapter | Read-only product research; writes only quarantined candidate output | no |
| Codebase Memory | Provenance-bound knowledge candidate/review/retrieval | not product code |
| Review Runtime | Independent correctness, security, architecture, and test review | no |
| Integrator | Non-conflicting Git metadata integration | no product-code edits |
| Upgrade guardian | Stage, activate, monitor, and roll back release bundles | no product-code edits |

## Product Capabilities

1. Register arbitrary local Git projects through explicit roots and identities.
2. Freeze an immutable Task Spec and approval subject.
3. Plan with read-only code graph, repository, and memory evidence.
4. Run exactly one code-writing Implementer in an isolated worktree.
5. Verify tests, scope, security, and architecture before integration.
6. Record durable, replayable task and evidence events in PostgreSQL.
7. Store only provenance-bound Codebase Memory with candidate, accepted,
   rejected, superseded, and quarantined states.
8. Learn from outcomes by proposing improvements without silently promoting
   them.
9. Build upgrades in an inactive location, shadow-check them, activate through
   a separate guardian, and roll back on failure.
10. Remain stoppable, inspectable, versioned, and recoverable.

## Safety And Trust Boundaries

- Unknown roles, actions, schemas, binaries, adapter capabilities, and
  approvals default to deny.
- External agent output is untrusted data, never an instruction with authority.
- Graphify inferred edges and Hermes proposals cannot authorize work.
- A profile or tool setting is not OS isolation. External processes require
  independently enforced containment and separately writable output roots.
- Memory cannot change policy, scope, approvals, leases, or release state.
- A passing Git diff check is evidence, not operating-system containment.
- Model/runtime approvals are defense in depth; LATTICE policy is authoritative.
- Self-improvement may not alter policy, module constitutions, the supervisor,
  database compatibility, credentials, network exposure, or protected actions
  without the responsible human's approval.
- No installation, login, payment, public publication, deployment, permanent
  deletion, or security-control disablement is implied by this charter.

## Technology Boundary

- Trusted control plane: Rust.
- Durable database: PostgreSQL.
- Normal local IPC: OS-local authenticated transport; no public listener by
  default.
- OpenClaw boundary: thin TypeScript/ESM plugin.
- External tools: separately pinned and supervised Graphify and Hermes Python
  processes.
- Product changes: Codex app-server with exact executable pinning, a schema hash
  generated by that binary, and explicit feature probes.

## Compatibility Position

The existing Node.js V1 work is a preserved prototype and characterization
source. V2 must port its useful invariants through observable fixtures and
versioned compatibility readers. It must not dual-write with the Rust core.

The V2 charter does not activate revised module constitutions. The proposed
topology, ADRs, and versioned amendments require explicit user approval before
implementation.

## Human-Owned Gates

- V2 architecture and module-constitution approval.
- Installation and exact-version pinning of external components.
- Account, OAuth, API key, database credential, and secret handling.
- Capability expansion, new network access, and paid model/API use.
- Destructive or incompatible database migration.
- Initial LATTICE core release promotions and supervisor updates.
- Primary-branch merge, public release, and deployment.
