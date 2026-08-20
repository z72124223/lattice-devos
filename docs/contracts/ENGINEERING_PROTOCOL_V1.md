---
protocol_id: LATTICE_ENGINEERING_PROTOCOL
version: 2.0.0
status: active
canonical_path: docs/contracts/ENGINEERING_PROTOCOL_V1.md
---

# LATTICE engineering protocol

## Entry

Inspect the current branch, HEAD, worktree status, relevant code, and this
protocol. Preserve unrelated work and classify the actual risk before editing.

## Product priority

Prefer a small usable product over closing every historical governance item.
Use official Codex platform capabilities instead of recreating its agent loop.
Keep optional modules independently testable and independently fallible.

## Complexity circuit breaker

Dashboard, ticket, validator, finisher, label, historical branch, plan, or
handoff defects do not by themselves make product behavior fail. Do not create
another task only to repair governance. Do not require all optional modules in
one acceptance. Do not add proof machinery larger than the behavior protected.

After two failed attempts at the same acceptance, preserve current evidence,
stop retrying, and return to the shortest usable product path.

## Verification

- Routine work: inspect the final diff and run focused checks.
- Standard behavior: add affected integration checks.
- High-risk authority, persistence, containment, live-service, default-branch,
  deployment, or release work: add explicit scope, negative tests, disposable
  resources where relevant, and independent review proportional to risk.

Tests prove only what they execute. Static files do not prove live services.

## Delivery and authority

Ordinary local completion does not require a ticket, finisher, dashboard
refresh, root plan update, root handoff update, or separate review document.

`npm.cmd run delivery:finish` is an optional boundary for an explicitly
authorized non-force feature delivery. It never grants permission for a force
push, default-branch mutation, merge, deployment, release, public exposure,
credential change, destructive cleanup, or irreversible action.

Preserve unrelated work, credential confidentiality, local-only network
defaults, and explicit user authority for consequential external actions.
