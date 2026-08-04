# LATTICE DevOS Direction Change

## Authority

- Date: 2026-07-29 (Asia/Taipei)
- Source: direct user clarification in the active Codex task
- Status: current product direction

This record supersedes product-scope inferences drawn from the example project
inside `pasted-text.txt`. The source file remains historical evidence; it is no
longer the sole authority for the active product plan.

## Current Product

LATTICE DevOS is a **general-purpose, local-first autonomous AI development
platform** for the user's computer. It is not a feature of, deployment path for,
or continuation of any particular website.

Required platform components:

- OpenClaw as the normal human gateway;
- a Rust LATTICE control core;
- PostgreSQL as the durable control-plane truth;
- Codex as the exclusive product-code Implementer;
- Graphify as a read-only-source code-knowledge-graph adapter whose derived
  output goes to LATTICE-owned artifact storage;
- Hermes as a read-only-product-input research and reflection adapter whose
  output remains an untrusted candidate;
- LATTICE-owned Codebase Memory with provenance and review states;
- a controlled self-improvement and self-upgrade loop.

## Explicit Exclusions

- No project-specific website is part of the LATTICE product.
- No unrelated user repository is an implicit target or dependency.
- No installation, account login, payment, publication, deployment, or public
  network exposure is authorized by this direction change.
- No component may become a second durable truth or a second product-code
  writer.

## Technology Preference

- Rust is the preferred language for the control core and trusted local
  services.
- PostgreSQL is the preferred durable database.
- A small TypeScript/ESM boundary is permitted where the OpenClaw plugin SDK
  requires it.
- Python tools such as Graphify and Hermes remain isolated external adapters;
  they do not own LATTICE state.

## Governance Consequence

The existing Node.js Phase 1 implementation is preserved as a prototype and
characterization reference. Its active plan, specification, tickets, ADR-003,
and module constitutions require versioned V2 replacement or amendment before
Rust implementation begins. Existing uncommitted work must not be reset,
cleaned, deleted, or silently included in a V2 implementation.
