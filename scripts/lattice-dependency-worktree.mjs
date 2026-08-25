import process from "node:process";

import { GitWorkspace, WorkspaceError } from "../src/workspace/git-workspace.js";

const DEPENDENCY_SCHEMA = "lattice.dependency-blocker/1.0";
const TASK_ID_PATTERN = /^TASK-[A-Z0-9][A-Z0-9_-]{2,58}$/;
const WORKTREE_ID_PATTERN = /^[A-Z0-9][A-Z0-9_-]{2,63}$/;
const BASE_SHA_PATTERN = /^[0-9a-f]{40}$/;

async function main() {
  const [operation, parentTaskId, dependencyTaskId, worktreeId, baseSha, ...extra] =
    process.argv.slice(2);
  if (
    operation !== "create" ||
    extra.length !== 0 ||
    [parentTaskId, dependencyTaskId, worktreeId, baseSha].some(
      (value) => typeof value !== "string" || value.length === 0,
    ) ||
    !TASK_ID_PATTERN.test(parentTaskId) ||
    !TASK_ID_PATTERN.test(dependencyTaskId) ||
    parentTaskId === dependencyTaskId ||
    !WORKTREE_ID_PATTERN.test(worktreeId) ||
    !BASE_SHA_PATTERN.test(baseSha)
  ) {
    throw new WorkspaceError(
      "DEPENDENCY_WORKTREE_ARGUMENTS_INVALID",
      "The dependency worktree command requires one closed create request.",
    );
  }
  const worktreeRoot = process.env.LATTICE_DEPENDENCY_WORKTREE_ROOT;
  if (typeof worktreeRoot !== "string" || worktreeRoot.length === 0) {
    throw new WorkspaceError(
      "DEPENDENCY_WORKTREE_ROOT_MISSING",
      "The configured dependency worktree root is unavailable.",
    );
  }
  const workspace = new GitWorkspace({
    repositoryRoot: process.cwd(),
    worktreeRoot,
  });
  const created = await workspace.createWorktree({
    task_id: dependencyTaskId,
    worktree_id: worktreeId,
    base_commit_sha: baseSha,
  });
  process.stdout.write(
    `${JSON.stringify({
      schema: DEPENDENCY_SCHEMA,
      parent_task_id: parentTaskId,
      dependency_task_id: created.task_id,
      dependency_worktree_id: created.worktree_id,
      dependency_branch: created.branch,
      base_sha: created.base_commit_sha,
      next_action: "COMPLETE_DEPENDENCY",
    })}\n`,
  );
}

try {
  await main();
} catch (error) {
  const code =
    error instanceof WorkspaceError
      ? error.code
      : "DEPENDENCY_WORKTREE_CREATE_FAILED";
  process.stderr.write(`${code}\n`);
  process.exitCode = 1;
}
