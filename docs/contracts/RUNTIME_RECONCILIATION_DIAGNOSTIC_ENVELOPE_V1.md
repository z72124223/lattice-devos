---
contract_id: LATTICE_RUNTIME_RECONCILIATION_DIAGNOSTIC_ENVELOPE
version: 1.0.0
status: active
owner: latticed
---

# Runtime reconciliation diagnostic envelope V1

## Purpose

When a bounded delivery result is uncertain, provide one closed, actionable
instruction without claiming recovery, revealing evidence contents, or widening
the MCP surface.

## Closed output

Only an error whose stable code is
`LATTICE_DELIVERY_RECONCILIATION_REQUIRED` may additionally contain:

```json
{
  "reconciliation_action": "INSPECT_ONLY",
  "retry": "FORBIDDEN"
}
```

Both values are static protocol values. They do not identify a filesystem path,
database row, receipt digest, process, workspace, command, or account.

## Fail-closed rules

- No error other than `LATTICE_DELIVERY_RECONCILIATION_REQUIRED` gains these
  fields.
- The fields never authorize a retry, mutation, task submission, approval, or
  release.
- Missing or malformed evidence remains a generic bounded error; it does not
  acquire a reconciliation diagnosis by inference.
- The two zero-argument MCP tools, success results, and terminal-cause envelope
  remain unchanged.
