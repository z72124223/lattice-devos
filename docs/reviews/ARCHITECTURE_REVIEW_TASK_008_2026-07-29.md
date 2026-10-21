# TASK-008 Architecture Review

## Triggers

- New Rust workspace and modules.
- New public bootstrap manifest.
- New PostgreSQL schema draft.
- New CLI dependency direction.

## Initial Finding And Resolution

The initial review blocked integration because `lattice-core-bootstrap` and
`lattice-cli` were active constitutions but were absent from SPEC-002 and the
module amendment record.

Resolution:

- SPEC-002 became version 2 and now lists both modules in frontmatter and
  Module Impact.
- The approved module record defines both missions, non-goals, contracts,
  gates, and direct user approval.
- The module routing document and TASK-008 reference the same versions.
- Module parity verification reported 20 specification modules and 20 proposal
  headings with no missing or extra IDs.

## Final Result

No architecture integration blocker. No ADR or constitution amendment is
currently required.

Confirmed:

- OpenClaw remains the only planned normal gateway.
- PostgreSQL remains the only durable control truth.
- Codex remains the only planned product-code writer.
- Graphify and Hermes remain read-only with respect to product source/code.
- The CLI has no runtime authority and must use approved local IPC before any
  future operational command is added.
- The SQL draft contains only three namespace declarations.
- Cargo dependencies are acyclic and contain no third-party package.
- Both packages have `publish = false`.

Residual non-blocking risks:

- `DurableMemory` must never be implemented as a second durable authority
  outside PostgreSQL.
- A real database migration requires role, ownership, `search_path`, rollback,
  and least-privilege evidence.
