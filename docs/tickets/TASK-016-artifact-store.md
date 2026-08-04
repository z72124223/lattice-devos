---
ticket_id: TASK-016
spec_id: SPEC-002
spec_version: 12
module_id: artifact-store
constitution_version: 1.0
status: complete
parallel_safe: false
depends_on:
  - TASK-015
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/**
  - crates/lattice-artifact-store/**
  - PLANS.md
  - HANDOFF.md
  - docs/PROJECT_CHARTER.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-014-project-scoped-artifact-identity-provenance-and-sweep.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/lattice-contracts/**
  - docs/modules/artifact-store/**
  - docs/tickets/TASK-016-artifact-store.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_016_2026-07-30.md
  - docs/reviews/CODE_REVIEW_TASK_016_2026-07-30.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_016_2026-07-30.md
  - docs/reviews/INTEGRATION_TASK_016_2026-07-30.md
likely_files:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-contracts/src/lib.rs
  - crates/lattice-contracts/tests/contracts.rs
  - crates/lattice-artifact-store/Cargo.toml
  - crates/lattice-artifact-store/src/lib.rs
  - crates/lattice-artifact-store/tests/artifact_store.rs
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Implement Artifact Store 1.0 as the pure owner of project-scoped
content-addressed object identity, immutable provenance references,
byte-digest/aggregate-quota verification, exact command idempotency,
generation and availability, replay/currentness, typed reference authority,
retention, durable-delete-claim/unknown-outcome semantics, and safe sweep
planning. Supply a visibly non-durable in-memory fake without filesystem or
PostgreSQL I/O.

## Acceptance Criteria

- [x] Contracts 1.6 provides neutral immutable artifact object/reference,
  provenance, purpose, availability, positive signed-BIGINT-compatible
  generation/revision/byte-length, fixed producer, receipt, and complete head
  representations.
- [x] Object identity is exactly project-scoped `(project_id, sha256)`.
  Cross-project deduplication, existence responses, reference sets, or
  lifecycle sharing are unrepresentable in the public owner API.
- [x] Every reference binds the complete project/snapshot/task/revision/spec/
  attempt/request, object/generation, media/schema, producer/version/runtime,
  producer/adapter binary, invocation/correlation/run/sequence/produced-at/payload,
  capability/input/config/evidence, Registry authority, exact effect claim,
  daemon instance/epoch/admission, capability-owner receipt/current head,
  hash-bound limit snapshot, purpose, and retention fields.
- [x] Artifact Store receipts use fixed
  `lattice-artifact-store`/`1.0` identity and cannot be produced or
  cross-labeled by Graphify, Hermes, Codex, a model, Guardian, or product
  repository.
- [x] Raw bytes are verified against declared length and SHA-256 before
  publication; empty bytes and exact configured limit pass, while over-limit
  input and digest/length mismatch deny with zero partial mutation.
- [x] Hard bounds cover 1 GiB/object, 64 KiB/canonical manifest, bounded
  identifier/media/schema/producer fields, bundle entries/depth/bytes,
  per-object active references, per-task objects/references/active bytes/
  staging bytes/streams, and per-project objects/references/unique bytes.
  Configuration may lower but never raise them; every command binds an
  independently loaded immutable limit-snapshot digest and checked quota
  deltas update atomically with state.
- [x] Same-project equal bytes reuse one available generation while preserving
  separate immutable references. Cross-project equal bytes remain isolated.
- [x] Initial publication/reference, retain, and release each require a typed
  fixed-owner authority receipt plus an independently queried complete current
  owner head bound to the exact owner record/revision/status,
  `PUBLISH_INITIAL_REFERENCE`/`ADD_REFERENCE`/`RELEASE_REFERENCE` action,
  project/task/object/generation/reference, and fake/live runtime. No caller
  count, Boolean, producer string, or bare digest authorizes mutation. The
  TASK-016 fake accepts only visibly fake owner authority.
- [x] Reference release is exact and terminal. Add/release changes the object
  revision, quota projection, and active-reference-set digest; released IDs
  cannot be rebound.
- [x] Exact command retry returns the identical applied or denied terminal
  receipt before stale-head/time checks under exact key
  `(project_id, algorithm, content_digest, command_id)`. Changed content under
  one key rejects permanently; another project/object key is independent.
- [x] Every durable fake command record retains the complete sanitized
  canonical request source, request digest, storage key, and terminal receipt
  without raw bytes. Request/object/reference/head/receipt/checkpoint/delete
  plan/claim/result use separate hash domains.
- [x] Delete claim requires an exact independently obtained object head,
  exact generation, internally recomputed zero references/quota projection,
  expired retention/grace, explicit valid time, current daemon/epoch/admission/
  root binding, and a typed fixed-owner sweep receipt/current-head pair. A
  retain race or stale plan denies.
- [x] The lifecycle is `AVAILABLE -> DELETE_CLAIMED -> DELETED |
  AVAILABLE(verified no effect) | RECONCILIATION_REQUIRED`. A unique exact
  claim token blocks retain/normal read, retries idempotently, and an unknown
  transaction/filesystem outcome never guesses success or safety.
- [x] `RECONCILIATION_REQUIRED` returns to `AVAILABLE` or `DELETED` only from
  verified metadata plus exact owned-byte/digest evidence.
- [x] Project/store bytes count each non-deleted generation once; task active
  bytes count one object once when that task has any active reference; active
  reference/read/staging/command/history counters count exact identities or
  canonical bytes. Object, task, project, and store quota aggregates update
  atomically and have independent checkpoint/current-head evidence.
- [x] `DELETE_CLAIMED`, `RECONCILIATION_REQUIRED`, and sealed orphans retain
  worst-case object/byte/staging quota. Object quota releases only on verified
  `DELETED`; staging releases only after authoritative metadata publication or
  verified cleanup/reconciliation. Unknown never frees quota.
- [x] Active reads use typed-authority object-scoped acquire/release commands,
  exact retry, and hard object/task/project/store counts. Reaching the maximum
  15-minute lease creates `EXPIRED_SUSPECT`; it remains quota/delete blocking
  until verified holder-death or handle-closure reconciliation.
- [x] Reintroduction after a simulated deletion allocates a higher
  non-wrapping generation and rejects old sweep/reference evidence.
- [x] Public raw replay rejects unknown versions/kinds, malformed values,
  tamper, reorder, truncation, duplication, orphan references,
  cross-project/object/generation substitution, fake/live mixing,
  reference-set disagreement, and receipt-chain/high-water disagreement.
- [x] Rollback-sensitive restore requires an independently retained validated
  checkpoint binding delete claim/reconciliation and quota projections, and
  rejects a coherent older prefix.
- [x] Raw bytes never enter command receipts, metadata snapshots, errors, or
  `Debug`; fake byte storage remains separate and is verified on every read.
- [x] The crate has no Policy, Task Domain, Ledger, Registry, Writer,
  Approval, ports/store, filesystem, database, Git, process, network,
  environment, credential, provider, payment, publication, deployment,
  Guardian, or product-repository dependency/I/O.
- [x] Real PostgreSQL reference authority, filesystem staging/flush/rename/
  link containment/unlink, provider staging, durability/restart, and live
  cleanup remain explicitly open.
- [x] Later live filesystem gates are frozen: root physical identity and
  product-root separation; Windows reparse/junction/symlink/hardlink/ADS/
  device/non-regular denial; case-fold collision; same-volume exclusive
  staging; no-clobber rename; directory flush; handle identity/TOCTOU;
  bounded bundle normalization; read/delete serialization; exact unlink; and
  orphan quarantine without directory-to-authority promotion.

## Non-Goals

- Install or invoke OpenClaw, Codex, Graphify, Hermes, PostgreSQL, or a model.
- Store or read a real file, directory, database row, credential, provider
  session, or product-repository path.
- Decide producer authenticity, factual trust, independent review, memory
  promotion, code acceptance, approval, or release.
- Implement a live `ArtifactStagingPort`, PostgreSQL migration, filesystem
  adapter, process sandbox, publication, deployment, or protected action.
- Commit, push, merge, or activate a release.

## Module And Constitution Constraints

- Artifact Store 1.0 owns pure manifest/byte/reference/quota/currentness/
  delete-claim/reconciliation/sweep semantics and depends only on Contracts
  1.6, cjson, exact SHA-256, and canonical time mechanics.
- Contracts 1.6 owns immutable representation only.
- The fake proves binding and lifecycle only; it is neither durable truth nor
  evidence of safe filesystem deletion.
- Future PostgreSQL and filesystem adapters must reuse rather than duplicate
  the semantic owner.

## Dependencies And Overlap

`parallel_safe: false`: this ticket materially changes shared Contracts public
types and the workspace lockfile. No other ticket may modify these paths or
interfaces concurrently.

## TDD Behaviors

1. RED/GREEN: shared artifact values are absent, then reject every malformed
   identifier/bound/producer/runtime/receipt/head substitution.
2. RED/GREEN: Artifact Store crate/fake is absent, then publish and read one
   empty and one non-empty exact digest-bound object.
3. RED/GREEN: declared length/digest, manifest/object/reference/read/task/
   project/store/staging/command/history hard bounds, checked aggregate quota,
   exact unique/per-task accounting, worst-case claim/orphan retention, and
   zero-partial mutation.
4. RED/GREEN: same-project deduplication and cross-project/object/generation
   isolation.
5. RED/GREEN: complete provenance/reference field mutation matrix and provider
   non-authority, including Registry/effect/daemon/admission/capability/
   adapter/limit-snapshot binding.
6. RED/GREEN: exact scoped retry key, complete canonical denied request
   retention, separate hash domains, changed command, stale expected head, and
   terminal denied receipt chain.
7. RED/GREEN: typed initial-publish/retain/release owner receipt/current-head
   authority, action/scope/runtime substitution, terminal reference reuse, and
   quota updates.
8. RED/GREEN: zero-reference/retention/grace typed delete claim, retain/read
   block, exact token retry, stale plan, verified no-effect, unknown outcome,
   reconciliation, simulated deletion, and higher-generation reintroduction.
9. RED/GREEN: typed read acquire/release, exact retry, hard counts,
   expiry-suspect, holder reconciliation, and fake-backend missing/corrupt
   bytes without changing metadata authority.
10. RED/GREEN: raw metadata replay, denial-tail truncation, delete-claim/quota
    substitution, and coherent rollback against a trusted checkpoint.
11. REVIEW RED/GREEN: every actionable independent finding receives a failing
    regression before repair.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Shared contracts | `cargo test -p lattice-contracts --locked` | complete artifact values and substitution matrix |
| Artifact behavior | `cargo test -p lattice-artifact-store --locked` | identity/provenance/bytes/reference/retry/replay/sweep matrices |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Full Rust suite | `cargo test --workspace --all-targets --all-features --locked` | exit 0 |
| Preserved Node suite | `npm.cmd run verify` | exit 0 |
| Dependencies | `cargo tree -p lattice-artifact-store --edges normal --locked` | only contracts/cjson/hash/time approved edges |
| Scope/secrets | forbidden-I/O/raw-byte/provider/product scans plus `git diff --check` | zero forbidden source matches; exit 0 |

## Human Gate

None for this bounded pure/fake local implementation. Credentials/account/
payment actions, public exposure, irreversible real deletion,
security-control changes, provider installation, live PostgreSQL/filesystem
activation, real product effects, protected release, primary-branch merge,
publication, and deployment remain outside this ticket.
