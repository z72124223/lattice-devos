---
protocol_id: LATTICE_ENGINEERING_PROTOCOL
version: 2.0.0
status: active
canonical_path: docs/contracts/ENGINEERING_PROTOCOL_V1.md
---

# LATTICE engineering protocol

## Safety Boundaries

Preserve One Gateway, One Truth, One Writer; unrelated work; credential
confidentiality; database ownership; process containment; and explicit human
authority for default-branch mutation, merge, deployment, release, public
exposure, account changes, irreversible actions, and destructive cleanup.

## Risk Tiers

- Routine work uses a final diff inspection and focused verification.
- Standard product work adds affected integration verification.
- High-risk authority, persistence, containment, live-service, default-branch,
  deployment, or release work adds explicit scope, negative tests, disposable
  resources where relevant, and independent review proportional to the risk.

Tests prove only what they execute. Static files do not prove live services.

Modules verify independently. Run pairwise integration only for an interface
used by the current product path. The retired all-component full chain is not a
completion gate; `cargo test --workspace` remains an explicit regression tool.

## Complexity Circuit Breaker

Product value and simplification take precedence over closing every governance
artifact. Dashboard, ticket, validator, finisher, handoff, or label defects do
not by themselves make product behavior fail. Do not create another task only
to repair governance for the current task.

One governance repair is the maximum for a product task. On a second governance
blocker, simplify or remove the gate, or record a known limitation. Do not build
bridge-task chains, retrofit historical branches to current metadata, or add a
proof mechanism larger than the behavior it protects. After two failed attempts
at the same acceptance, stop retrying and return to the shortest usable path.

## Delivery

Ordinary local completion does not require the finisher, a dashboard refresh, a
terminal ticket, a review document, or a root handoff update.

`npm.cmd run delivery:finish` is an optional high-risk boundary for an explicitly
authorized remote feature delivery. When invoked, its terminal ticket,
repository identity, non-force push, remote equality, and archive marker remain
strict. It never authorizes default-branch mutation, merge, deployment, release,
credential changes, force push, or destructive cleanup.

`npm.cmd run status:refresh` is optional diagnostic projection. Missing or stale
projection evidence stays visible but cannot create tasks, grant authority, or
override Git, tests, runtime receipts, or product behavior.
