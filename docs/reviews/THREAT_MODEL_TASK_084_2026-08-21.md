# TASK-084 Threat Model

## Scope And Assets

This review covers the pure workflow-learning contract and its future durable
boundary. Assets are protected authority, the one durable truth/writer,
evidence integrity, project/worktree isolation, private data, cost limits,
dispatch correctness, policy history, and rollback availability.

## Trust Boundaries

- Untrusted: model/report text, chat, dashboard, sidebar, process discovery,
  provider output, stale files, and candidate policy.
- Corroborating evidence only: tickets, PLANS, dashboard, and Git observations
  until exact identity/freshness and cross-source consistency pass.
- Future authority: verified Task Ledger replay under current Writer Lease and
  fence. No current standalone fixture or checkpoint claims durability.

## Threats And Required Controls

| Threat | Failure | Control | Test evidence |
|---|---|---|---|
| Quiet/idle poisoning | silence becomes no-work | mandatory pre-stop challenge; quiet unrepresentable as proof | UNKNOWN/idle/all-stale replay |
| Projection poisoning | stale dashboard overrides tickets/Git | cross-source contradiction finding and freshness gate | TASK-080 fixture |
| Epistemic lock-in | one incident becomes timeless policy truth | expiring multi-hypothesis record; counterevidence lowers confidence | expiry/counterevidence tests |
| Confirmation bias | probe seeks only expected answer | competing hypotheses, falsifier, risk/cost and stop condition | probe-overturn test |
| Malicious report | false claim trains policy | untrusted source cannot support diagnosis/promotion | untrusted-report rejection |
| Sybil/duplicate event | one incident amplifies votes | exact event identity deduplication | duplicate idempotency |
| Tool/path omission | failed relative path hides work | tool coverage is a required stop-certificate source | TASK-080 wrong-path replay |
| Policy self-escalation | candidate grants protected action | closed safe rule vocabulary plus explicit authority-expansion rejection | protected authority matrix |
| Fence rollback | stale candidate replaces current policy | exact base version and current fence; monotonic promotion/rollback fence | version/fence matrix |
| Gate fabrication | boolean PASS without evidence | every required gate has fresh trusted evidence commitment | no-evidence promotion denial |
| Single-case overfit | one replay promotes a global rule | distinct multi-case and counterexample gates | single-case denial |
| Cost evasion | policy dispatches unlimited work | immutable hard cap and per-candidate bounded cost | hard-cap tests |
| Safety regression | new rule bypasses blocker/conflict | regression replay must preserve dependency/resource/duplicate denial | blocker fault injection |
| Privacy leak | chat/secret/raw output persisted | bounded typed fields and secret/transcript/path rejection | privacy matrix |
| Rollback suppression | degraded policy remains active | prior version retained; hard metric violation auto-rolls back | monitor test |
| Second truth/writer | local checkpoint becomes authority | pure checkpoint explicitly non-durable; future Ledger/Lease binding required | architecture gate blocked |
| Path escape/nesting | child worktree dirties parent or escapes approved root | resolved identity, direct ancestry, reparse and parent before/after probe | TASK-083 topology replay |
| Destructive repair | delete/move/reset/clean loses work | only Git-native worktree move receipt; exact pre/post identity | forbidden-action contract |
| Suspicion busywork | endless governance probes displace product value | queue expiry/dedup, value priority, cost/reversibility/conflict bounds | queue ranking tests |
| Suspicion-as-defect | hypothesis triggers mutation before proof | explicit hypothesis state; only discriminating probe allowed | queue state test |

## Protected Authority Ceiling

The learning module can never promote payment, credential/account, public
network exposure, force push, default-branch merge, deployment, release,
archive, security-control disablement, or durable-writer acquisition. Adding or
removing one of these items is a constitution/ADR change requiring direct user
authorization; it is not a learned policy revision.

## Abuse And Fault Cases

- Single malicious reporter plus duplicated event IDs.
- Fresh dashboard paired with stale Git; fresh Git paired with stale ticket.
- Every heartbeat missing; every window idle; sidebar empty.
- Ready backlog with no open window; dependency became satisfied after snapshot.
- Real blocker, same-worktree collision, duplicate task identity, and cost cap.
- Candidate created on policy N/fence F, then promotion attempted after either
  changes.
- Two plausible hypotheses remain unresolved; record expires or receives
  counterevidence; a bounded probe falsifies the initial judgment.
- Monitor metrics worsen immediately after promotion and after replay/restart.
- Nested worktree, reparse escape, wrong approved root, dirty parent before/after,
  and successful Git-native move with no destructive fallback.
- Fresh duplicate suspicion and low-value governance work competing with a
  higher-value product hypothesis.

## Residual Risk And Blocked Gate

The pure crate cannot prove authenticated durability, cross-process currentness,
or atomic policy activation. Those require TASK-079's committed contract plus
Task Ledger/Postgres/Writer Lease conformance. Until then the module is a
versioned characterization artifact and production promotion is blocked.

## Security Review Result

Bounded-slice result: `PASS`. No protected-authority expansion is present;
focused privacy, malicious-input, topology, cost, fencing, rollback, and
protected-authority tests pass. Production integration remains `NEEDS_REVIEW`
until the TASK-079 durable gate is available.
