# LATTICE Development Governance v1

Status: active governance amendment pending merge authorization  
Effective decision date: 2026-08-11  
Normative architecture reference:
`docs/adr/ADR-024-connectivity-first-microkernel-and-mcp-providers.md`

This document defines the minimum trustworthy handoff, Issue-authority, and
simplicity review required for LATTICE development. It governs evidence and
workflow; it does not create a second Task truth, queue, orchestrator, or
release authority.

## 1. Checkpoint Receipt v1

Every push that changes a remote branch requires one receipt bound to the exact
remote SHA. The receipt is stored either:

1. in the same logical checkpoint as a repository document; or
2. immediately after the push as a comment on the canonical Issue.

A receipt is invalid if its branch no longer points to its recorded SHA unless
it explicitly identifies an immutable historical checkpoint.

### Required fields

```yaml
schema: lattice.checkpoint-receipt/v1
canonical_issue: 0
issue_relationship: CANONICAL
branch: feature/example
base_sha: 0000000000000000000000000000000000000000
parent_sha: 0000000000000000000000000000000000000000
commit_sha: 0000000000000000000000000000000000000000
tree_sha: 0000000000000000000000000000000000000000

development_started_at: 2026-08-11T00:00:00+08:00
development_ended_at: 2026-08-11T00:30:00+08:00
timezone: Asia/Taipei
elapsed_seconds_monotonic: 1800

changed_paths:
  - exact/path

checks:
  - name: format
    command: cargo fmt --check
    exit_code: 0
    evidence_level: LOCAL_VERIFIED
    started_at: 2026-08-11T00:25:00+08:00
    elapsed_seconds_monotonic: 4
    evidence_path: null
    evidence_digest: null

live_components_exercised: []
not_tested:
  - remote CI
known_blockers: []

remote_sha_verified: true
ahead: 0
behind: 0
worktree_clean: true
ci_status: NOT_CONFIGURED
branch_protection: OFF
pr_status: NONE
merge_status: NOT_MERGED
deployment_status: NOT_VERIFIED
release_status: NONE
highest_evidence_level: REMOTE_VERIFIED
```

### Time semantics

- `development_started_at` and `development_ended_at` are declared wall-clock
  observations and must include timezone.
- `elapsed_seconds_monotonic`, when available, is the trusted duration measure
  for a process or development window.
- Git author/committer time and GitHub push time are retained as corroborating
  evidence but do not prove development duration.
- Missing monotonic duration must be `UNKNOWN`, not estimated from chat history.

### Check evidence

Every check records the exact command or service, result, and evidence level.
Do not summarize several commands as one generic PASS. If raw output can contain
secrets, retain only an allowlisted result, fixed reason code, elapsed time, and
digest of the protected evidence.

The report must explicitly list `NOT_TESTED` items. Silence never means passed.

## 2. Evidence levels

### `DECLARED`

A statement by an agent or human with no independently retained current command
or remote binding.

### `LOCAL_VERIFIED`

Current local file, command, digest, process, or service evidence was retained.
This does not prove remote synchronization, CI, or a real external product.

### `REMOTE_VERIFIED`

GitHub independently confirms the exact branch and SHA. It does not prove local
worktree cleanliness unless the receipt separately records local evidence.

### `CI_VERIFIED`

A remote CI service ran the stated checks against the exact SHA and passed.
Configuration files without a workflow run are not CI evidence.

### `LIVE_VERIFIED`

The named real external component and complete stated boundary were exercised.
Examples:

- real ChatGPT tool invocation;
- real Codex provider execution;
- PostgreSQL physical restart and fresh-process replay;
- real Hermes provider call.

A lower-level gate cannot substitute for a higher-level gate:

```text
tool discovery != tool invocation
local MCP invocation != ChatGPT invocation
same-session status != cross-session durable replay
unit PostgreSQL tests != physical restart replay
```

## 3. Trust rules for later GPT and agents

Later consumers must:

1. verify the referenced branch and exact SHA;
2. compare the receipt with current remote state;
3. distinguish local, CI, and live evidence;
4. preserve explicit blockers and `NOT_TESTED` items;
5. reject any broader success claim than the retained evidence supports.

If a receipt is missing, stale, internally inconsistent, or not bound to an
exact SHA, report `CHECKPOINT_UNTRUSTED` and inspect the repository directly.

## 4. Issue authority and overlap

Every capability has one canonical long-term Issue. Implementation slices may
have their own Issues, but each declares its relationship to the canonical
Issue.

### Relationship vocabulary

- `CANONICAL`: long-term authority for the capability.
- `DUPLICATE_OF`: fully redundant; no independent implementation scope.
- `OVERLAPS_WITH`: partial overlap; both Issues state the exact boundary.
- `SUPERSEDES`: replaces named sections or decisions of another Issue.
- `SUPERSEDED_BY`: points to the newer effective authority.
- `SPLIT_FROM`: narrower execution slice extracted from a larger Issue.
- `MERGED_INTO`: remaining work and authority moved elsewhere.
- `HISTORICAL`: retained as evidence but not current specification.

### Required overlap record

```yaml
capability_id: CAP-EXAMPLE
canonical_issue: 4
relationship: SUPERSEDES
effective_at: 2026-08-11T00:00:00+08:00
affected_sections:
  - old synchronous acceptance requirement
remaining_authority:
  - transport and bounded schema requirements
```

### Creation and expansion gate

Before creating or materially expanding an Issue:

1. search titles, bodies, comments, capability IDs, modules, provider names,
   and reason codes;
2. compare acceptance criteria and owned data;
3. select an existing canonical Issue or record the exact relationship;
4. stop on unresolved conflict and request responsible-user reconciliation.

An agent cannot resolve a conflict by silently following whichever Issue it
read last. Chronology is evidence, but explicit supersession is authority.

## 5. Current canonical mapping

- Issue #4: ChatGPT MCP connectivity and bounded gateway.
- Issue #5: capability-oriented providers, replacement, fallback, retirement.
- Issue #9: Reflection, evidence learning, asynchronous Hermes evolution.
- Issue #10: circuit breaking, segmented acceptance, local invalidation,
  multi-project governance.

Issue #9 supersedes any ordinary-task requirement in Issue #4 or historical
TASK-037 documentation that makes synchronous Hermes success a condition for
core Task completion. Issue #4 otherwise remains authoritative for ChatGPT MCP
transport and bounded tool behavior.

## 6. Simplicity review

The optimization target is `time_to_verified_outcome`, not code volume or first
visible progress.

Before implementation, write a short comparison:

```yaml
required_capability: example
candidates:
  - reuse existing reviewed capability
  - narrower contract / smaller slice
  - mature replaceable provider
  - new internal implementation
selected: narrower contract / smaller slice
reason: shortest path with equivalent safety and evidence
complexity_added:
  crates: 0
  processes: 0
  protocols: 0
  schemas: 0
  queues: 0
  truth_sources: 0
  writers: 0
```

### Mandatory complexity justification

Required before adding:

- a Rust crate;
- a runtime process or MCP server;
- a protocol or public listener;
- a database schema or migration;
- a queue, scheduler, or long-running worker;
- an authentication or credential system;
- a mutable projection claimed as current state;
- a truth source or writer.

The justification explains why reuse or a smaller slice cannot deliver the same
verified result and how the new component is removed, replaced, health-checked,
and rolled back.

### No unsafe simplification

Simplicity cannot waive:

- One Gateway, One Truth, One Writer;
- Task legality and Policy;
- Writer Lease and fencing;
- scope and credential isolation;
- durable evidence and exact replay;
- independent verification and rollback;
- protected human / Guardian gates.

## 7. Connectivity-first development order

Until the responsible user changes priority:

1. prove current real ChatGPT discovery;
2. prove a real read-only ChatGPT -> LATTICE -> ChatGPT round-trip;
3. prove or record a platform blocker for the fixed bounded Submit action;
4. prove cross-session PostgreSQL-backed Status with zero repeated effects;
5. only then consider broader controls or provider extraction;
6. GH-9 live PostgreSQL replay and TASK-037 formal remain paused.

Do not modify Rust merely because a ChatGPT workspace, plan, connector snapshot,
or platform permission blocks a write action. Record
`PLATFORM_CAPABILITY_BLOCKED` with versioned evidence and compare approved
alternatives.

## 8. Enforcement state

These rules are immediately normative for human and agent behavior. Repository
automation, schemas, hooks, CI checks, and branch protection may later enforce
them, but their absence must be reported and cannot be described as existing
enforcement.