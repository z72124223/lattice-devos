# TASK-012 Architecture Review

## Triggers

- New Project Registry owner module and public lifecycle/receipt contract.
- Material Contracts 1.2 and Policy 2.3 public-contract amendments.
- Cross-project identity reservation and defensive state-transition semantics.
- Registry/Policy dependency-cycle risk.
- One Gateway, One Truth, One Writer, project isolation, and MVP-3 safety.

## Independent Result

`PASS`. No remaining P1, P2, or P3 architecture finding.

Confirmed:

- Project Registry 1.1 is the sole semantic owner of mutable project identity,
  lifecycle, accepted/pending observations, revision, snapshot lineage, drift,
  suspension, reconciliation, and receipt issuance.
- Contracts 1.2 owns immutable representation only. Policy 2.3 owns
  deterministic sufficiency/deny meaning only.
- Dependency direction remains acyclic:
  `lattice-project-registry -> lattice-contracts + lattice-cjson` and
  `lattice-policy -> lattice-task-domain + lattice-contracts`.
- `Denied` is a zero-mutation terminal outcome for ordinary
  registration/reconciliation rejection. `Blocked` is a defensive mutation
  that rotates an authoritative collision observer to `SUSPENDED` without
  taking another project's accepted or pending identity.
- Pending reservation order prevents front-running and still permits the
  collision observer to reactivate its prior accepted identity.
- NFC validation occurs before hashing/mutation; command subjects cannot
  depend on hidden normalization.
- Full authority heads cover every receipt security field. Policy requires an
  independent current owner head rather than treating a receipt projection as
  freshness proof.
- The Registry implementation performs no filesystem, Git, database, process,
  network, clock, environment, credential, provider, payment, publication,
  deployment, or product-repository I/O.
- No product-code writer, gateway, provider, store, or protected-release
  authority was introduced.

## Enforcement Truth

| Gate | Classification | Evidence |
|---|---|---|
| Pure deterministic Registry API | machine-enforced locally | Rust types and 16 Registry tests |
| Registration/resolve/drift/suspend/reconcile | machine-enforced locally | lifecycle and snapshot-lineage tests |
| Accepted/pending identity isolation | machine-enforced locally | duplicate, reservation, front-run, collision, and reactivation tests |
| `Denied` versus `Blocked` outcomes | machine-enforced locally | typed outcomes and canonical result hashes |
| NFC command/root/ref subjects | machine-enforced locally | construction/command regressions |
| Fixed producer and full security head | machine-enforced locally | Contracts and Policy substitution matrices |
| Dependency and no-I/O direction | machine-enforced locally | Cargo trees, Clippy, source scan |
| Current fake owner lookup | machine-enforced in process | `FakeProjectRegistry::current_head` plus Policy comparison |
| Authenticated/durable current-head lookup | documented-only and deferred | Orchestrator/PostgreSQL future gate |
| Real Windows/Git physical identity | deferred and fail-closed | future Workspace Git fixtures |
| PostgreSQL restart/durability | deferred and fail-closed | future store ticket |
| Remote CI/merge policy | missing/unverified | no remote configured |

## Verification

- Contracts: 11 tests pass.
- Project Registry: 16 tests pass.
- Policy: 70 tests pass.
- Full Rust workspace: 118 tests pass.
- Preserved Node: 38 tests pass.
- `cargo fmt --all -- --check`: pass.
- locked workspace Clippy with `-D warnings`: pass.
- selected constitution validation and project check: pass.
- `git diff --check`: pass.
- forbidden I/O scan: zero matches.
- locked Registry and Policy dependency trees: approved edges only.

## Residual Non-Blocking Owner Work

TASK-012 deliberately uses immutable fake observations and an in-process fake
current-head lookup. Later owner tickets must add real Windows/Git identity,
PostgreSQL serialization/durability, and authenticated Orchestrator
composition. Those gates remain explicit and cannot be replaced by the fake
evidence.
