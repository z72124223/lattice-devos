# LATTICE DevOS

**LATTICE DevOS（織網 AI 開發中樞）** is a general-purpose, local-first
autonomous AI development platform.

> **One Gateway. One Truth. One Writer.**

Target composition:

```text
OpenClaw gateway
        ↓
Rust LATTICE control core
        ↓
PostgreSQL durable truth
        ├─ Codex: exclusive code Implementer
        ├─ Graphify: read-only source → derived graph artifact
        ├─ Hermes: read-only product input → untrusted candidate
        └─ Codebase Memory: provenance and review
```

This repository is not part of any particular website or user project.
Projects become targets only after explicit registration and a bounded Task
Packet.

## Current Status

`MVP-1 IN PROGRESS — TASK-021 COMPLETE; TASK-022 TDD IN PROGRESS`.

TASK-021 completed the first durable domain repository. Task Ledger 2.1 remains
the pure Rust semantic owner, while Postgres Store 1.3 atomically persists its
terminal commands, optional events/outbox admissions, projection/checkpoint,
and applied physical Store receipt in PostgreSQL. The latest completed baseline
is 12/22 MVP-1 tickets (54.5%); this does not complete MVP-1, MVP-2, MVP-3, or
the full platform.

TASK-022 is the current TDD slice for durable global Project Registry
persistence. Its first independent governance review returned
`CHANGES REQUIRED — IMPLEMENTATION BLOCKED`; the corrected governance set then
passed fresh independent re-review with P0=P1=P2=P3=0. Only the governance
blocker is released. No TASK-022 Rust, SQL, migration, PostgreSQL acceptance,
or completion claim exists yet.

- Active plan: `PLANS.md`
- Current charter: `docs/PROJECT_CHARTER.md`
- Direction authority: `docs/source/DIRECTION_CHANGE_2026-07-29.md`
- Ready V2 specification:
  `docs/specs/SPEC-002-autonomous-development-platform.md`
- Approved module direction: `docs/modules/V2_AMENDMENT_PROPOSAL.md`
- Current governance ticket: `docs/tickets/TASK-039-hermes-broker-protocol.md`
- Current workflow audit:
  `docs/reviews/WORKFLOW_AUDIT_TASK_022_2026-08-03.md`
- First TASK-022 governance review:
  `docs/reviews/GOVERNANCE_REVIEW_TASK_022_2026-08-03.md`
- Passing TASK-022 governance re-review:
  `docs/reviews/GOVERNANCE_REREVIEW_TASK_022_2026-08-03.md`
- Latest completed implementation handoff: `HANDOFF.md` (TASK-021)

Completed TASK-021 evidence includes a marker-owned loopback PostgreSQL 17.10
initial/restart harness, durable Store/Task-Ledger transactions, 432 Rust tests,
44 preserved Node tests, strict Clippy, format, and dependency audit. That local
disposable-database evidence is not production activation or TASK-022 evidence.
OpenClaw, Codex, Graphify, Hermes, and Codebase Memory are not yet live-integrated;
no account/payment action, production database change, publication, deployment,
push, or merge is claimed here.

## Quick Start

The current CLI surface remains inspection-only:

```powershell
cargo run -p lattice-cli -- status
```

It lists the approved component lanes and their authority modes. It does not
start services, open network listeners, activate a daemon, connect the normal
runtime to PostgreSQL, invoke Codex, or run Graphify/Hermes. PostgreSQL evidence
to date comes only from the separate marker-owned disposable verification
harness completed through TASK-021.

## Architecture Position

- Rust owns trusted orchestration, policy, scope, process supervision, and
  local service behavior.
- PostgreSQL owns durable tasks, events, approvals, leases, evidence references,
  memory promotion, and release activation.
- OpenClaw is a thin normal gateway and does not directly access PostgreSQL,
  Git, providers, or product files.
- Codex app-server is the approved exclusive product-code writer.
- Graphify and Hermes are read-only with respect to product inputs/code. Their
  writable output is confined to separate LATTICE artifact/candidate roots and
  remains derived evidence or an untrusted candidate.
- Self-improvement produces normal reviewed tasks. A separate guardian stages,
  activates, monitors, and rolls back immutable release bundles.

## Preserved V1 Prototype

The existing Node.js source and tests are retained as characterization
evidence. They are not the active V2 implementation and must not be reset,
deleted, dual-written, or presented as a completed Rust/PostgreSQL platform.

The current worktree is the intentional cumulative MVP-0-through-TASK-021 result
plus TASK-022 governance and TDD work. It is not a clean per-ticket diff and
must not be reset, cleaned, or switched. The preserved Node verification passed
44/44 at TASK-021 closure; that result is a compatibility baseline, not
TASK-022 acceptance.

## Verification

```text
npm.cmd run verify
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
powershell -File scripts/run-task019-postgres.ps1
```

These commands describe the completed TASK-021 verification baseline. TASK-022
has passed governance re-review and must now rerun its own focused, full, and
marker-owned PostgreSQL matrices after implementation. Live Project Registry
inspection, Workspace Git/Scope Check, memory, providers, daemon activation,
production, release, deployment, and A/B rollback remain future work with
explicit capability gates.
