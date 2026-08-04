---
ticket_id: TASK-010
spec_id: SPEC-002
spec_version: 4
module_id: lattice-cjson
constitution_version: 1.0
additional_modules:
  - module_id: task-domain
    constitution_version: 2.0
status: completed
parallel_safe: false
depends_on:
  - TASK-009
allowed_paths:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-cjson/**
  - crates/lattice-task-domain/**
  - PLANS.md
  - HANDOFF.md
  - docs/specs/SPEC-002-autonomous-development-platform.md
  - docs/adr/ADR-008-canonical-encoding-ownership.md
  - docs/modules/V2_AMENDMENT_PROPOSAL.md
  - docs/modules/README.md
  - docs/modules/lattice-cjson/**
  - docs/modules/task-domain/**
  - docs/tickets/TASK-010-task-domain-cjson.md
  - docs/workflow/WORKFLOW_LEDGER.md
  - docs/reviews/WORKFLOW_AUDIT_TASK_010_2026-07-29.md
  - docs/reviews/CODE_REVIEW_TASK_010_2026-07-29.md
  - docs/reviews/ARCHITECTURE_REVIEW_TASK_010_2026-07-29.md
  - docs/reviews/INTEGRATION_TASK_010_2026-07-29.md
likely_files:
  - Cargo.toml
  - Cargo.lock
  - crates/lattice-cjson/Cargo.toml
  - crates/lattice-cjson/src/lib.rs
  - crates/lattice-cjson/tests/canonical_bytes.rs
  - crates/lattice-task-domain/Cargo.toml
  - crates/lattice-task-domain/src/lib.rs
  - crates/lattice-task-domain/tests/task_domain.rs
branch: feature/v2-rust-postgres-bootstrap
---

## Objective

Implement the first pure Rust Task Domain V2 slice and freeze the reusable
`lattice-cjson-1` byte/hash mechanism required before any durable store,
approval, gateway, or provider fake can safely bind immutable subjects.

## Acceptance Criteria

- [x] `lattice-cjson` accepts only null, Boolean, string, array, and
  duplicate-preserving object input; it normalizes Unicode NFC, sorts
  normalized UTF-8 keys, minimally escapes JSON, and rejects normalized-key
  collisions.
- [x] `lattice-hash-1` uses the exact ADR-008 length-prefixed algorithm,
  canonical algorithm, schema ID/version, payload length, and payload bytes;
  schema/version changes produce different digests.
- [x] Raw numeric JSON values are impossible through the canonical value type.
  Task Domain validates integer, decimal, and timestamp strings before
  canonicalization.
- [x] Task Spec V2 owns every immutable project/snapshot/base/goal/scope/
  acceptance/check/capability/budget/runtime/network/deployment/approval field,
  rejects malformed or duplicate values, and hashes exactly those fields under
  `lattice.task-spec/2.0`.
- [x] Mutable task status, approvals, evidence, events, and projections cannot
  enter `spec_hash`; changing each immutable approval-relevant field changes
  the hash.
- [x] The complete 14-state V1 transition matrix remains a read-only
  characterization contract with stable unknown-state and illegal-transition
  errors; V2 does not hard-code a human actor as the only way to satisfy an
  approval transition.
- [x] Stable DAG validation rejects unknown dependencies, self-cycles, and
  multi-node cycles.
- [x] Both crates are I/O-free, non-publishable, and use only exact approved
  dependencies. No PostgreSQL, filesystem, process, network, provider,
  credential, model, or product-repository operation occurs.

## Non-Goals

- Implement Task Ledger persistence/replay, Policy, PostgreSQL, approvals,
  artifacts, gateway IPC, orchestration, or any fake/live provider.
- Define event/receipt, memory, approval, or release hash subjects.
- Treat the historical V1 JavaScript hash as V2 or silently approve a V1
  compatibility manifest.
- Create a mutable Task Packet truth or read the clock.

## Module And Constitution Constraints

- `lattice-cjson` 1.0 owns byte mechanics only, per ADR-008.
- `task-domain` 2.0 owns Task Spec V2 semantics and hash-subject selection.
- `lattice-contracts` remains unchanged and serialization/hashing-free.
- Task Ledger retains future event/receipt canonicalization semantics.

## Frozen Algorithm Decisions

- Canonical strings and keys use Unicode NFC.
- Object keys sort by normalized UTF-8 bytes; arrays preserve order.
- Only quote, reverse solidus, standard short control escapes, and lowercase
  `\u00xx` for remaining U+0000–U+001F are escaped.
- Slash, U+2028, U+2029, and other Unicode remain literal UTF-8.
- Absent and explicit `null` differ.
- The exact hash frame is the binary format in ADR-008.
- Schema numeric values use validated canonical strings; no exponent or raw
  float enters a hash.

## TDD Behaviors

1. RED: canonical byte/framing tests fail because `lattice-cjson` does not
   exist. GREEN: byte, collision, escaping, length, and digest fixtures pass.
2. RED: Task Domain tests fail because Task Spec V2, normalized scalar types,
   transitions, and DAG validation do not exist. GREEN: valid subjects are
   stable and invalid/unknown inputs fail closed.
3. RED: immutable-field mutation tests prove a missing field binding if any
   changed input reuses a digest. GREEN: every named immutable field family
   changes `spec_hash`, while packet status is outside the API.
4. REVIEW RED/GREEN: every actionable independent review finding receives a
   failing regression test before repair.

## V1 Characterization Boundary

The preserved Node fixture currently computes 1081 canonical bytes and
SHA-256
`88e9f8502132b7216bb0d4a1080c32429a1e982e6a80d572654ba1dd5a21da51`.
This is current characterization evidence, not an approved compatibility
manifest. TASK-010 freezes the V1 transition matrix and proves V1/V2 hash-path
separation; it does not claim full V1 hash-import compatibility.

## Dependencies

- `sha2 = 0.11.0`, MIT OR Apache-2.0, Rust 1.85 minimum.
- `unicode-normalization = 0.1.25`, MIT OR Apache-2.0, Rust 1.36 minimum.
- `time = 0.3.54`, MIT OR Apache-2.0, Rust 1.88 minimum, parsing/formatting only.
- Local edge:
  `lattice-task-domain -> lattice-contracts + lattice-cjson`.

All three versions were resolved from crates.io metadata on 2026-07-29 and fit
the workspace's Rust 1.97 toolchain. `Cargo.lock` must retain exact resolved
transitives.

## Verification

| Check | Command or service | Expected evidence |
|---|---|---|
| Canonical bytes | `cargo test -p lattice-cjson` | all frozen byte/hash/error fixtures pass |
| Task Domain | `cargo test -p lattice-task-domain` | spec/state/DAG behaviors pass |
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Full Rust suite | `cargo test --workspace --locked` | exit 0 |
| Preserved Node suite | `npm.cmd run verify` | exit 0 |
| Dependencies | `cargo metadata --locked --format-version 1` plus source scan | only approved edges and no I/O dependency |
| Scope/hygiene | exact allowed-path audit plus `git diff --check` | zero foreign change; exit 0 |

## Human Gate

None for this bounded local implementation and exact Rust dependency setup.
Credentials, account/payment actions, public exposure, irreversible actions,
live providers, primary-branch merge, publication, and deployment are outside
this ticket and remain protected.

## Completion Evidence

- Initial canonical-byte and Task Domain tests failed because their public
  contracts did not exist, then passed after implementation.
- Independent review regressions reproduced and fixed wire-order drift, unsafe
  Git refs and Windows aliases, loose timestamp syntax, the real RFC 3339
  leap-second collision, and missing DAG self-cycle evidence.
- `cargo fmt --check` and locked all-target/all-feature Clippy with
  `-D warnings` pass.
- `lattice-cjson`: 8 tests pass.
- `lattice-task-domain`: 6 tests pass.
- Full Rust workspace: 28 tests pass.
- Preserved Node suite: 38 tests pass.
- Project check, locked dependency inspection, zero-I/O source scan,
  SPEC/proposal parity, and `git diff --check` pass.
- Independent code review reports `No findings`.
- Independent architecture review reports no blocker.
- Local combined integration passes; remote CI and merge readiness remain
  separately missing because there is no committed candidate or remote.

Residual resource limits for recursive canonical values and DAGs are assigned
to a future untrusted-input/Policy boundary before wire ingestion. TASK-010
does not expose a wire parser.
