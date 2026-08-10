# TASK-022 Durable PostgreSQL Project Registry Handoff

Status: `LOCAL_ACCEPTANCE_PASS / INDEPENDENT_REVIEW_PASS / PUSH_PENDING_COMMIT`

Repository: `z72124223/lattice-devos`

Branch: `feature/task-022-postgres-project-registry`

Base/working HEAD: `2b424ec9a5401a6fbdc4f37d3d401592331afca0`

Worktree: `C:\Users\f7212\Documents\Codex\2026-07-29\lattice-worktrees\task-022-postgres-project-registry`

## Delivered Candidate

- Project Registry 1.2 is the sole zero-I/O owner of global project identity,
  lifecycle, reservation, reconciliation, receipt, replay, and checkpoint
  semantics. Fake and PostgreSQL consume the same pure plan/apply path.
- Immutable migration `0005_project_registry_repository.sql` advances only an
  exact bare V1/V2/V3 source to bare global schema V4. Its current identity is
  200,547 bytes and
  `b7af1f8a8ac370bbfc8a5312497461587cb8a86eb32ff97e5b865c7ae9bf0dcf`.
- `PostgresProjectRegistry` uses one caller-supplied authenticated client and
  fixed functions only. Exact replay and changed-ID classification precede
  mutable admission; first-seen work requires exact ACTIVE authority.
- Historical Store-v2 and Task Ledger receipts/checkpoints replay
  byte-identically through bare V4 successors.
- TASK-038 compatibility remains the separately frozen global V3 + Memory v2
  + Writer Lease v1 profile. V3 extensions are rejected as V4 migration
  sources and all V4 extension combinations are unsupported and unadvertised.
- The harness invokes the Writer Lease owner's own live suite externally, then
  lets Store perform only fixed read-only profile/catalog/ACL assertions. Store
  has no normal or dev dependency on the Writer Lease adapter.

## Current Executable Evidence

- Pure Registry: 37 PASS (2 unit + 35 integration).
- Registry adapter: 1 PASS; migration contract: 17 PASS; Store package: 65
  PASS.
- Exact no-flag PostgreSQL harness: two independent marker-owned PostgreSQL
  17.10 initial/restart clusters PASS, including
  `TASK019_WRITER_LEASE_OWNER_PROFILE=PASS` and
  `TASK019_COMPOSITE_PROFILE_HARNESS=PASS`.
- Full locked/offline Rust workspace all-target/all-feature tests PASS.
- Node verify PASS: project check plus 44/44 tests.
- Strict Clippy PASS for both changed crates and for the proportional workspace
  excluding the unchanged Hermes package. The unexcluded workspace invocation
  fails only on 11 pre-existing `lattice-hermes-adapter` lints; the candidate
  has no Hermes diff and TASK-022 may not edit that path.
- `cargo audit` PASS: 1,198 advisories loaded, 118 dependencies scanned.
- `cargo tree -p lattice-postgres-store --edges normal,dev` contains no Writer
  Lease edge. Format, diff, migration-prefix, dynamic-SQL, allowed-path, and
  secret gates pass in the local candidate.

## Independent Findings Closed In This Lane

- The proposed V4+Memory reviewer route was rejected by exact authority
  arbitration; no `V4_MEMORY_*` contract was invented.
- Caller-supplied STOPPED exact replay/changed-ID ordering was repaired in Rust
  and SQL, with live regressions for exact, changed, and first-seen commands.
- The forbidden Store-to-Writer-Lease dev dependency and installer calls were
  removed, with a static regression and `normal,dev` dependency proof.
- Existing hook-only callers again select the frozen V3 Memory profile;
  StoreOnly+hook remains rejected, composite execution returns to its caller,
  and `psql.exe` is checked before cluster creation.
- Final independent code/security/architecture review is
  `APPROVED_WITH_RESIDUALS` with P0/P1/P2/P3 all zero. Its only residual is the
  explicitly recorded, unchanged, out-of-scope Hermes Clippy baseline.

## Remaining Closeout

1. Stage only TASK-022 allowed paths, repeat staged diff/secret/scope checks,
   and create a clean task-owned checkpoint commit.
2. Repeat fetch/auth/divergence preflight and perform an ordinary push to exact
   `feature/task-022-postgres-project-registry` only if no remote divergence is
   present.
3. After proving preliminary `git ls-remote` SHA equality, update the
   ticket/spec/ledger and this handoff to completed publication truth in a
   final closeout commit. Repeat fetch/auth/divergence checks, ordinary-push
   that closeout commit, and only then use `git ls-remote` again to prove the
   remote SHA equals the final local SHA.

No force push, PR, primary merge, tag, release, deploy, production database,
credential discovery, or remote CI claim is authorized or performed here.
