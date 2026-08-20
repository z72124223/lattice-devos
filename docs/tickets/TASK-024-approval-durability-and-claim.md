---
ticket_id: TASK-024
spec_id: SPEC-002
spec_version: 36
module_id: approval-verifier
constitution_version: 1.1
status: complete
parallel_safe: false
depends_on:
  - TASK-015
  - TASK-023
  - commit:78b4e2e
allowed_paths:
  - docs/tickets/TASK-024-approval-durability-and-claim.md
  - docs/tickets/red-fixtures/TASK-024-approval-claim-durability-red.json
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-024-durable-postgres-approval-normal-claim.md
  - docs/modules/approval-verifier/MODULE_CONSTITUTION.md
  - docs/modules/postgres-approval-verifier/MODULE_CONSTITUTION.md
  - PLANS.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-approval-verifier/src/lib.rs
  - crates/lattice-approval-verifier/tests/approval_verifier.rs
  - crates/lattice-postgres-approval-verifier/Cargo.toml
  - crates/lattice-postgres-approval-verifier/src/lib.rs
  - crates/lattice-postgres-approval-verifier/src/setup.rs
  - crates/lattice-postgres-approval-verifier/src/adapter.rs
  - crates/lattice-postgres-approval-verifier/tests/adapter_api.rs
  - crates/lattice-postgres-approval-verifier/tests/extension_contract.rs
  - crates/lattice-postgres-approval-verifier/tests/postgres_live.rs
  - db/extensions/approval-verifier/v1.sql
  - scripts/run-task019-postgres.ps1
  - scripts/test-task024-postgres-approval-verifier.ps1
branch: feature/task-023-025-durable-repositories
base_commit: 845328dcc06d51c7554c93a09739a27ddd827941
---

# TASK-024 Approval durability and atomic claim

## Corrected mapping

TASK-024 is the durable Approval Verifier repository and claim slot. Approval
Verifier remains the sole semantic owner of subject, challenge, proof, nonce
binding, time interval, availability, revocation, exact retry, replay and claim
preconditions. PostgreSQL owns only atomic currentness, database-time, global
nonce uniqueness, claim persistence, transaction serialization and restart
evidence.

Normal claim and protected Guardian claim are different transactions:

- a normal approval is claimed atomically by the transaction that performs or
  claims the exact approved task transition/effect;
- a protected release remains `VERIFIED_PROTECTED_PENDING_CLAIM` here and may
  be claimed only by the future Guardian-only `claim_activation` transaction,
  atomically with `ACTIVATION_CLAIMED` and admission changing to `DRAINING`.

No normal repository API may expose a general protected consume command.

## Current-state matrix

| Acceptance surface | Status | Evidence and boundary |
|---|---|---|
| Pure typed subject/proof/nonce/time owner | complete | TASK-015, ADR-013 and Approval Verifier 1.0 |
| Pure normal-claim planning | complete | `ConsumeNormalApprovalCommand` and protected-lane denial exist |
| Protected claim separation | complete | pure owner exposes protected pending state but no general protected consume |
| Pure replay/checkpoint | complete | public untrusted replay and independent checkpoint comparison exist |
| Canonical repository bytes and repository trait | complete | Approval Verifier 1.1 canonical requests, receipts, snapshots, checkpoints, and component-free repository trait |
| PostgreSQL currentness/nonce/claim repository | complete | independent v1 extension and typed adapter pass exact install/no-op/live/restart verification |
| Same-transaction normal effect claim | complete | live serializable claim persists one domain consume and exact immutable effect row atomically |
| Guardian-only protected claim | blocked | belongs to the later Guardian transaction, not this normal claim adapter |
| Stable physical profile convention | complete | exact embedded SQL/manifest plus ten catalog/ACL profile signatures and namespace closure |

## Ready gate and dependency order

1. TASK-038 stable checkpoint.
2. TASK-023 crosswalk closes or records only non-overlapping residual gaps.
3. Freeze an Approval Verifier 1.1 repository contract and exact physical
   profile in a versioned SPEC/ADR/constitution amendment.
4. Only then expand `allowed_paths`; no implementation path is pre-authorized
   by this readiness ticket.

## Ready gate resolution on 2026-08-20

TASK-023 completed at local checkpoint `78b4e2e` after the clean TASK-076
dependency and a current marker-owned PostgreSQL acceptance rerun. SPEC-002
v36, ADR-024, Approval Verifier 1.1, and Postgres Approval Verifier 1.0 now
freeze the repository bytes, global aggregate serialization, database-time
observation, independently retained checkpoint, fixed-function physical
profile, normal effect-claim transaction, and protected-lane exclusion.
The exact paths above replace the former readiness-only allowlist. The first
implementation action is the existing repository-trait RED; only TASK-024 is
current.

The physical adapter may depend one-way on Approval Verifier public
planner/replay/checkpoint APIs. Approval Verifier may never depend on a
database crate, Guardian, Policy, Orchestrator or another concrete adapter.

## Acceptance criteria after the ready gate

- [x] Approval Verifier exports bounded canonical snapshot/checkpoint bytes and
  one component-free repository contract reused by fake/conformance and live
  persistence without changing 1.0 subject/proof/receipt meaning.
- [x] A live repository persists challenge, verification, nonce binding,
  revocation, current head, command receipts and an independently retained
  checkpoint atomically or not at all.
- [x] Global nonce uniqueness holds across approvals/projects/lanes and after
  denial, expiry, claim or revocation; raw nonce/token/key/assertion bytes are
  never persisted, logged or returned.
- [x] Normal claim rechecks exact receipt/head, subject, trust lane, status,
  database time, nonce availability, daemon epoch/admission and exact effect or
  transition claim in one transaction.
- [x] Two concurrent normal claimers yield one applied claim and one durable
  terminal denial. Exact retry is byte-identical; changed claim content denies.
- [x] A successful normal claim makes later current-head lookup unavailable and
  replays identically after a new connection/process and database restart.
- [x] Normal claim rejects every protected approval. Protected state remains
  pending and unchanged; only a separately authenticated Guardian composition
  may call its dedicated protected claim transaction.
- [x] Commit-response uncertainty returns no approval/effect success and
  reconciles only through fresh current-state replay plus exact retry.
- [x] The physical layer uses fixed functions and exact verified catalog/ACL
  closure; no direct table DML, arbitrary SQL, DSN, credential, subject
  reconstruction or caller Boolean is exposed.

## RED fixture and first executable RED

The data-only fixture is
`docs/tickets/red-fixtures/TASK-024-approval-claim-durability-red.json`.

The first light readiness RED is:

```powershell
$source = Get-Content -LiteralPath 'crates/lattice-approval-verifier/src/lib.rs' -Raw
if ($source -notmatch 'pub trait Approval[^\r\n]*Repository') { exit 1 }
```

The readiness RED failed before Approval Verifier 1.1 existed. The subsequent
behavioral RED/GREEN sequence covered canonical repository bytes, live setup,
normal claim, two concurrent claimers, protected denial, physical corruption,
and a loopback proxy that discards the server's successful `COMMIT` response.

## Completion evidence on 2026-08-20

- `scripts/test-task024-postgres-approval-verifier.ps1` returned
  `TASK024_APPROVAL_DURABILITY_ACCEPTANCE=PASS`.
- The marker-owned PostgreSQL 17.10 fixture completed install, exact no-op,
  independent process restart, replay, strict catalog/ACL closure, concurrent
  claim, protected denial, commit-response uncertainty, and contained cleanup.
- The verified 12-event holder receipt is
  `target/task019-holder-receipts/560a1967fa6447bf8b34f0b0c2a38479.jsonl`,
  raw SHA-256
  `5c829e19580b990212479f3d44458df446a7551c38fd4615a27bbef7283e7d00`.
- Strict package Clippy, package tests, format, project check, and diff check
  passed inside the official wrapper. Push, primary merge, deployment, and
  release were not performed.

## Scope that remained forbidden

- PostgreSQL must not decide subject/proof/nonce/revocation/claim semantics.
- No normal protected-release consume command and no Guardian authority inside
  the normal adapter.
- No production/user database, arbitrary target, WSL, push, primary merge,
  deployment, release, public exposure, credential change, or permanent
  deletion was used.
