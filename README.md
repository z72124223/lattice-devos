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

`BOOTSTRAP_READY` — the approved V2 Rust bootstrap is implemented in the
dedicated sibling worktree:

`C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`

- Active plan: `PLANS.md`
- Current charter: `docs/PROJECT_CHARTER.md`
- Direction authority: `docs/source/DIRECTION_CHANGE_2026-07-29.md`
- Ready V2 specification:
  `docs/specs/SPEC-002-autonomous-development-platform.md`
- Approved module direction: `docs/modules/V2_AMENDMENT_PROPOSAL.md`
- Completed bootstrap ticket: `docs/tickets/TASK-008-v2-rust-bootstrap.md`
- V2 audit: `docs/reviews/WORKFLOW_AUDIT_V2_REPLAN_2026-07-29.md`
- Component-contract evidence:
  `docs/reviews/COMPONENT_CONTRACT_EVIDENCE_2026-07-29.md`

The Rust workspace contains an inert component manifest, read-only status CLI,
and unexecuted PostgreSQL namespace draft. No OpenClaw, Graphify, or Hermes
installation, account action, database connection, model call, payment,
publication, deployment, push, or merge occurred.

## Quick Start

```powershell
Set-Location "C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2"
cargo run -p lattice-cli -- status
```

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

The current worktree contains pre-existing uncommitted TASK-004 security
changes. The latest V2 planning-pass attempt to run the Node verification timed
out and is not a passing result.

## Verification

```text
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

PostgreSQL, Git, live adapter, memory, and A/B rollback checks remain future
work and will use disposable local fixtures plus explicit capability gates.
