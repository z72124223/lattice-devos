# LATTICE DevOS Repository Rules

These rules add to, and do not weaken, the global Codex workflow.

## Product Boundary

- Preserve **One Gateway. One Truth. One Writer.**
- Only the `IMPLEMENTER` role may write product code.
- The `INTEGRATOR` may mutate Git metadata only. A merge conflict that requires
  product-code edits must fail closed and return to a new Implementer task.
- Execution approval, merge approval, and production deployment approval are
  separate gates.
- Phase 1 must use `runtime_profile: "fake"`, `network_policy: "deny"`,
  `deployment_policy: "deny"`, `max_model_calls: 0`, and
  `max_external_cost: 0`.
- Do not access the playmate website repository from this project.
- Do not authenticate, buy, publish, deploy, push, or call a real model/API as
  part of Phase 1.

## Development Workflow

1. Read `PLANS.md`, the current specification, the active ticket, and every
   affected module constitution before editing.
2. Keep exactly one ticket current.
3. Use test-first RED/GREEN evidence for each behavior.
4. Do not edit outside the active ticket's `allowed_paths`.
5. Treat the Task Ledger as the only durable control-plane truth.
6. Run focused tests after each behavior and `npm run verify` before review.
7. Record code review, architecture review, and integration readiness before
   changing the primary branch.
8. Never merge the primary branch without explicit user approval.

## Evidence Labels

- Passing local tests prove only the tested local behavior.
- Static plugin checks do not prove OpenClaw can load the plugin.
- Git Scope Check is a detection gate, not an operating-system sandbox.
- A local CI file is documented automation until a remote service actually
  runs it and branch policy requires it.

