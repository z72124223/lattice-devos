# TASK-017 Workflow Audit

- Date: 2026-08-01
- Repository:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Mode: read-only audit before TASK-017 application-code modification

## Confirmed Slice And Continuity

TASK-016 is complete and remains aligned with MVP-1. Its 32 Contracts, 97
Artifact Store, 322 full Rust workspace, and 38 preserved Node tests passed;
independent code/security/architecture review reported zero remaining P0-P3
findings. Artifact Store authority is not changed by this ticket.

TASK-017 freezes the next planned MVP-1 dependency: a pure/fake OpenClaw-facing
gateway protocol. It will not install or invoke OpenClaw, choose a live Windows
transport, authenticate an OS peer, start a listener/daemon, mutate PostgreSQL,
touch Git/product files, call a provider, use credentials, or perform a
protected action.

The configured project router entry point remains absent and returned
`MODULE_NOT_FOUND`; `PLANS.md`, `HANDOFF.md`, SPEC-002, and repository-local
governance provide the direct project match. No relevant LATTICE memory entry
was found.

## Repository And Enforcement Evidence

- Feature HEAD remains four commits ahead and zero behind local `main`.
- No remote/upstream is configured; CI definition exists, but remote execution,
  branch protection, and required reviews are unverified.
- The shared V2 worktree is intentionally dirty and uncommitted. No reset,
  clean, branch switch, commit, push, merge, install, deployment, or external
  action occurred.
- The repository has SPEC, ADR, ticket, constitution, project-check, Rust
  format/Clippy/test, Node characterization, independent review, integration,
  workflow-ledger, and handoff conventions.
- Merge readiness remains blocked by dirty/uncommitted state, absent remote
  enforcement, absent synchronization evidence, and absent primary-merge
  authorization.

## Capability Classification Before TASK-017

| Capability | Status | Evidence | Enforcement |
|---|---|---|---|
| Repository/rules/plan/handoff | valid | TASK-016 closure plus current Git state | documented plus machine-observed |
| OpenClaw adapter governance | stale | active 1.0 is inert; SPEC proposes 2.0 | documented-only |
| Gateway action names | partial | six closed names exist | machine-compiled |
| Action-specific payloads | missing | only `Invocation + GatewayAction` | missing |
| Server-derived peer context | missing | no peer/session type | missing |
| Typed terminal reply | missing | only `GatewayEvidence(output_digest)` | missing |
| Exact retry/substitution | missing | no gateway receipt owner/fake | missing |
| Canonical frame/limits | missing | cjson does not parse arbitrary JSON | missing |
| Approval/protected separation | partial | Approval contracts reject invalid trust pairs | gateway routing missing |
| Stop semantics | missing | bare `Stop` action only | missing |
| Live OpenClaw/OS transport/auth | deferred | MVP-2 exact-version gate | missing by design |
| Remote CI/branch protection | missing/unverified | no remote | missing |

## Blocking Findings And Resolution

Three P1 findings blocked code before governance:

1. the nominal command cannot bind project, actor/session, Task Spec, status,
   approval, stop, or idempotency subjects;
2. generic gateway evidence cannot distinguish routing, denial, unknown
   outcome, or stop request and incorrectly labels the core reply as OpenClaw
   evidence;
3. active OpenClaw/Orchestrator/Contracts/Ports constitutions did not match the
   V2 boundary.

ADR-015, SPEC-002 v14, Gateway IPC 1.1, OpenClaw Adapter 2.0,
Orchestrator Runtime 2.0, Contracts 1.7, Ports 1.2, and TASK-017 resolve the
governance blockers before RED tests. The user's approved V2 direction and
later instruction to execute bounded local work through MVP-3 authorize these
reversible amendments; protected actions remain fail-closed.

## Required Execution Order

1. Validate all TASK-017 governance and exactly one current plan marker.
2. Add failing shared-contract and port-signature tests.
3. Add failing codec/limit/digest/binding/retry/role/fault tests.
4. Implement only the minimum pure Rust contracts and fake loopback needed to
   turn each RED case GREEN.
5. Run focused/full tests, strict Clippy/format, dependency/no-I/O/redaction/
   forbidden-action/governance scans, and diff checks.
6. Complete independent code/security and architecture reviews; every accepted
   finding receives a failing regression before repair.
7. Write integration evidence, ledger, ticket closure, PLANS, and HANDOFF.

No unresolved responsible-user decision remains for this bounded pure/fake
slice. Physical transport, OS authentication, live OpenClaw compatibility,
credentials, protected release, deployment, and primary-branch merge remain
outside TASK-017.
