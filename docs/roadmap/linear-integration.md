# Linear Integration Direction

Status: **Approved future direction**  
Recorded: **2026-08-09**

## Decision

LATTICE DevOS will treat Linear as an external human-facing project/task control surface rather than rebuilding a full issue/project-management UI inside LATTICE.

The intended responsibility split is:

### Linear — Work / UI layer
- Project and Issue management
- Boards and human-facing task status
- Agent assignment/delegation entry point
- Agent activity/status presentation
- Human collaboration around work

### LATTICE DevOS — AI control and execution governance
- Agent orchestration
- Planner / scheduling policy
- PostgreSQL durable control-plane truth
- Codebase Memory
- Graphify
- Hermes reflection/research boundary
- Verification
- One Gateway / One Truth / One Writer enforcement
- Capability, scope, evidence, and execution governance

### GitHub — Source and delivery layer
- Repository
- Branch / worktree
- Commit
- Pull Request
- CI and code review artifacts

## Target topology

```text
Human / ChatGPT
      |
      +-------------------+
      |                   |
      v                   v
 LATTICE Gateway        Linear
      |             Issues / Projects / UI
      |                   |
      |             MCP / API / Webhook
      |                   |
      +---------+---------+
                v
        LATTICE DevOS Core
        - Planner
        - Scheduler
        - PostgreSQL / Memory
        - Graphify
        - Hermes
        - Verification
        - One Writer
                |
        Agent Orchestration
          /      |       \
       Codex   Hermes   Local/Other Agents
          |
          v
        GitHub
     Code / PR / CI
          |
          +---- status/evidence ----> LATTICE ----> Linear
```

## Integration plan

### Phase 1 — Validate the existing ecosystem before building an adapter

1. Create/configure a Linear workspace for LATTICE.
2. Connect Linear to GitHub.
3. Connect Codex to Linear through Linear's MCP surface where appropriate.
4. Create one real LATTICE Issue in Linear.
5. Verify that Codex can read the Issue, execute a bounded repository task, and return useful status/result information.
6. Evaluate whether this workflow materially reduces custom UI/task-management work.

This phase should avoid changing the LATTICE core unless required for a bounded compatibility fix.

### Phase 2 — LATTICE Linear adapter

After Phase 1 is proven useful, add a bounded Linear integration adapter (working name: `lattice-linear`; final crate/module placement follows the repository architecture at implementation time).

Expected responsibilities:
- Linear API client boundary
- Issue/project identity mapping
- Webhook/event ingestion
- LATTICE task/execution mapping
- Status synchronization
- PR/result/evidence linking
- Idempotency and replay protection

Linear must not become an independent durable truth for LATTICE execution state. PostgreSQL remains authoritative for the LATTICE control plane.

### Phase 3 — LATTICE as a Linear Agent

Evaluate exposing LATTICE as an assignable/delegatable Linear Agent so a human can hand an Issue to LATTICE from Linear.

Conceptual flow:

```text
Linear Issue assigned/delegated to LATTICE
                |
                v
        LATTICE receives event
                |
                v
       Planner / policy decision
                |
                v
         Agent orchestration
          /       |       \
       Codex    Hermes   Reviewer/etc.
                |
                v
           Verification
                |
                v
            One Writer
                |
                v
              GitHub
                |
                v
     PR / result / evidence
                |
                v
        Linear status update
```

## Architectural constraints

The integration must preserve existing LATTICE invariants:

> **One Gateway. One Truth. One Writer.**

In particular:
- Linear is not allowed to become a second execution truth store.
- Linear must not bypass LATTICE scope/capability/verification policy for LATTICE-managed work.
- Linear/Codex integration must not create a second uncontrolled product-code writer for the same LATTICE execution.
- External webhook/API payloads are untrusted inputs and require validation, authentication, idempotency, and explicit mapping into typed LATTICE contracts.
- GitHub remains the source/delivery system; Linear references work, while LATTICE governs execution.
- Existing local-first operation must not require Linear. Linear is an optional external control surface/integration.

## Product implication

Do not spend LATTICE engineering effort rebuilding commodity project-management UI when Linear can satisfy that layer. Concentrate LATTICE development on the differentiating control plane: governed AI execution, durable memory/evidence, orchestration, verification, and safe multi-agent coordination.

This document records strategic direction, not completion of the integration. Phase 1 validation is the next gate before implementing the adapter.