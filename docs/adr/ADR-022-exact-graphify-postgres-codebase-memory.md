# ADR-022: Exact Graphify To PostgreSQL Codebase Memory

- Status: accepted for the pure/adapter boundary; PostgreSQL extension
  implementation pending an explicit versioned owning-module amendment
- Date: 2026-08-05
- Decision owner: user
- Related: SPEC-002 v32, ADR-004, ADR-005, ADR-006, ADR-014, ADR-021,
  ADR-019, ADR-020, TASK-022, TASK-032, TASK-033, TASK-075

## Context

The scripted TASK-032 delivery checkpoint already produces a real isolated Git
commit and durable PostgreSQL receipt. Official Codex live remains
`FAILED_DIAGNOSTIC` and cannot be retried, but that incident need not prevent a
separate read-only graph/memory node over the committed fixture.

Graphify itself is not Git-object-aware: it walks a filesystem path. Its raw
graph is derived output and does not provide LATTICE's exact project/commit/
tree/source-manifest binding. Codebase Memory also cannot become a second
truth, writer, gateway, or authority source.

Official upstream evidence on 2026-08-05 identifies latest stable release
`v0.9.33`, lightweight tag commit
`4e7e6b1f7e0df10ed07d5f28f9189bbde42940f1`, package
`graphifyy==0.9.33`, Apache-2.0, and PyPI wheel SHA-256
`c32b5792c783a6e66b1100b35bc65df3538e3f69b9df45fb098c9634c1b8eb01`.
The actual installed CLI advertises the bounded headless extraction flags used
below. The unsigned lightweight tag is recorded as supply-chain evidence, not
misrepresented as a signed tag.

## Decision

1. LATTICE pins Graphify to the exact version, commit, package artifact hash,
   complete dependency payload manifest, license, adapter version, and fixed
   capability/config digest. The execution identity also binds the reviewed
   WSL launcher/distribution, Python 3.14.4, and bubblewrap 0.11.1 boundary.
   Runtime never follows `latest` automatically.
2. `CodeSnapshotPort` materializes only files tracked by one exact Git commit
   into a LATTICE-owned immutable snapshot. It records project, snapshot,
   commit, tree, sorted path/content-digest manifest, manifest digest, and
   exclusion digest. Untracked working-tree content and secrets cannot enter.
3. `GraphifyAnalysisPort` invokes only the process-owned equivalent of:

   ```text
   graphify extract <snapshot> --code-only --no-cluster --max-workers 1 --out <staging>
   ```

   The fixed Windows path is `wsl.exe --exec` directly into bubblewrap, never a
   shell. Bubblewrap unshares user/mount/network namespaces, disables nested
   user namespaces, exposes read-only ingress, and gives the identity-bound
   embedded runner private runtime/source/output tmpfs. The runner verifies
   copied bytes, requires Landlock ABI 3 plus a direct truncate-denial probe,
   and returns only one strict framed result through exclusive parent handles.
   LATTICE clears the child environment, sets
   `GRAPHIFY_QUERY_LOG_DISABLE=1`, and never invokes install, hooks, query,
   watch, global, live PostgreSQL, semantic backend, or in-source output.
4. A successful exit is insufficient. The adapter strictly parses the complete
   `graphify-out/graph.json`, rejects unknown structure or source provenance,
   normalizes bounded structural nodes/edges, preserves
   `EXTRACTED/INFERRED/AMBIGUOUS`, sorts canonically, and computes graph,
   record-set, and terminal analysis digests. Timeout, malformed/partial output
   or a changed binding rejects before memory persistence.
5. `lattice-codebase-memory` is pure Rust. It validates normalized structural
   observations, plans `OBSERVATION/CANDIDATE` records, and deterministically
   ranks a fixed process-owned query. Exact identifier/path/token matches
   outrank partial matches and record digest breaks ties. It stores no raw
   source and grants no trust or authority.
6. The durable repository is planned as an independent same-database Memory
   extension profile at `db/extensions/codebase-memory/v1.sql`. It has its own
   exact embedded SQL/hash, Memory identity/extension ledger, explicit admin
   runner, four domain tables, fixed `SECURITY DEFINER` functions, and a
   V3+Memory catalog/ACL verifier. It does not join or advance the global
   migration manifest. Normal runtime has no direct table access. Analysis
   staging/finalize is one bounded `SERIALIZABLE` transaction; retrieval audit
   is atomic and exact-snapshot bound. Production implementation waits for an
   explicitly approved versioned constitution for the owning persistence
   boundary.
7. Exact `(project, commit)` is the retrieval boundary. A later commit does not
   delete historical evidence, but it becomes the only current binding for
   that configured run and cannot return records from the old commit. Reusing
   the same commit with different manifest/tool/config evidence fails closed.
8. Orchestrator 2.2 alone orders snapshot -> Graphify -> normalize/validate ->
   durable persist -> deterministic retrieve/audit -> receipt. Every first
   failure suppresses later effects.
9. `latticed` 1.1 retains exactly `lattice_delivery_run` and
   `lattice_delivery_status`, both zero-parameter. Run extends the fixed
   scripted checkpoint; status returns delivery plus durable analysis/retrieval
   evidence. No third tool or caller-provided query/path/shell/SQL/credential is
   introduced.

## PostgreSQL Extension Boundary

ADR-020 and Postgres Store 1.4 remain authoritative for the Project Registry's
reserved global `0005`/schema-v4 profile. TASK-033 neither supersedes nor
renumbers it. Historical migration bytes `0001` through `0004`, the current
global v3 runtime surface, and TASK-022 governance remain unchanged. Memory's
independent extension identity and ledger cannot masquerade as
`control.migration_history` or `control.schema_compatibility`.

## Consequences

- Graph evidence is reproducible from a pinned tuple and queryable after
  database/process restart, but remains derived candidate evidence.
- The pure domain is independently testable; Git, Graphify, filesystem,
  process, JSON, and PostgreSQL remain edge adapters.
- Malformed, partial, timed-out, ambiguous, secret-bearing, untracked, or
  cross-snapshot evidence cannot become a durable success.
- Hermes and OpenClaw remain later nodes. Official Codex live remains
  `FAILED_DIAGNOSTIC`; this ADR does not alter its safety posture.

## TASK-075 Schema-v5 Compatibility Amendment

Codebase Memory remains outside the global Store manifest. Its v1/v2 SQL bytes
and v2 identity (extension schema 2, global schema 3) are immutable. TASK-075
adds only `db/extensions/codebase-memory/v3.sql`: fresh global-v5 installation
and exact v2-to-v3 upgrade converge on the same verified catalog/ACL profile;
partial, drifted, extra, ambiguous, or substituted profiles fail closed with
no automatic repair.

Contracts 1.13 keeps v1/v2 persistence-identity constructors bound to global
schema 3 and adds a distinct extension-v3/global-v5 constructor. Postgres
Codebase Memory 1.1 retains complete global/extension profile provenance on
every authoritative analysis, retrieval, graph-receipt, and reflection row,
backfills old rows with their exact v2 profile, cross-checks related rows, and
uses that row profile when replaying graph-memory or Hermes-reflection
receipts. The current adapter identity never rehashes historical receipts. This changes no pure
Codebase Memory/Graphify/Hermes semantics or MCP surface.

## Verification

- Contract/port/orchestrator RED-GREEN call-order and rejection tests.
- Exact Git fixture proving tracked-only snapshot, commit/tree binding,
  changed-source invalidation, and deterministic manifest.
- Real pinned Graphify extraction, complete LATTICE-owned dependency-payload
  tamper rejection, reviewed system-boundary identity/preflight,
  namespace/mount/network probes, and timeout/malformed/partial-output fakes.
- PostgreSQL V3+Memory extension install/no-op/restart/replay/ACL/catalog/
  zero-side-effect tests after the owning-module amendment is approved.
- Existing two-tool MCP enumeration and extra-property rejection tests.
- Independent code and architecture review before checkpoint commit.
