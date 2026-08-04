# TASK-009 Architecture Review

## Triggers

- New shared Rust contracts and public port traits.
- New dependency direction used by all future adapters.
- One Gateway, One Truth, and One Writer authority boundaries.

## Initial Blocker And Resolution

The initial independent review blocked integration because generic public
`Evidence` construction and generic port return types allowed an implementation
to cross-label any component/authority pair.

Resolution:

- generic evidence construction became crate-private;
- five public lane-specific evidence wrappers fix component/boundary pairs;
- all five traits return their own evidence type;
- SPEC-002, the amendment record, constitutions, and ticket now state this
  invariant;
- focused regression RED/GREEN and complete verification passed.

## Final Result

No architecture integration blocker. No ADR or constitution amendment is
required because the fix tightens the already-approved ADR-004/006 and
constitution intent before integration.

Confirmed:

- `GatewayService` is an inbound Rust-core service; OpenClaw is not a second
  control core or outbound provider.
- `CodexRunRequest` binds exact writer-claim evidence and only `CodexPort`
  returns product-code-writer evidence.
- Graphify and Hermes cannot return writer/control-store evidence through the
  typed public port API.
- `ControlStore` is an abstract boundary with no PostgreSQL driver, SQL,
  connection, or durability implementation.
- Dependency direction remains `lattice-ports -> lattice-contracts`, with no
  concrete adapter or cycle.

Residual non-blocking risks:

- `GatewayAction::Approve` must later be separated by domain/policy from
  guardian-only protected-release approval.
- `RuntimeKind::Live` is a classification only, never proof of capability,
  durability, lease authority, or real execution.
- Lane-specific error constructors may later improve `PortError` diagnostic
  labeling.
- PostgreSQL epoch, admission, idempotency, and transaction semantics remain
  future ADR-005 work.
