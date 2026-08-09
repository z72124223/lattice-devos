# TASK-028～TASK-031 Hardening Completion Crosswalk

- Audit date: 2026-08-10 (Asia/Taipei)
- Audit branch: `feature/task-028-031-hardening-audit`
- Exact base: `845328dcc06d51c7554c93a09739a27ddd827941`
- Scope: Git/history/spec/ADR/module-ownership reconstruction only
- Result: `READY` crosswalk; `BLOCKED` for any new implementation in this window
- Production acceptance: not performed and not claimed

## Executive Decision

TASK-028 through TASK-031 never acquired four legal, one-to-one ticket
definitions. At the last pre-delivery-first checkpoint (`5fd0142`), PLANS
assigned five outcomes to the four numbers as one aggregate sequence:

1. independent Review Runtime authority and fake;
2. fake Codex protocol adapter;
3. one offline end-to-end flow;
4. retained V1 compatibility; and
5. the MVP-1 exit ledger.

No `docs/tickets/TASK-028*` through `TASK-031*`, matching branch, matching
commit subject, or task-specific review artifact exists in the reachable Git
history. Therefore the number-to-outcome mapping below is historical inference,
not authority to invent four tickets retroactively.

The delivery-first change at `c7278e5` deliberately preserved these items as
hardening backlog and prohibited the old fake-adapter sequence from overriding
TASK-032. Current PLANS and SPEC-002 retain that rule. The legal conclusion is:

- do not resurrect a parallel fake/runtime/provider path;
- do not mark 028～031 complete as four tickets;
- keep only the still-missing independent-review authority as a future
  capability decision; and
- treat the remaining outcomes as absorbed, superseded, or split across the
  delivery-first tickets and acceptance gates identified below.

## Live Repository And Worktree Evidence

The directory above the worktrees is not a Git repository. The common Git
repository reported these relevant worktrees before this audit worktree was
created:

| Worktree | Branch | HEAD | Live state |
|---|---|---|---|
| `chatgpt-mcp` | `feature/task-038-chatgpt-mcp` | `512732d` | dirty, 52 status entries |
| `full-chain-integration` | `feature/task-037-full-chain-integration` | `845328d` | clean, one commit ahead of fetched remote |
| `task-026-027-workspace-scope` | `feature/task-026-027-workspace-scope` | `fd9561c` | dirty, 10 status entries |
| `task-041-rust-ci` | `feature/task-041-rust-ci` | `e08f994` | clean |
| `task-042-hermes-strict-clippy` | `feature/task-042-hermes-strict-clippy` | `845328d` | dirty, 3 status entries |
| `task-043-memory-benchmark` | `feature/task-043-codebase-memory-benchmark` | `845328d` | dirty, 1 status entry |

`git fetch --all --prune` completed without changing the fetched comparison:
the clean TASK-037 worktree remained `1` ahead / `0` behind
`origin/feature/task-037-full-chain-integration`.

The active TASK-038 tree currently overlaps all plausible positive Review
Runtime composition surfaces: root `Cargo.toml`/`Cargo.lock`, Contracts, Ports,
Task Domain, Orchestrator, `latticed` composition, Codex adapter, Writer Lease,
Postgres Store, and their constitutions. This audit did not copy, reset, clean,
stash, or edit that tree or any other pre-existing worktree.

## Source Reconstruction

### Last pre-delivery-first checkpoint

At `5fd01426339ea3186bbc76fdaea1454d4dce30a0`:

- `PLANS.md:137-143` described 028～031 only as the five-outcome aggregate
  sequence above.
- `PLANS.md:637-640` repeated the aggregate and said live Codex remained MVP-2.
- `HANDOFF.md:673-674` and `HANDOFF.md:902-903` retained the same unresolved
  collection without assigning individual tickets.
- `docs/modules/V2_AMENDMENT_PROPOSAL.md:893-920` proposed Codex Adapter 1.0.
- `docs/modules/V2_AMENDMENT_PROPOSAL.md:922-945` proposed Review Runtime 1.0.
- `docs/adr/ADR-006-single-codex-owner-and-read-only-agents.md:94-104`
  required a dedicated read-only review port, a separate reviewer
  process/thread/profile, a frozen reviewer/evidence/acceptance subject, and no
  implementer mutation capability.

The ordering makes the following labels a reasonable historical inference:
028 = Review Runtime; 029 = fake Codex; 030 = offline end-to-end; 031 = V1
compatibility plus exit ledger. Because five outcomes were packed into four
numbers and no tickets froze the split, these labels are not legal ticket
definitions.

### Delivery-first replan and current governance

Commit `c7278e52ab311065ae1263cf3a40adb92aad447f` introduced TASK-032 and changed
the product direction before any 028～031 ticket existed. At base `845328d`:

- `PLANS.md:321-329` says TASK-032 begins the runnable real chain and preserves
  unfinished 022～031 only as hardening backlog.
- `PLANS.md:840-845` says 028～031 controls may be pulled forward only when a
  missing control blocks the executable path and must not override TASK-032.
- `docs/specs/SPEC-002-autonomous-development-platform.md:724-735` defines
  MVP-1 as direct delivery evidence, explicitly excludes fake/scripted claims,
  and preserves 022～031 as backlog.
- The ticket directory at this base contains TASK-001～022, TASK-032, and
  TASK-033 only.

## Completion Crosswalk

| Historical number | Inferred old outcome | Intended semantic owner and dependencies | Delivery-first evidence | Classification |
|---|---|---|---|---|
| TASK-028 | Independent Review Runtime authority and deterministic fake | A separate read-only review semantic owner; frozen change/evidence/acceptance subject; fixed reviewer set; normalized findings/recommendation/evidence; no writer lease, product mutation, approval, memory promotion, or release authority. Positive composition would require versioned Contracts/Ports plus Policy and Orchestrator consumption. | TASK-015 already landed the safe negative boundary: caller review Booleans are absent and R3/independent-review paths deny `REVIEW_AUTHORITY_UNAVAILABLE` (`crates/lattice-policy/src/checks.rs:245-345`; exhaustive matrix in `crates/lattice-policy/tests/policy_matrix.rs`). No `review-runtime` crate, active constitution, receipt/current-head owner, or positive composition exists. Hermes reflection in TASK-037 is untrusted memory/reflection evidence and cannot substitute. | **split: fail-closed control already landed; positive owner still missing and BLOCKED** |
| TASK-029 | Fake Codex protocol adapter | Historically Codex Adapter with a fake protocol server; now the sole production Codex adapter owns exact binary/schema/session/process evidence, while fakes remain test-only injected fixtures. | TASK-032 ticket requires a real adapter and a scripted app-server matrix. Commits `0cbe8c2` through `61cca93` created the protocol, identity/session/process supervision, pure injected `FakeCodex` orchestration tests, and bounded scripted acceptance. TASK-038 further binds the production writer request to Task Spec/lease/fence. A second fake adapter path would violate One Writer and the replan. | **superseded and absorbed by TASK-032; further hardened by TASK-038** |
| TASK-030 | One offline end-to-end flow | Orchestrator owns effect order; `latticed` is the sole composition root; PostgreSQL is truth; concrete adapters do not call each other. | TASK-032 provides the scripted Codex/PostgreSQL/workspace/test/Git/restart path; TASK-033 adds Graphify and PostgreSQL Codebase Memory; TASK-037 attaches the production Hermes/OpenClaw chain and remains acceptance-open; TASK-038 is adding governed Submit/Status and real writer fencing. Recreating an old fake-only offline orchestrator would be a competing path and weaker evidence. | **superseded by delivery-first TASK-032/033 and absorbed into TASK-037/038 acceptance** |
| TASK-031 | Retained V1 compatibility and MVP-1 exit ledger | Compatibility must be a wrapper/projection over the identical composition, never a second orchestrator or truth. Exit evidence must be direct durable production-chain evidence. | Commit `61cca93` added the compatibility wrapper and explicit `legacy-delivery-result-v1` restart projection without inventing typed evidence (`apps/lattice-runtime/src/delivery_ledger.rs`, `apps/lattice-runtime/src/composition.rs`). The old fake-era MVP-1 exit ledger was replaced by TASK-037/TASK-038 production acceptance and remains open. One number therefore contains one landed control and one superseded/open gate. | **split: compatibility already landed; old exit-ledger meaning superseded; production acceptance still open elsewhere** |

## Why No New Ticket, RED Fixture, Or Implementation Was Created

There is no eligible control that is simultaneously still missing, not
superseded, independent of active files, and legal under the prohibition on a
new runtime/orchestrator/provider path:

1. The remaining positive Review Runtime owner would create the exact new
   runtime path prohibited in this window.
2. Even an owner-only pure redesign needs new shared authority types and ports,
   then Policy/Orchestrator consumption. Those files are actively modified by
   TASK-038 and their next stable contract versions are not yet committed.
3. A RED fixture written against base `845328d` would freeze stale Contracts
   1.11 / Ports 1.7 / Orchestrator 2.2 assumptions while TASK-038 is moving to
   Contracts 1.12 / Ports 1.8 / Orchestrator 2.3.
4. The safe negative Review Runtime behavior and exhaustive read-only role/
   action/security matrix already exist under TASK-015, so another pure Policy
   slice would duplicate landed behavior rather than close a real gap.
5. Fake Codex, fake/offline orchestration, and legacy compatibility are already
   covered by the production adapter/composition path and test fixtures. New
   parallel implementations would violate the delivery-first decision.

Creating a ticket or deliberately RED test under these conditions would be
false precision, not a cherry-pickable hardening control.

## Minimal Renumber / Supersede Recommendation

Do not backfill four tickets named TASK-028 through TASK-031. At the next
authorized PLANS/HANDOFF integration window, record only these aliases:

- `TASK-029 -> superseded by TASK-032`;
- `TASK-030 -> superseded by TASK-032/033; acceptance continues in TASK-037/038`;
- `TASK-031 compatibility -> absorbed by TASK-032`;
- `TASK-031 exit gate -> superseded by TASK-037/038 production acceptance`;
- `TASK-028 -> retain as an unnumbered independent-review authority capability
  gap`, unless a later stable contract audit proves it blocks the runnable
  path.

If independent-review authority becomes an actual blocker after TASK-038
produces a clean stable checkpoint, allocate one then-current unused task
number rather than retroactively claiming TASK-028. The first bounded ticket
must be owner-only and pure: fixed frozen subject, bounded findings, exact
idempotency/replay, receipt/current-head substitution matrix, deterministic
fake, and fail-closed Policy consumption. It must explicitly exclude process/
provider launch, product writes, writer lease acquisition, new orchestrator,
PostgreSQL schema, and production acceptance. Reconstruct its RED fixtures only
after rebasing on the stable contract versions.

## Integration-Window Evidence

### READY

- This crosswalk is an isolated documentation artifact based exactly on
  `845328d`.
- It changes no root PLANS/HANDOFF, application code, contract, module
  constitution, ticket, test, runtime, database, service, credential, or
  production state.
- It is suitable for cherry-pick as audit/scheduling evidence if the
  integration owner wants a durable record.

### BLOCKED

- No TASK-028～031 completion claim is valid.
- No positive Review Runtime authority implementation is safe before a stable
  TASK-038 checkpoint and a fresh non-overlap audit.
- TASK-037 production `Hermes -> Memory -> Status` and TASK-038 Phase 2 remain
  separate open acceptance gates.

### Resource-bounded verification

Per the 2026-08-10 resource update, this audit did not start full Cargo,
PostgreSQL, WSL, or production acceptance. No behavior changed, so no focused
Rust test was justified. Verification for this artifact is limited to Git
scope/status, project-document check, diff hygiene, and exact commit evidence.
