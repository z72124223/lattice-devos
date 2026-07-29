import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import {
  access,
  appendFile,
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import test from "node:test";

import {
  GitWorkspace,
  WorkspaceError,
  defaultGitExecutor,
} from "../src/workspace/git-workspace.js";
import { ProjectLock } from "../src/workspace/project-lock.js";

const execFileAsync = promisify(execFile);

async function git(cwd, args) {
  const result = await execFileAsync("git", args, {
    cwd,
    encoding: "utf8",
    windowsHide: true,
  });
  return result.stdout.trim();
}

async function exists(target) {
  try {
    await access(target);
    return true;
  } catch {
    return false;
  }
}

async function repositoryFixture(t) {
  const root = await mkdtemp(path.join(os.tmpdir(), "lattice-git-"));
  const repositoryRoot = path.join(root, "repository");
  const worktreeRoot = path.join(root, "worktrees");
  await mkdir(repositoryRoot);
  await mkdir(worktreeRoot);
  t.after(async () => {
    await rm(root, { force: true, recursive: true });
  });

  await git(repositoryRoot, ["init", "-b", "main"]);
  await git(repositoryRoot, ["config", "user.name", "LATTICE Test"]);
  await git(repositoryRoot, ["config", "user.email", "lattice-test@invalid.example"]);
  await writeFile(path.join(repositoryRoot, "tracked.txt"), "base\n");
  await writeFile(path.join(repositoryRoot, "old-name.txt"), "rename me\n");
  await writeFile(path.join(repositoryRoot, "delete-me.txt"), "delete me\n");
  await writeFile(path.join(repositoryRoot, "shared.txt"), "base\n");
  await git(repositoryRoot, ["add", "."]);
  await git(repositoryRoot, ["commit", "-m", "initial"]);
  const baseCommit = await git(repositoryRoot, ["rev-parse", "HEAD"]);

  return {
    root,
    repositoryRoot,
    worktreeRoot,
    baseCommit,
  };
}

test("creates, inspects, and safely removes an owned disposable Git worktree", async (t) => {
  const fixture = await repositoryFixture(t);
  const calls = [];
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
    executor: async (request) => {
      calls.push(structuredClone(request));
      return defaultGitExecutor(request);
    },
  });

  const created = await workspace.createWorktree({
    task_id: "TASK-2026-0001",
    worktree_id: "WORKTREE-0001",
    base_commit_sha: fixture.baseCommit,
  });

  assert.equal(created.base_commit_sha, fixture.baseCommit);
  assert.equal(created.branch, "lattice/task-2026-0001");
  assert.equal(await git(created.path, ["rev-parse", "HEAD"]), fixture.baseCommit);
  assert.equal(calls.every((call) => call.command === "git"), true);
  assert.equal(calls.every((call) => Array.isArray(call.args)), true);

  await appendFile(path.join(created.path, "tracked.txt"), "unstaged\n");
  await git(created.path, ["mv", "old-name.txt", "renamed.txt"]);
  await rm(path.join(created.path, "delete-me.txt"));
  await writeFile(path.join(created.path, "staged.txt"), "staged\n");
  await git(created.path, ["add", "staged.txt"]);
  await appendFile(path.join(created.path, "staged.txt"), "unstaged too\n");
  await writeFile(path.join(created.path, "untracked.txt"), "untracked\n");

  const changes = await workspace.changedFiles({
    worktree_id: "WORKTREE-0001",
    worktreePath: created.path,
    base_commit_sha: fixture.baseCommit,
  });
  const byOperation = Object.groupBy(changes, (change) => change.operation);

  assert.equal(byOperation.modify.some((entry) => entry.path === "tracked.txt"), true);
  assert.deepEqual(
    byOperation.modify.find((entry) => entry.path === "tracked.txt").states,
    ["unstaged"],
  );
  assert.equal(byOperation.delete.some((entry) => entry.path === "delete-me.txt"), true);
  assert.deepEqual(
    byOperation.delete.find((entry) => entry.path === "delete-me.txt").states,
    ["unstaged"],
  );
  assert.equal(
    byOperation.rename.some(
      (entry) =>
        entry.from_path === "old-name.txt" && entry.path === "renamed.txt",
    ),
    true,
  );
  assert.deepEqual(
    byOperation.rename.find((entry) => entry.path === "renamed.txt").states,
    ["staged"],
  );
  assert.equal(
    byOperation.create.some((entry) => entry.path === "staged.txt"),
    true,
  );
  assert.deepEqual(
    byOperation.create.find((entry) => entry.path === "staged.txt").states,
    ["staged", "unstaged"],
  );
  assert.equal(
    byOperation.create.some((entry) => entry.path === "untracked.txt"),
    true,
  );
  assert.deepEqual(
    byOperation.create.find((entry) => entry.path === "untracked.txt").states,
    ["untracked"],
  );

  await assert.rejects(
    workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0001" }),
    (error) =>
      error instanceof WorkspaceError && error.code === "WORKTREE_DIRTY",
  );
  assert.equal(await exists(created.path), true);

  await git(created.path, ["add", "-A"]);
  await git(created.path, ["commit", "-m", "test worktree changes"]);
  const removed = await workspace.removeOwnedWorktree({
    worktree_id: "WORKTREE-0001",
  });
  assert.equal(removed.removed, true);
  assert.equal(await exists(created.path), false);
  await assert.rejects(
    workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-9999" }),
    (error) =>
      error instanceof WorkspaceError && error.code === "WORKTREE_NOT_OWNED",
  );
});

test("reports integration conflicts without changing the target checkout", async (t) => {
  const fixture = await repositoryFixture(t);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });
  const featureWorktree = await workspace.createWorktree({
    task_id: "TASK-2026-0002",
    worktree_id: "WORKTREE-0002",
    base_commit_sha: fixture.baseCommit,
  });

  await writeFile(path.join(featureWorktree.path, "shared.txt"), "feature\n");
  await git(featureWorktree.path, ["add", "shared.txt"]);
  await git(featureWorktree.path, ["commit", "-m", "feature change"]);
  const featureCommit = await git(featureWorktree.path, ["rev-parse", "HEAD"]);

  await writeFile(path.join(fixture.repositoryRoot, "shared.txt"), "target\n");
  await git(fixture.repositoryRoot, ["add", "shared.txt"]);
  await git(fixture.repositoryRoot, ["commit", "-m", "target change"]);
  const targetCommit = await git(fixture.repositoryRoot, ["rev-parse", "HEAD"]);
  const targetContentBefore = await readFile(
    path.join(fixture.repositoryRoot, "shared.txt"),
    "utf8",
  );

  const result = await workspace.verifyIntegration({
    integration_id: "VERIFY-0001",
    target_commit_sha: targetCommit,
    feature_commit_sha: featureCommit,
  });

  assert.equal(result.can_integrate, false);
  assert.equal(result.outcome, "conflict");
  assert.deepEqual(result.conflicts, ["shared.txt"]);
  assert.equal(
    await git(fixture.repositoryRoot, ["rev-parse", "HEAD"]),
    targetCommit,
  );
  assert.equal(
    await readFile(path.join(fixture.repositoryRoot, "shared.txt"), "utf8"),
    targetContentBefore,
  );
  assert.equal(
    await git(fixture.repositoryRoot, ["status", "--porcelain=v1"]),
    "",
  );
  assert.equal(
    await exists(path.join(fixture.worktreeRoot, "integration-verify-0001")),
    false,
  );

  await workspace.removeOwnedWorktree({
    worktree_id: "WORKTREE-0002",
  });
});

test("reports a clean integration candidate and restores its verification worktree", async (t) => {
  const fixture = await repositoryFixture(t);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });
  const featureWorktree = await workspace.createWorktree({
    task_id: "TASK-2026-0003",
    worktree_id: "WORKTREE-0003",
    base_commit_sha: fixture.baseCommit,
  });

  await appendFile(path.join(featureWorktree.path, "tracked.txt"), "feature\n");
  await git(featureWorktree.path, ["add", "tracked.txt"]);
  await git(featureWorktree.path, ["commit", "-m", "non-conflicting feature"]);
  const featureCommit = await git(featureWorktree.path, ["rev-parse", "HEAD"]);

  await appendFile(path.join(fixture.repositoryRoot, "shared.txt"), "target\n");
  await git(fixture.repositoryRoot, ["add", "shared.txt"]);
  await git(fixture.repositoryRoot, ["commit", "-m", "non-conflicting target"]);
  const targetCommit = await git(fixture.repositoryRoot, ["rev-parse", "HEAD"]);

  const result = await workspace.verifyIntegration({
    integration_id: "VERIFY-0002",
    target_commit_sha: targetCommit,
    feature_commit_sha: featureCommit,
  });

  assert.equal(result.can_integrate, true);
  assert.equal(result.outcome, "clean");
  assert.deepEqual(result.conflicts, []);
  assert.equal(
    result.changes.some((change) => change.path === "tracked.txt"),
    true,
  );
  assert.equal(
    await git(fixture.repositoryRoot, ["rev-parse", "HEAD"]),
    targetCommit,
  );
  assert.equal(
    await git(fixture.repositoryRoot, ["status", "--porcelain=v1"]),
    "",
  );
  assert.equal(
    await exists(path.join(fixture.worktreeRoot, "integration-verify-0002")),
    false,
  );

  await workspace.removeOwnedWorktree({
    worktree_id: "WORKTREE-0003",
  });
});

test("rejects a tampered ownership marker instead of removing another worktree", async (t) => {
  const fixture = await repositoryFixture(t);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });
  const first = await workspace.createWorktree({
    task_id: "TASK-2026-0004",
    worktree_id: "WORKTREE-0004",
    base_commit_sha: fixture.baseCommit,
  });
  const second = await workspace.createWorktree({
    task_id: "TASK-2026-0005",
    worktree_id: "WORKTREE-0005",
    base_commit_sha: fixture.baseCommit,
  });
  const firstMarkerPath = path.join(
    fixture.worktreeRoot,
    ".lattice-ownership",
    "worktree-0004.json",
  );
  const firstMarker = JSON.parse(await readFile(firstMarkerPath, "utf8"));
  await writeFile(
    firstMarkerPath,
    `${JSON.stringify({
      ...firstMarker,
      worktree_path: second.path,
    })}\n`,
  );

  await assert.rejects(
    workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0004" }),
    (error) =>
      error instanceof WorkspaceError && error.code === "WORKTREE_NOT_OWNED",
  );
  assert.equal(await exists(first.path), true);
  assert.equal(await exists(second.path), true);

  await writeFile(firstMarkerPath, `${JSON.stringify(firstMarker)}\n`);
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0004" });
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0005" });
});

test("rejects changed-file inspection through an intermediate junction", async (t) => {
  const fixture = await repositoryFixture(t);
  const foreignParent = path.join(fixture.root, "foreign-parent");
  const foreignRepository = path.join(foreignParent, "foreign-repository");
  await mkdir(foreignRepository, { recursive: true });
  await git(foreignRepository, ["init", "-b", "main"]);
  await git(foreignRepository, ["config", "user.name", "Foreign Test"]);
  await git(foreignRepository, [
    "config",
    "user.email",
    "foreign-test@invalid.example",
  ]);
  await writeFile(path.join(foreignRepository, "foreign.txt"), "base\n");
  await git(foreignRepository, ["add", "."]);
  await git(foreignRepository, ["commit", "-m", "foreign initial"]);
  const foreignBase = await git(foreignRepository, ["rev-parse", "HEAD"]);
  await appendFile(path.join(foreignRepository, "foreign.txt"), "changed\n");

  const junction = path.join(fixture.worktreeRoot, "junction-escape");
  await symlink(
    foreignParent,
    junction,
    process.platform === "win32" ? "junction" : "dir",
  );
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });

  await assert.rejects(
    workspace.changedFiles({
      worktree_id: "WORKTREE-ESCAPE",
      worktreePath: path.join(junction, "foreign-repository"),
      base_commit_sha: foreignBase,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "WORKTREE_NOT_OWNED",
  );
});

test("rejects a junctioned worktree root before creating external ownership data", async (t) => {
  const fixture = await repositoryFixture(t);
  const unrelatedRoot = path.join(fixture.root, "unrelated-worktrees");
  const junctionRoot = path.join(fixture.root, "junction-worktrees");
  await mkdir(unrelatedRoot);
  await symlink(
    unrelatedRoot,
    junctionRoot,
    process.platform === "win32" ? "junction" : "dir",
  );
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: junctionRoot,
  });

  await assert.rejects(
    workspace.createWorktree({
      task_id: "TASK-2026-0006",
      worktree_id: "WORKTREE-0006",
      base_commit_sha: fixture.baseCommit,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "WORKTREE_PATH_ESCAPE",
  );
  assert.equal(
    await exists(path.join(unrelatedRoot, ".lattice-ownership")),
    false,
  );
});

test("treats only exact ProjectLock metadata as non-product repository state", async (t) => {
  const fixture = await repositoryFixture(t);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });
  const lock = new ProjectLock({
    projectRoot: fixture.repositoryRoot,
    idFactory: () => "LEASE-CROSS-CONTRACT",
  });
  const lease = await lock.acquire({
    project_id: "lattice-devos",
    task_id: "TASK-2026-0007",
    task_revision: 1,
    spec_hash: "b".repeat(64),
    attempt_id: "ATTEMPT-0007",
    worktree_id: "WORKTREE-0007",
    role: "IMPLEMENTER",
  });

  const activeLockWorktree = await workspace.createWorktree({
    task_id: "TASK-2026-0007",
    worktree_id: "WORKTREE-0007",
    base_commit_sha: fixture.baseCommit,
  });
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0007" });
  await lock.release({
    lease_id: lease.lease_id,
    fencing_token: lease.fencing_token,
  });

  const releasedLockWorktree = await workspace.createWorktree({
    task_id: "TASK-2026-0008",
    worktree_id: "WORKTREE-0008",
    base_commit_sha: fixture.baseCommit,
  });
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0008" });

  await writeFile(
    path.join(fixture.repositoryRoot, ".lattice", "unowned.txt"),
    "must remain visible\n",
  );
  await assert.rejects(
    workspace.createWorktree({
      task_id: "TASK-2026-0009",
      worktree_id: "WORKTREE-0009",
      base_commit_sha: fixture.baseCommit,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "REPOSITORY_DIRTY",
  );
});

test("disables repository hooks and accepts only a clean created worktree", async (t) => {
  const fixture = await repositoryFixture(t);
  const gitDirectory = path.join(fixture.repositoryRoot, ".git");
  const postCheckoutHook = path.join(gitDirectory, "hooks", "post-checkout");
  await writeFile(
    postCheckoutHook,
    "#!/bin/sh\nprintf 'hook ran\\n' > hook-created.txt\n",
  );
  await chmod(postCheckoutHook, 0o755);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });

  const created = await workspace.createWorktree({
    task_id: "TASK-2026-0011",
    worktree_id: "WORKTREE-0011",
    base_commit_sha: fixture.baseCommit,
  });

  assert.equal(await exists(path.join(created.path, "hook-created.txt")), false);
  assert.equal(await git(created.path, ["status", "--porcelain=v1"]), "");
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0011" });
});

test("rejects empty, whitespace, or NUL workspace roots before path resolution", () => {
  for (const [repositoryRoot, worktreeRoot] of [
    ["", "C:\\safe-worktrees"],
    ["   ", "C:\\safe-worktrees"],
    ["C:\\safe-repository", ""],
    ["C:\\safe-repository", "   "],
    ["C:\\safe\u0000repository", "C:\\safe-worktrees"],
  ]) {
    assert.throws(
      () => new GitWorkspace({ repositoryRoot, worktreeRoot }),
      (error) =>
        error instanceof WorkspaceError &&
        error.code === "INVALID_GIT_WORKSPACE",
    );
  }
});

test("cleans an owned integration worktree when conflict-evidence collection fails", async (t) => {
  const fixture = await repositoryFixture(t);
  let injectEvidenceFailure = true;
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
    executor: async (request) => {
      if (
        injectEvidenceFailure &&
        request.args.includes("--diff-filter=U")
      ) {
        injectEvidenceFailure = false;
        return {
          exit_code: 91,
          stdout: "",
          stderr: "injected conflict-evidence failure",
        };
      }
      return defaultGitExecutor(request);
    },
  });
  const featureWorktree = await workspace.createWorktree({
    task_id: "TASK-2026-0012",
    worktree_id: "WORKTREE-0012",
    base_commit_sha: fixture.baseCommit,
  });
  await writeFile(path.join(featureWorktree.path, "shared.txt"), "feature\n");
  await git(featureWorktree.path, ["add", "shared.txt"]);
  await git(featureWorktree.path, ["commit", "-m", "feature conflict"]);
  const featureCommit = await git(featureWorktree.path, ["rev-parse", "HEAD"]);
  await writeFile(path.join(fixture.repositoryRoot, "shared.txt"), "target\n");
  await git(fixture.repositoryRoot, ["add", "shared.txt"]);
  await git(fixture.repositoryRoot, ["commit", "-m", "target conflict"]);
  const targetCommit = await git(fixture.repositoryRoot, ["rev-parse", "HEAD"]);

  await assert.rejects(
    workspace.verifyIntegration({
      integration_id: "VERIFY-0099",
      target_commit_sha: targetCommit,
      feature_commit_sha: featureCommit,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "GIT_COMMAND_FAILED",
  );
  assert.equal(
    await exists(path.join(fixture.worktreeRoot, "integration-verify-0099")),
    false,
  );
  assert.equal(
    await exists(
      path.join(
        fixture.worktreeRoot,
        ".lattice-ownership",
        "integration-verify-0099.json",
      ),
    ),
    false,
  );
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0012" });
});

test("rejects external Git filter or merge drivers before creating a worktree", async (t) => {
  const fixture = await repositoryFixture(t);
  await git(fixture.repositoryRoot, [
    "config",
    "filter.danger.smudge",
    "dangerous-external-command",
  ]);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });

  await assert.rejects(
    workspace.createWorktree({
      task_id: "TASK-2026-0013",
      worktree_id: "WORKTREE-0013",
      base_commit_sha: fixture.baseCommit,
    }),
    (error) =>
      error instanceof WorkspaceError &&
      error.code === "GIT_EXECUTION_CONFIG_UNSAFE",
  );
  assert.equal(
    await exists(path.join(fixture.worktreeRoot, "worktree-0013")),
    false,
  );
});
