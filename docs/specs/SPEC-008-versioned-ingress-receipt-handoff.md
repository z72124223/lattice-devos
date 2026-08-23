# SPEC-008: Versioned ingress receipt handoff

## Outcome

Permit verified replay of a completed LATTICE task after a canonical
`latticed` binary upgrade without modifying historical evidence.

## Scope

Affected modules: Task Ledger 2.4, PostgreSQL Task Ledger persistence, and
the `latticed` task-control edge.  Public MCP schemas and output fields remain
unchanged.

## Required behavior

1. A completed stream created by ingress profile A remains immutable.
2. A successor profile B may be accepted only by one Ledger-owned handoff
   event binding A, the verified terminal receipt, B, and handoff verifier
   version `1.0`.
3. Replay under B verifies the original A evidence and the handoff; replay
   under any other profile fails closed.
4. A missing, duplicate, reordered, or substituted handoff fails closed.
5. The handoff has no effect authority and does not alter task result, state,
   command idempotency, or public MCP surface.

## Evidence

Focused unit tests cover each acceptance rule, including tampering and
historical no-handoff compatibility.  PostgreSQL replay and fresh-process MCP
status must prove the repaired production record before deployment.
