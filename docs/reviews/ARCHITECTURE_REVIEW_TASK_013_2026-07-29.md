# TASK-013 Architecture Review

## Triggers

- New Task Ledger semantic owner module and public event/receipt/replay API.
- Material Contracts 1.3 and Policy 2.4 public-contract amendments.
- Cross-module Task ID, resource ownership, and current-head consistency.
- Persistence-boundary, dependency-cycle, and fake-versus-live authority risk.
- One Gateway, One Truth, One Writer, project isolation, and MVP-3 safety.

## Independent Result

`PASS`. No remaining P0, P1, P2, or P3 architecture finding.

Confirmed:

- Task Ledger 2.0 is the sole semantic owner of task stream identity, events,
  hash chain, terminal command receipts, verified replay, and resource
  projection.
- Task Domain remains the sole owner of legal task-state transitions. Future
  Orchestrator composition consumes both contracts; neither module depends on
  the other.
- Contracts 1.3 owns immutable shared representations only. Policy 2.4 owns
  deterministic sufficiency/deny meaning only.
- Task Ledger's normal dependency graph is exactly Contracts, cjson mechanics,
  and exact `time` parsing/formatting.
- Policy's normal graph remains Contracts plus Task Domain. Its Ledger edge is
  an explicit one-way `dev-dependency` used only by cross-crate owner
  composition tests; it is absent from the production graph and creates no
  cycle.
- Public raw snapshots preserve complete appended and denied command records.
  The fake exports and verifies through the same replay boundary, including a
  zero-event stream that contains only terminal denials.
- Task Ledger performs no filesystem, Git, database, process, network,
  environment, clock-read, randomness, provider, credential, payment,
  publication, deployment, or product-repository I/O.
- No second gateway, durable truth, product-code writer, or protected-release
  authority was introduced.

## Enforcement Truth

| Gate | Classification | Evidence |
|---|---|---|
| Pure deterministic Ledger API | machine-enforced locally | Rust types and 20 Ledger tests |
| Append/retry/denial/replay | machine-enforced locally | focused event, command, tamper, and snapshot matrices |
| Diagnostic bounds and redaction | machine-enforced locally | raw/sanitized/Debug/error regressions |
| Resource projection/currentness | machine-enforced locally | Ledger owner lookup and Policy composition matrices |
| Task ID cross-module rule | machine-enforced locally | shared valid/invalid suffix fixtures |
| Normal dependency/no-I/O direction | machine-enforced locally | Cargo trees, Clippy, and source scan |
| Policy test-only owner composition | machine-enforced locally | all-edge Cargo tree plus cross-crate test |
| Governance semantics | documented plus structural review | ADR-011, constitutions, SPEC, ticket, validator |
| Authenticated/durable current head | documented-only and deferred | future Orchestrator/PostgreSQL gate |
| PostgreSQL atomicity/restart/concurrency | deferred and fail-closed | future store ticket |
| Remote CI/merge policy | missing/unverified | no remote configured |

## Verification

- Contracts: 13 tests pass.
- Task Ledger: 20 tests pass.
- Policy: 75 tests pass.
- Focused three-crate combined result: 108 tests pass.
- `cargo fmt --all -- --check`: pass.
- locked all-target/all-feature Clippy with `-D warnings`: pass.
- selected constitution validation: three valid, zero warning/error.
- normal/all Cargo edge checks: pass.
- forbidden Task Ledger I/O scan: zero matches.
- `git diff --check`: pass.

## Residual Non-Blocking Owner Work

- The PostgreSQL ticket must add a Ledger-owned pure live append-plan boundary
  before a store adapter is allowed to persist events; it must not replicate
  private hash/event rules.
- Durable resource-observation revision/latest-receipt rows, authenticated
  current-head lookup, transaction serialization, unknown-commit retry,
  restart, and power-loss behavior remain future PostgreSQL work.
- A cryptographically self-consistent but unauthorized history rewrite is
  outside what local SHA replay alone can detect; database identity and access
  control must supply the trust boundary.
