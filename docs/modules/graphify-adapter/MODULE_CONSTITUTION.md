---
module_id: graphify-adapter
name: LATTICE Graphify Adapter
version: 1.1
status: active
owner: LATTICE maintainers
last_reviewed: 2026-08-05
---

## Mission

Run one exact pinned Graphify headless code-only extraction over a LATTICE-
materialized immutable Git snapshot and return strictly validated, digest-bound
derived graph evidence through the typed production port.

## Non-Goals

- Install Graphify skills/hooks, follow latest, query its memory, watch source,
  clone repositories, use global graphs, inspect live PostgreSQL, or invoke an
  LLM/backend.
- Select a project, commit, source path, output path, query, command, provider,
  environment, credential, or timeout from MCP/user arguments.
- Mutate product source, Git, PostgreSQL, Codebase Memory, policy, scope,
  approval, review, release, or deployment state.

## Owned Data

- Exact Graphify package/version/upstream commit/license/wheel, complete
  LATTICE-owned Python payload manifest, and reviewed WSL/Python/bubblewrap
  execution identity.
- Fixed namespace/mount/environment profile, owned launcher lifetime, separate
  staging output, strict parser, bounded normalized evidence, and diagnostics.
- No source, task, project, memory, database, credential, or authority truth.

## Public Contracts

- Implement `GraphifyAnalysisPort` only for one validated `CodeSnapshotEvidence`.
- Invoke only fixed `wsl.exe -d Ubuntu --exec bwrap ... python3.14 -I -S -B -c
  <embedded-runner>`; the identity-bound runner performs only pinned Graphify
  version/help plus `extract /source --code-only --no-cluster --max-workers 1
  --out /output`. No shell or caller-selected command/path enters this shape.
- Unshare user/mount/network namespaces, disable nested user namespaces, copy
  read-only ingress into private `/runtime` and `/source` tmpfs, verify the
  copies, require Landlock ABI 3 with a direct truncate-denial probe, and copy
  out only one strict framed result through parent-owned exclusive handles.
- Clear provider/backend environment and set
  `GRAPHIFY_QUERY_LOG_DISABLE=1` before starting the child.
- Require successful exit plus one complete strict graph output. Every
  `source_file` must match the supplied tracked-source manifest.
- Preserve confidence provenance, canonicalize/sort bounded nodes and edges,
  and return tool/config/input/output/record-set digests.
- Kill/reap only the owned process tree on deadline; timeout, malformed/partial
  output, unknown provenance, overflow, or teardown ambiguity fails closed.

## Invariants

1. Source is read-only; staging is disjoint and LATTICE-owned.
2. No provider/API-key environment reaches the child.
3. No install/hook/query/watch/global/postgres/backend command is reachable.
4. Exit zero alone never proves valid analysis.
5. The adapter performs no durable write and grants no authority.
6. Raw source text and secrets never appear in returned evidence/diagnostics.
7. Process capture remains outside the child-visible staging bind; Windows Job
   ownership is lifecycle control, while bubblewrap is the filesystem/network
   containment boundary.

## Allowed Dependencies

- `lattice-contracts` 1.11 and `lattice-ports` 1.7.
- Process, filesystem, strict JSON, hashing, timeout, and path-containment
  libraries needed at the adapter edge.
- Direct Win32 Job Object FFI is confined to one crate-private lifecycle
  module; unsafe code remains denied in every other adapter module.

## Forbidden Dependencies

- Orchestrator internals, PostgreSQL clients, Codebase Memory implementation,
  Codex/OpenClaw/Hermes SDKs, Git mutation, provider SDKs, network clients,
  Guardian/release/deployment/payment code, or unrelated product modules.

## Acceptance Gates

- Complete LATTICE-owned dependency-payload identity/tamper rejection plus
  reviewed system-boundary identity/help/preflight, real controlled-fixture
  extraction, provenance and deterministic tests.
- OS probes prove distinct user/mount/network namespaces, verified private
  runtime/source copies, Landlock truncate denial, private-output-only writes,
  invisible host siblings, and no external network.
- Timeout, non-zero exit, missing/malformed/partial graph, foreign source,
  provider-env, overflow, and owned-child teardown rejection tests.
- Independent code/security and architecture review.

## Change Policy

Version any CLI shape, accepted raw schema, identity tuple, evidence schema,
environment policy, containment, dependency direction, or failure semantics.
Upstream version changes require a new exact pin and live fixture verification.

## Failure, Compatibility, And Migration

- Identity, process, teardown, schema, provenance, containment, or output
  ambiguity returns a typed failure and never a success receipt.
- Version 1.1 accepts only the pinned Graphify v0.9.33 capability and typed
  production ports; the frozen generic Graphify port is not a compatibility
  fallback.
- Staging is disposable derived output. This module owns no database schema,
  persistent migration, installer, hook, or in-place source migration.

## Amendment History

| Version | Date | Change | Authority |
|---|---|---|---|
| 1.0 | 2026-08-05 | Pinned exact-Git-snapshot Graphify extraction and strict typed evidence | TASK-033 user direction |
| 1.1 | 2026-08-05 | Identity-bound private tmpfs runner, strict framed copy-out, exclusive capture, and Landlock ABI 3 truncate gate | User-directed private-runtime containment repair |
