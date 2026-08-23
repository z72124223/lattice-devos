---
spec_id: SPEC-007
status: ready
version: 1
modules:
  - module_id: orchestrator-runtime
    constitution_version: 2.7
---

# Evidence-Bound Autonomy Cycle Supervisor

## Objective

Define a closed, evidence-bound decision cycle for LATTICE self-improvement.
The cycle may recommend only a small reversible engineering change already
within the current task authority. It never creates authority, directly starts
a process, changes files, invokes a model, or exposes a new MCP operation.

## Input trust and authority

The user's stated goal and explicit authorization remain authoritative. Claims
from any source, including the user, model output, cached status, dashboards,
or worker reports, are untrusted technical evidence until independently
verified by the existing LATTICE evidence path. Missing, stale, conflicting,
or malformed evidence blocks selection rather than being assumed true.

## Required round behavior

1. Consider only candidates marked reversible, locally authorized, and paired
   with a current verification plan.
2. Evaluate the selected candidate plus four explicit alternatives, producing
   exactly five bounded safety/value challenges.
3. Select a candidate only when it dominates the alternatives on evidence,
   reversibility, verification, and authority. Otherwise return a typed
   blocker.
4. Select `gpt-5.6-luna` at low reasoning first. A candidate may recommend a
   higher reasoning effort only after a recorded bounded insufficiency. It may
   recommend `gpt-5.6-terra`, then `gpt-5.6-sol`, only after the preceding
   model-level insufficiency. No recommendation is model invocation.
5. If a new foreground Codex task is observed, return `YieldToForeground`.
   The autonomous cycle retains no execution claim and may resume only from a
   new evidence snapshot after foreground completion.
6. Every result is data-only and includes a typed completion or blocker reason
   suitable for the existing durable Task Ledger owner to record.

## Non-goals

- No free-form user task ingestion, scheduler, daemon, process/window control,
  model call, filesystem write, network access, PostgreSQL write, or MCP change.
- No push, merge, deployment, release, account/credential action, security
  downgrade, or irreversible deletion.
- No replacement of Task Ledger, Policy, Writer Lease, or user authority.

## Acceptance criteria

- Missing verification evidence, an unauthorized candidate, protected action,
  or fewer/more than five challenges fails closed.
- A valid round recommends Luna/low first and exposes no execution authority.
- Escalation requires ordered recorded insufficiency: Luna -> Terra -> Sol;
  skipping, regressing, or escalating without evidence fails closed.
- A foreground-task observation always yields before candidate selection.
- The module remains pure and uses no I/O or concrete-adapter dependency.
