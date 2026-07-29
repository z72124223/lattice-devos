# LATTICE DevOS Product Charter

## Identity

- Formal name: **LATTICE DevOS**
- Chinese name: **織網 AI 開發中樞**
- Expansion: **Layered Agent Task, Tool, Intelligence, Context & Execution**
- Version: **v1.0 — Controlled Swarm**
- Repository: `lattice-devos`
- OpenClaw primary agent: `lattice-pm`
- User command: `/lattice`

## Principle

> **One Gateway. One Truth. One Writer.**
>
> 一個入口、一个事實來源、同時間一個程式碼寫入者。

## Phase 1 Outcome

Phase 1 establishes an offline control core that can prove the intended task
flow with a deterministic Fake Runtime:

1. accept and freeze a Task Spec;
2. wait for execution approval bound to that spec;
3. acquire one repository/project writer lease;
4. prepare an isolated Git worktree;
5. invoke one Implementer;
6. stop the writer before verification;
7. verify build/test/scope evidence;
8. collect read-only reviews;
9. wait for a separate merge approval;
10. let the Integrator mutate Git metadata without editing product code;
11. record every decision in one append-only Task Ledger.

## Roles

| Role | Purpose | Product-code write |
|---|---|---:|
| `LATTICE_PM` | Gateway task control, status, approvals, stop | no |
| `PLANNER` | Plan and decompose | no |
| `CODE_MAPPER` | Confirm code structure and dependencies | no |
| `GRAPHIFY` | Optional later knowledge graph lane | no |
| `IMPLEMENTER` | Execute the approved code change | yes, exclusive |
| `CORRECTNESS_REVIEWER` | Read-only correctness review | no |
| `SECURITY_REVIEWER` | Read-only security review | no |
| `ARCHITECTURE_REVIEWER` | Read-only architecture/test review | no |
| `INTEGRATOR` | Branch/worktree/PR/merge metadata | no product-code edits |

At most four worker-agent invocations may be active concurrently. The
deterministic Orchestrator is control-plane code and is not a model agent.

## Deferred Lanes

- Graphify is not part of the Phase 1 execution path.
- Hermes remains an optional later research/documentation lane.
- Hostinger, Codex OAuth, Telegram, private Git access, and real process
  containment are capability-preflight work after the local MVP.

## Human-Owned Gates

- Service purchase and payment.
- OAuth, account, credential, and Telegram token handling.
- High-risk approvals.
- Merge to the primary branch.
- Production deployment and acceptance.

