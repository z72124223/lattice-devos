# TASK-020 Workflow Audit

## Decision

`READY`. The repository, dirty-worktree boundary, current plan, TASK-019
closure, affected module constitutions, public interfaces, migration runner,
schema, tests, harness, and Git state were inspected before implementation.

## Repository And Git Evidence

- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch/HEAD: `feature/v2-rust-postgres-bootstrap` at
  `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Primary: `main` at `d856cf7eb7be0072aeb32316e43f0b653d73dca2`
- Committed primary/V2 ahead/behind: `0/4`; remote/upstream: none.
- The shared dirty MVP-0-through-TASK-019 result is intentional and must not be
  reset, cleaned, switched, overwritten, or inferred as a per-ticket Git diff.

## Reused And Updated Workflow Stages

| Stage | Classification | Evidence or action |
|---|---|---|
| Repository inspection | valid/current | Git, AGENTS, PLANS, HANDOFF, ledger, package manifests, schema, tests, and harness read |
| Requirements clarification | valid/current | user authorized continued MVP-3 execution; ADR-016/017 and prior constitutions fix domain/store/activation boundaries; no material question remains |
| Specification | updated/current | SPEC-002 v22 AC-34 is observable and bounded |
| Module governance | updated/current | Contracts 1.9, Ports 1.4, Postgres Store 1.2; ADR-018 records the material tradeoffs |
| Ticket decomposition | current | one non-parallel TASK-020 with exact allowlist and TASK-019 dependency |
| Branch/worktree plan | reused | existing feature branch/shared worktree; no branch mutation authorized |
| TDD | pending | first implementation action must be a focused failing test |
| Focused/full verification | pending | commands and disposable evidence frozen in TASK-020 |
| Code/security review | pending | independent review required after GREEN/refactor |
| Architecture review | pending | required for three module versions, migration, and SECURITY DEFINER surface |
| Integration | pending | local combined verification required; remote CI remains missing |
| Handoff/ledger | pending | required before ticket completion |
| Merge | blocked | no committed candidate, remote policy/CI, or primary merge authorization |

## Key Evidence And Conflicts Resolved

- Contracts 1.8 can construct only Fake/NonDurableFake receipts; 1.9 is required
  before a truthful live/durable implementation.
- Ports 1.3 uses `current_head(&self)` while the synchronous PostgreSQL client
  query is mutable; 1.4 makes that mutation explicit rather than hiding it.
- `0002` intentionally grants no runtime physical/terminal table access and
  forbids owned functions; ADR-017 requires narrow fixed SECURITY DEFINER
  operations. `0003` plus a versioned verifier is therefore mandatory.
- The runner previously handled only fresh apply/full no-op. TASK-020 must
  verify exact history prefixes and apply only missing entries before a v1-to-v2
  expansion can be claimed.
- TASK-020 is physical Store only. Durable Ledger/outbox, Registry, Lease,
  Approval, and Artifact work remains decomposed into later tickets.
- Marker-owned PostgreSQL evidence may use a test-admin ACTIVE fixture. Normal
  runtime still has no activation path and no production target is authorized.

## Enforcement Truth

- Local Rust/Node tests, manifest/catalog/ACL assertions, and the owned
  PostgreSQL harness can be machine-enforced.
- The ticket allowlist, module ownership, TDD order, and protected boundaries
  are documented and locally checked but not enforced by a remote merge system.
- Remote Rust CI, required remote review, branch protection, merge queue,
  committed candidate, and `cargo-audit` availability are missing/unverified.
- This missing remote enforcement does not block continued reversible local
  implementation; it does block merge-readiness claims.
