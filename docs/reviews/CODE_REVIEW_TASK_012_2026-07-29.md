# TASK-012 Independent Code And Security Review

## Target

- Worktree: `lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Specification: SPEC-002 version 8
- Ticket: TASK-012
- Active contracts: Project Registry 1.1, Contracts 1.2, Policy 2.3
- Reviewers: independent read-only code and security subagents

## Review RED Findings

Independent review did not accept the first GREEN implementation. Each
actionable finding was reproduced with a failing regression before repair:

- arbitrary producer ID/version could substitute for the Registry owner;
- the original authority head did not mirror all security-relevant receipt
  fields, so a receipt field could differ while a narrower head still matched;
- using `receipt.head()` as both receipt projection and alleged current head
  did not establish currentness;
- an authoritative observation collision could leave the observed project
  `ACTIVE`, or later governance incorrectly described all duplicate outcomes as
  zero-mutation denials;
- a pending observation did not reserve its identity, so another registration
  could front-run reconciliation;
- canonical hashing normalized text after semantic acceptance, allowing
  non-NFC command/root/ref aliases to share bytes without sharing validation;
- broad uppercase-name rejection incorrectly denied valid branches such as
  `WIP` and `RELEASE_2026`;
- governance described every command as carrying only an expected revision,
  although register has no prior head and observe/suspend/reconcile bind the
  full expected head.

## Resolutions

- Contracts fixes producer ID `lattice-project-registry` and semantic producer
  version `1.0`; unsupported substitutions fail construction.
- `ProjectAuthorityHead` mirrors producer/version, runtime,
  project/snapshot, revision, lifecycle/class, primary ref, observation digest,
  and receipt digest. Policy compares the complete current head.
- `receipt.head()` is explicitly a structural projection. Policy facts require
  a head from an independent current Registry-owner lookup; the fake Registry
  exposes such a lookup for composition tests.
- Ordinary registration/reconciliation collisions return `Denied` without
  mutation. An authoritative cross-project observation returns the distinct
  hashed `Blocked` outcome, advances the observed project to a new
  `SUSPENDED` head, clears its colliding pending observation, and does not
  steal the other identity.
- Accepted identities are checked before pending reservations. The first
  non-colliding pending observation reserves its identity, preventing
  front-running while preserving exact reactivation/reconciliation paths.
- Command ID, canonical-root text, and primary-ref text must already be NFC
  before request hashing or mutation.
- Git reference validation uses an explicit pseudo-ref denylist and preserves
  valid uppercase branch names.
- SPEC-002 v8, ADR-010, TASK-012, module routing, and versioned constitutions
  now distinguish command/read receipts, full expected heads, `Denied`, and
  defensive `Blocked` semantics exactly.

## Final Results

Code review: `PASS`, no P1, P2, or P3 finding.

Security review: `PASS`, no P1, P2, or P3 finding.

Governance rescan: `PASS`, no active version or semantic inconsistency.

Independent and main-agent evidence:

- Contracts: 11 tests pass;
- Project Registry: 16 tests pass;
- Policy: 3 unit + 7 contract + 52 matrix + 8 V1 compatibility = 70 tests;
- full Rust workspace: 118 tests pass;
- preserved Node suite: 38 tests pass;
- selected constitution validator: 3 valid, zero warning/error;
- final project check: `check=ok files=146 constitutions=13`;
- locked all-target/all-feature Clippy with `-D warnings`: pass;
- `cargo fmt --all -- --check`: pass;
- approved Cargo dependency trees: pass;
- forbidden Registry/Policy/Contracts I/O scan: zero matches;
- `git diff --check`: pass.

## Documented Residual

A pure Policy function cannot determine that a self-consistent historical
receipt/head pair is stale without independently receiving the latest owner
head. TASK-012 documents and tests the composition rule but does not
authenticate or durably serialize that lookup. The real
Orchestrator/PostgreSQL owner path must provide this evidence and reject a
historical active pair against the latest head before live authority exists.
This is a future fail-closed integration gate, not a remaining TASK-012 defect.
