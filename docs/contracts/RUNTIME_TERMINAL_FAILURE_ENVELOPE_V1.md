---
contract_id: LATTICE_RUNTIME_TERMINAL_FAILURE_ENVELOPE
version: 1.0.0
status: active
owner: latticed
---

# Runtime terminal failure envelope V1

## Purpose

Carry a verified known TASK-032 terminal failure through the `latticed` MCP
tool result and the `lattice-runtime` compatibility CLI without carrying any
payload, path, stdout, stderr, SQL, credential, or secret.

## Closed output

Every failure envelope has `status: "ERROR"` and a stable `code`. The
compatibility CLI retains its `message` field but fixes it to the same stable
code, rather than a child payload or variable diagnostic. Only a known `FAILED`
terminal receipt whose persisted stage and code exactly match the orchestrator
cause may additionally contain:

```json
{
  "status": "ERROR",
  "code": "LATTICE_DELIVERY_FAILED",
  "message": "LATTICE_DELIVERY_FAILED",
  "stage": "CODEX",
  "cause_code": "CODEX_APP_SERVER_TIMEOUT"
}
```

`stage` is one of the existing `DeliveryStage` values rendered in upper snake
case. `cause_code` is selected only from the composition-owned static
allowlist of current delivery-adapter leaves. It is not copied from arbitrary
port text.

## Fail-closed rules

- Unknown, malformed, or receipt-mismatched cause codes emit the separate
  `LATTICE_DELIVERY_TERMINAL_CAUSE_REJECTED` code and omit `stage` and
  `cause_code`.
- Ambiguous and reconciliation-required behavior remains unchanged and never
  becomes completed.
- Success receipt bytes, fields, and behavior are unchanged.
- MCP retains its two zero-argument tools and the compatibility CLI retains
  the same command surface.
