# TASK-011 Architecture Review

## Triggers

- New pure Policy Engine V2 public contract and dependency edge.
- Material Task Domain 2.1 budget-contract amendment.
- Merge, recovery, resource, Guardian, and rollback authority boundaries.
- One Gateway, One Truth, One Writer, project isolation, and MVP-3 safety.

## Independent Result

`PASS`. No remaining P0 through P3 architecture finding.

Confirmed:

- `lattice-policy` owns deterministic allow/deny meaning only and consumes a
  complete immutable Task Spec plus typed facts.
- Dependency direction remains
  `lattice-policy -> lattice-task-domain + lattice-contracts`; no store,
  adapter, runtime, Git, network, process, clock, or environment dependency
  entered Policy.
- Normal recovery carries a typed resolved effect, holder-death, or
  replaced-leadership result and can move only to `STOPPED`.
- Restoring `ACTIVE` is Guardian-only and binds the exact producer, protected
  activation receipt, and mutually consistent saga/database/boot
  release/manifest/slot/epoch evidence.
- Rollback carries the exact failed activation receipt, reverses source/target
  slots, advances to a strictly newer epoch, and admits no migration.
- Registry and Workspace Git own physical `GitRefIdentity` evidence. Policy
  classifies primary by matching identity digests, closing Windows `main/Main`
  aliases without collapsing distinct refs on case-sensitive storage.
- Resource facts bind their owner, Task Spec, independent expected Ledger
  stream/head/revision/effect claim, freshness, and accounting currency.
- Governance routing agrees on SPEC-002 v6, Task Domain 2.1, and Policy Engine
  2.1.

## Enforcement Truth

| Gate | Classification | Evidence |
|---|---|---|
| Pure deterministic Policy API | machine-enforced | Rust types, closed enums, 66 tests |
| Exact subject/substitution denials | machine-enforced locally | matrix regressions |
| Recovery/Guardian/rollback restrictions | machine-enforced locally | typed subjects and transition tests |
| Git physical-ref identity classification | machine-enforced at Policy consumption | identity comparison and alias tests |
| Decimal precision/scale parity | machine-enforced | shared Task Domain constants and boundary tests |
| Dependency and no-I/O direction | machine-enforced locally | Cargo tree, Clippy, source scan |
| Owner-fact authenticity/freshness | documented-only | constitutions/ADR; owner modules not yet built |
| Nonce/resource atomic consumption | documented-only | PostgreSQL owner contract deferred |
| Scope Check exact owner receipt | deferred, fail-closed | TASK-012 owner-module gate |
| Real PostgreSQL/Git owner integration | unverified outside TASK-011 | later MVP-1 tickets |
| Remote CI/merge policy | missing/unverified | no remote configured |

## Verification

- Rust workspace: 94 tests pass.
- Policy: 66 tests pass.
- Preserved Node: 38 tests pass.
- `cargo fmt --check`: pass.
- locked workspace Clippy with `-D warnings`: pass.
- `git diff --check`: pass.
- Policy forbidden-I/O scan: zero matches.
- Locked Policy dependency tree: approved pure edges only.

## Residual Non-Blocking Owner Work

Later modules must produce and authenticate Registry/Workspace-Git ref
identities, Scope Check receipts, Ledger resource claims, approval nonces,
Guardian receipts, freshness, and atomic PostgreSQL transitions. These are
explicit owner-module acceptance gates and do not weaken the current
fail-closed pure Policy boundary.
