# TASK-084 Plan

## Goal

Deliver a versioned, fail-closed workflow-learning core that can replay the
2026-08-21 zero-work misclassification, challenge a foreman before it stops,
promote only machine-gated policy candidates, and roll back regressions.

## Global Strategy

Keep the first slice pure, injectable, and independent of the uncommitted
TASK-079 implementation. Freeze the specification, module boundary, threat
model, historical fixtures, and characterization contract on base `47bf0a5`.
Defer Task Ledger/PostgreSQL/Writer Lease composition until TASK-079 publishes a
clean versioned contract.

## Non-Goals

- No scheduler, worker/process control, dashboard mutation, MCP expansion, or
  live PostgreSQL/TASK-051 acceptance.
- No model-authored rule code and no learned expansion of protected authority.
- No edits to TASK-079 or any other dirty worktree.

## Scope

- `crates/lattice-workflow-learning/**`
- TASK-084 specification, constitution, ADR, threat model, ticket, reviews,
  task-specific plan, and task-specific handoff.

## Confirmed Facts

- `47bf0a5` contains clean committed ancestry for TASK-078 `5a5da01`, TASK-048
  `180a269`, and TASK-049 `f03fcd8`.
- TASK-079's worktree had uncommitted Cargo, plan, contract, module, and source
  changes during the TASK-084 entry audit; those changes are excluded.
- The 2026-08-21 dashboard snapshot was partial and the TASK-080 audit found
  ticket/PLANS/dashboard/Git contradictions plus immediately actionable work.
- The fixed foreman incorrectly treated many `UNKNOWN`/idle signals as evidence
  of no work until the user challenged the conclusion.

## Assumptions

- A standalone pure Rust characterization crate is a useful integration input
  even though it is intentionally not yet a root-workspace member.
- TASK-079 will later provide the authoritative foreman snapshot/replay port
  used by durable TASK-084 composition.

## Open Questions

- Exact Task Ledger event/port binding remains blocked on TASK-079's committed
  public contract and architecture review.

## Implementation Steps

- [x] Step 1: audit repository, dependencies, identities, safety base, and the
  real TASK-080/foreman incident evidence.
- [x] Step 2: freeze SPEC-007, ADR-025, module constitution, threat model,
  TASK-084 ticket, and historical fixtures.
- [x] Step 3: drive the pure closed contracts through observable
  RED/GREEN characterization tests.
- [x] Step 4: run focused/full available checks and read-only code, security,
  and architecture reviews.
- [x] Step 5: create one clean local checkpoint, task-specific handoff, and
  fixed-foreman completion envelope.

## Verification

- `cargo test --manifest-path crates/lattice-workflow-learning/Cargo.toml`
- `cargo fmt --manifest-path crates/lattice-workflow-learning/Cargo.toml -- --check`
- `cargo clippy --manifest-path crates/lattice-workflow-learning/Cargo.toml --all-targets -- -D warnings`
- module-constitution validator, `npm.cmd run check`, and `git diff --check`

## Risks

- A pure characterization module can prove contract behavior but not durable
  restart recovery through PostgreSQL; this remains an explicit blocked gate.
- Shared root `PLANS.md`, `HANDOFF.md`, `Cargo.toml`, `Cargo.lock`, and branch
  guide are active TASK-079 overlap and stay untouched in this slice.

## Drift Log

- 2026-08-21: TASK-079 was found uncommitted, so durable integration was
  removed from the current slice and replaced by a standalone pure contract.
- 2026-08-21: fixed-foreman requirements added a mandatory pre-stop
  counterexample loop and versioned zero-work certificate; both became core
  acceptance behavior rather than optional monitoring.
- 2026-08-21: bounded implementation/reviews pass; production durability stays
  PARTIAL on TASK-079, and the shared `npm check` CURRENT-marker failure is not
  repaired from this isolated branch.
