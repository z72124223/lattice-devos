# ADR-024 — Connectivity First, Microkernel Core, And MCP Providers

- Status: Accepted by responsible user
- Effective date: 2026-08-11
- Decision source: Issues #4, #5, #9, #10 and the user-approved architecture amendment
- Frozen implementation baseline: `a86747ddfa182590c49324a1add15ce4bdc70978`

## Context

LATTICE has grown into a large Rust workspace with many governance, domain,
persistence, provider, and acceptance layers. Source-code crates already exist,
but a single capability change can still cross Domain, Ports, runtime
composition, PostgreSQL, tests, scripts, ADRs, and module constitutions. This is
high change amplification rather than effective modularity.

At the same time, ChatGPT is intended to be a high-level human gateway, but the
most important real boundary remains unproven: a current ChatGPT session must
invoke LATTICE, receive a typed response, submit only a bounded governed intent,
and later read the same PostgreSQL-backed result from a separate session.
Existing local MCP tests, tunnel readiness, and tool discovery are valuable but
do not prove that complete path.

The project also needs trustworthy remote handoffs, explicit resolution of
Issue overlap, and a default preference for the smallest architecture that
reaches an equally strong verified outcome.

## Decision 1 — LATTICE Uses A Governed Microkernel

The stable kernel owns authority that must remain strongly consistent and
independent of replaceable providers:

- Task Domain and legal state transitions
- Policy and capability admission
- PostgreSQL authoritative Task Ledger
- Writer Lease and fencing
- canonical encoding and evidence validation
- deterministic orchestration order
- Guardian, release, and rollback boundaries

These responsibilities remain in-process unless a later ADR proves that a
process boundary preserves equivalent atomicity, evidence strength, failure
closure, and total simplicity.

## Decision 2 — External Capabilities Are Replaceable Providers

Codex, Hermes, Graphify, model providers, GitHub, Linear, technical radar, and
future project-specific tools are capabilities, not permanent platform
identity.

The integration order is:

```text
provider-neutral typed Port
        -> reviewed adapter
        -> MCP / HTTP / local RPC / in-process implementation
```

MCP is preferred when it creates a clean replaceable provider boundary or
useful process isolation. MCP is not mandatory for every module and must never
become a second kernel, task truth, queue owner, scheduler, policy engine, or
writer.

A provider implementation:

- cannot grant itself authority;
- cannot mutate another provider directly;
- cannot expose arbitrary SQL, shell, path, credentials, lease, fence, or
  product-writer controls;
- must be independently health-checkable and replaceable;
- must return typed evidence through the owning Port.

## Decision 3 — ChatGPT <-> LATTICE Is Product P0

No further broad runtime expansion is justified before the real communication
boundary is proven or a fixed platform blocker is recorded.

Acceptance is segmented and cannot be collapsed into one claim:

1. `DISCOVERY_VERIFIED`
   - a real ChatGPT session sees the exact approved tools and closed schemas.
2. `READ_ROUNDTRIP_VERIFIED`
   - a real ChatGPT request invokes a read-only LATTICE tool and receives a
     typed LATTICE response.
3. `BOUNDED_SUBMIT_VERIFIED`
   - a real ChatGPT request invokes only the fixed approved canary intent;
     arbitrary task text, shell, SQL, paths, providers, credentials, lease,
     fencing, and writer inputs remain impossible.
4. `CROSS_SESSION_DURABLE_READ_VERIFIED`
   - a separate new ChatGPT request/session reads the same PostgreSQL-backed
     result with zero repeated Codex, verification, Git, Graphify, Hermes, or
     Memory effects.
5. `CONTROL_VERIFIED`
   - pause, resume, cancel, approve, and reject are later versioned policy
     contracts, not implied by Submit/Status.

Tool discovery is not invocation. Local invocation is not ChatGPT invocation.
A same-process or same-session read is not durable cross-session replay.

If the ChatGPT workspace, plan, connector snapshot, or platform capability
prevents a gate, LATTICE records `PLATFORM_CAPABILITY_BLOCKED` with current
versioned evidence. It does not add unsafe bypasses or mislabel the gate as
passed.

## Decision 4 — Every Push Requires A Trustworthy Checkpoint

Every remote-changing push must be followed by a Checkpoint Receipt v1 bound to
its exact remote SHA. The receipt may be committed with the logical checkpoint
or immediately posted to the canonical Issue.

Required evidence includes:

- canonical Issue / task relationship;
- branch, base, parent, commit, and tree SHA;
- development start/end time, timezone, and monotonic elapsed time when
  available;
- exact changed paths;
- checks, exit results, evidence locations/digests, and evidence level;
- explicit `NOT_TESTED` items and blockers;
- remote equality, ahead/behind, worktree status;
- PR, merge, deployment, release, CI, and branch-protection state.

A commit timestamp does not prove development duration. A report without an
exact remote SHA is `DECLARED`, not trustworthy current state. Later agents and
ChatGPT must reject stale receipts that no longer match the referenced branch.

The operational format is defined by
`docs/governance/DEVELOPMENT_GOVERNANCE_V1.md`.

## Decision 5 — Issue Authority Must Be Canonicalized

Before creating or expanding an Issue, search existing Issues by capability,
module, provider, and failure code. Each overlap declares one relationship:

- `CANONICAL`
- `DUPLICATE_OF`
- `OVERLAPS_WITH`
- `SUPERSEDES`
- `SUPERSEDED_BY`
- `SPLIT_FROM`
- `MERGED_INTO`
- `HISTORICAL`

A newer Issue may supersede exact sections while leaving the rest of the older
Issue effective. The relationship records the effective date, affected
sections, and remaining authority. Conflicts require a reconciliation note and
responsible-user decision; an agent cannot silently choose one version.

For the current set:

- Issue #4 remains canonical for ChatGPT MCP connectivity and bounded gateway
  behavior.
- Issue #5 remains canonical for capability-oriented provider replacement.
- Issue #9 supersedes any requirement that ordinary core Task success must
  synchronously wait for Hermes.
- Issue #10 remains canonical for segmentation, retry budgets, circuit
  breaking, and architecture-review pauses.

## Decision 6 — Simplicity Means Time To Verified Outcome

Before adding code, compare reuse, narrower scope, mature providers, deletion
or deferral, and shorter acceptance paths.

When candidates provide equivalent safety, correctness, durability, rollback,
and evidence, choose the one with fewer:

- concepts and mutable states;
- processes and protocols;
- crates and adapters;
- queues and schemas;
- truth sources and writers;
- formal acceptance stages.

Adding a runtime process, MCP server, crate, schema, queue, authentication
system, long-running worker, truth source, or writer requires a written
complexity justification. A shortcut is valid only when it preserves all
invariants and produces equally strong evidence. `Time to first code` is not an
optimization target; `time to verified outcome` is.

## Consequences

- The frozen GH-9 code remains intact; this ADR does not authorize GH-9 live
  PostgreSQL replay or TASK-037 formal.
- Task Domain 2.3 and Ports 1.9 document the already-checkpointed independent
  Reflection vocabulary and bounded Port surface.
- `latticed` 1.5 records ChatGPT round-trip as P0 while preserving the four
  existing closed tools and One Gateway composition.
- Provider extraction will occur incrementally, only when a typed Port and a
  measurable reduction in coupling or risk exist.
- Existing local MCP and PostgreSQL evidence remains valid only for the exact
  gates it actually exercised.
- No PR, merge, deployment, release, or default-branch change is authorized by
  this ADR.

## Rejected Alternatives

### Convert every LATTICE module into an MCP server now

Rejected because it would multiply process supervision, authentication,
version negotiation, timeout, retry, cleanup, and distributed consistency
burdens while moving core authority across fallible boundaries.

### Keep adding crates without provider boundaries

Rejected because source-code subdivision alone has not reduced change
amplification.

### Continue GH-9 / TASK-037 before proving ChatGPT connectivity

Rejected because the primary human-control boundary is more valuable and less
expensive to validate first.

### Treat agent prose as current project truth

Rejected because later GPT sessions cannot distinguish stale or overstated
reports without exact remote and evidence bindings.