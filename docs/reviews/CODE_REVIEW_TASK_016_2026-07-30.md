# TASK-016 Independent Code And Security Review

## Target

- Worktree: `lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base HEAD: `06c3954cd941dc6b743e9e2a1d4f94c3658b3ff9`
- Specification: SPEC-002 version 12, AC-30
- Ticket: TASK-016
- Active contracts: Artifact Store 1.0 and Contracts 1.6
- Final read-only re-review: 2026-08-01

## Review RED Findings And Resolutions

Independent review rejected earlier implementations until every accepted
finding had a direct regression and repair:

- P1: checkpoint creation cloned the payload-bearing owner before clearing its
  byte backend. `snapshot_metadata_clone` now constructs a metadata owner with
  an empty backend from the beginning, and the checkpoint itself retains no
  owner or metadata rows.
- P1: untrusted canonical byte limits were checked only after canonical output
  allocation. Iterative preflight now computes exact JSON/NFC encoded size,
  including control-character escape expansion, before canonicalization.
- P2: aggregate replay compared raw input with a private trusted owner clone and
  returned that clone. Replay now strictly reconstructs lifecycle, history,
  quotas, staging, command attribution, retired scopes, and terminal receipts
  from raw input without owner context, validates every join/digest, and only
  then compares compact trusted commitments.
- P2: complete terminal lifecycle receipts were not present in raw aggregate
  snapshots, so context-free restoration could not reproduce exact applied
  retries. Raw snapshots now retain the full sanitized lifecycle receipt plus
  its digest; reconstruction validates both.
- P2: `FieldBytes` accounting omitted read holder identifiers, some retained
  lifecycle strings, and the 64-byte domain-separated delete claim token.
  Quota projection and exact/plus-one tests now cover them.
- Quota review confirmed that task object and active-byte attribution must be
  active-reference-only. Released references now reduce those task projections
  to zero while project/store retained-object accounting remains unchanged.
- Final review noted a non-blocking direct-evidence gap for applied retry after
  replay. A regression now proves both applied and denied exact retries return
  byte-identical terminal receipts after context-free reconstruction.

Additional adversarial tests reject duplicate histories, orphan reference map
keys, quota projection tampering, full lifecycle-receipt tampering, coherent
older prefixes, unknown/extra/reordered/truncated/cross-scope/fake-live input,
excessive depth, and canonical control-character expansion.

## Final Result

Code review: `PASS`.

Security review: `PASS`.

Remaining findings: P0=0, P1=0, P2=0, P3=0. No integration blocker remains.

## Verification Evidence

- Contracts: 32/32 tests pass.
- Artifact Store: 97/97 tests pass, including replay 8/8.
- Locked full Rust workspace: 322/322 tests pass.
- Preserved Node characterization: 38/38 tests pass; project check reports
  `check=ok`.
- Strict locked workspace Clippy with `-D warnings`: pass.
- Rust format and `git diff --check`: pass.
- Normal dependency tree: only Contracts, cjson, SHA-256, time, and their
  approved transitive dependencies.
- Forbidden I/O, provider dependency, product dependency, and unrelated-
  website scans: zero implementation matches.
- Raw-byte containment: deterministic snapshot/checkpoint/debug tests prove
  fixture payloads are absent; replayed metadata deliberately has no payload
  backend and verified reads return `MissingBytes`.

Reviewed source hashes:

- `src/aggregate.rs`: `f5eb775fa60e6b22556a38776bcd6b4932f45dc78b2c25b22b0136766b135879`
- `src/aggregate/snapshot_restore.rs`: `9b3b808bf16d1bc4a8405982c7217d90a5ed94cee47ab9a48afaa5a899d8e3de`
- `src/snapshot.rs`: `782d6ce0cd05d35c4d153850689519eed6180985e3a3924361125d2c04b43b92`
- `src/quota.rs`: `5e533323ad865948168f89f80b36d72e6a76dff9f6d748d6d597a94ba5d4fee6`
- `src/semantics.rs`: `8347cbe902cadec1ae8c2c8f06050eb44ab01ddc196efa87f211a60f1076d4eb`
- `tests/artifact_owner_quota.rs`: `24758d947820397609646dc31cb264911b211513ffa7a8888058cab7b443f0f3`
- `tests/artifact_owner_replay.rs`: `4c4590f18aea6f6850e80d753b58662c5b32443b7efb70421eb200b2a7a78f89`

## Documented Residuals

- The in-memory fake is not durable PostgreSQL truth.
- The fake byte backend is not a real contained filesystem staging/delete
  adapter and proves no Windows link/reparse/TOCTOU safety.
- Fixed fake authorities prove binding, not live authentication or provider
  compatibility.
- Remote Rust CI, branch protection, a committed candidate, primary-branch
  merge authorization, publication, and deployment remain absent.

These are explicit later-ticket boundaries, not defects in bounded AC-30.
