# V2 Bootstrap Preservation Record

## Source Worktree

- Path:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos`
- Branch: `feature/phase1-controlled-swarm`
- Observed base HEAD: `06c3954`
- State before bootstrap: 31 dirty paths, including preserved TASK-004 Node
  code/test work and V2 governance files.

## Preservation Decision

The dirty source worktree will not be reset, cleaned, stashed, switched, or
used for Rust implementation. A dedicated sibling Git worktree was created
from the observed base HEAD on branch
`feature/v2-rust-postgres-bootstrap`. The approved governance files were copied
to that worktree without copying the preserved V1 implementation paths.

The snapshot must include only the approved V2 governance/document paths. It
must exclude these preserved V1 implementation paths:

- `src/workspace/errors.js`
- `src/workspace/git-workspace.js`
- `src/workspace/project-lock.js`
- `test/git-workspace.integration.test.js`
- `test/workspace-lock.test.js`

No ignored or untracked user-project content is copied into the V2 worktree.

## Target Worktree

- Path:
  `C:\Users\f7212\Documents\Codex\2026-07-29\files-mentioned-by-the-user-2026\outputs\lattice-devos-v2`
- Branch: `feature/v2-rust-postgres-bootstrap`
- Base commit: `06c3954`
- Governance state: uncommitted and reviewable in the target worktree; no
  synthetic commit, merge, or push was created.

## Verification

1. Compare source status before and after worktree creation.
2. Verify the target's copied governance set contains no changed `src/` or
   `test/` path before Rust implementation begins.
3. Verify the target worktree branch and HEAD.
4. Verify the five preserved V1 implementation paths remain modified only in
   the source worktree.

## Authorization

The user authorized this local, reversible first implementation slice on
2026-07-29 with `好 開始執行`. No merge, push, publication, deployment,
database execution, installation, or external action is authorized.
