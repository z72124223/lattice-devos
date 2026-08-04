# TASK-016 Architecture Review

## Triggers

- New Artifact Store semantic owner and aggregate replay/checkpoint boundary.
- Material Contracts 1.6 public artifact representations.
- Project isolation, quota ownership, raw-byte containment, delete claim, and
  future PostgreSQL/filesystem composition risk.
- Context-free reconstruction and compact rollback trust-anchor design.

## Independent Result

`PASS`. P0=0, P1=0, P2=0, and P3=0. No architecture amendment or integration
blocker is required.

Confirmed:

- `FakeArtifactStore` is the only public aggregate mutation entry. Lifecycle,
  history, and quota mechanisms remain crate-private and cannot become a
  second writer.
- Artifact Store introduced neither another human gateway nor durable truth.
  PostgreSQL remains the future control-plane One Truth; Graphify, Hermes,
  Codex, and other providers may supply provenance but cannot issue Artifact
  Store authority.
- Object, history, quota, staging, and command identities bind `project_id`.
  Equal digests across projects share no identity, lifecycle, reference set,
  quota head, or existence response.
- The checkpoint retains only store identity, immutable-limit commitment,
  rollback-sensitive trust anchor, complete snapshot digest, replay bounds,
  and its own domain-separated digest. It retains no payload, owner clone, or
  metadata row set.
- Raw replay preflights structural and encoded byte bounds, reconstructs every
  closed-schema owner row from raw data, recomputes semantic/hash chains and
  cross-map joins, and only then compares independently retained commitments.
- Snapshot, checkpoint, trust-anchor, history, quota, object/reference/read,
  receipt, and delete plan/claim/result use separate hash domains.
- Restore/parser modules are private Artifact Store responsibilities. The
  normal dependency graph remains acyclic and contains only Contracts, cjson,
  SHA-256, time, and approved transitives.
- The pure crate performs no filesystem, Git, database, process, network,
  environment, credential, provider, payment, publication, deployment, or
  product-repository I/O.
- The unrelated playmate website is absent from active source and architecture.

ADR-014, Artifact Store constitution 1.0, SPEC-002 v12, TASK-016, source, and
tests remain aligned; no versioned constitution amendment is needed.

## Constitution Acceptance Gates

All 13 Artifact Store constitution gates have local evidence: exact project
isolation; full provenance and fixed owner; bytes/digest/bounds; immutable
references; aggregate quotas and worst-case retention; exact idempotency;
typed current authority; read lifecycle; delete claim/reconciliation; higher
generation; strict replay/rollback; raw-byte separation; and forbidden-
dependency/I/O absence. PostgreSQL and filesystem gates remain explicitly
deferred rather than misclassified as passed.

## Enforcement Truth

| Gate | Classification | Evidence |
|---|---|---|
| Pure owner state, authority, quota, retry, replay, and checkpoint behavior | machine-enforced locally | Rust privacy/types and 97 Artifact Store tests |
| Project isolation and fixed producer/runtime contracts | machine-enforced locally | Contracts plus Artifact Store matrices |
| Dependency and no-I/O direction | machine-observed locally | Cargo tree, manifests, Clippy, static scans |
| Governance and ownership boundaries | documented plus structurally checked | SPEC v12, ADR-014, constitution 1.0, TASK-016 |
| PostgreSQL One Truth, transactionality, durability, and restart | missing/deferred | later MVP-1 tickets |
| Real filesystem containment and delete effects | missing/deferred | later owned-root adapter ticket |
| One Gateway/One Writer at composed runtime | documented-only in this slice | later IPC/orchestrator integration |
| Remote Rust CI and branch protection | missing/unverified | no remote/upstream evidence |

## Verification

- `cargo check -p lattice-artifact-store --locked`: pass.
- `cargo test -p lattice-artifact-store --test artifact_owner_replay --locked`:
  8/8 pass.
- `cargo test -p lattice-artifact-store --locked`: 97/97 pass.
- `cargo test --workspace --all-targets --all-features --locked`: 322/322
  pass.
- Strict locked workspace Clippy, format, dependency, forbidden-I/O/provider/
  product scans, preserved Node suite, project check, and diff hygiene: pass.

## Residual Non-Blocking Owner Work

PostgreSQL persistence/serialization/restart, filesystem staging/containment/
unlink, live authority authentication, OpenClaw/Codex/Graphify/Hermes adapters,
Codebase Memory, Guardian activation, remote CI, merge, and deployment remain
later bounded work. TASK-016 makes no claim that these exist.
