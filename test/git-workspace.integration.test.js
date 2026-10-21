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
  unlink,
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

function shellQuotedPath(target) {
  return `"${target.replaceAll("\\", "/").replaceAll('"', '\\"')}"`;
}

async function markerFilter(fixture, name = "danger") {
  const scriptPath = path.join(fixture.root, `${name}-filter.cjs`);
  const markerPath = path.join(fixture.root, `${name}-executed.txt`);
  await writeFile(
    scriptPath,
    [
      'const fs = require("node:fs");',
      'fs.writeFileSync(process.argv[2], "executed\\n");',
      "process.stdin.pipe(process.stdout);",
      "",
    ].join("\n"),
  );
  return {
    command: [
      shellQuotedPath(process.execPath),
      shellQuotedPath(scriptPath),
      shellQuotedPath(markerPath),
    ].join(" "),
    markerPath,
  };
}

async function installMarkerFilter(fixture, name = "danger") {
  const filter = await markerFilter(fixture, name);
  await git(fixture.repositoryRoot, [
    "config",
    `filter.${name}.clean`,
    filter.command,
  ]);
  return filter;
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
  const stagedFileEvidence = changes.filter(
    (entry) => entry.path === "staged.txt",
  );
  assert.equal(
    stagedFileEvidence.some(
      (entry) =>
        entry.operation === "create" && entry.states[0] === "staged",
    ),
    true,
  );
  assert.equal(
    stagedFileEvidence.some(
      (entry) =>
        entry.operation === "modify" && entry.states[0] === "unstaged",
    ),
    true,
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
    (error) => {
      if (!(error instanceof WorkspaceError) || error.code !== "WORKTREE_DIRTY") {
        return false;
      }
      assert.equal(error.details.status.includes("tracked.txt"), true);
      return true;
    },
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

test("rejects an active clean filter before repository status can execute it", async (t) => {
  const fixture = await repositoryFixture(t);
  await writeFile(
    path.join(fixture.repositoryRoot, ".gitattributes"),
    "tracked.txt filter=preflight\n",
  );
  await git(fixture.repositoryRoot, ["add", ".gitattributes"]);
  await git(fixture.repositoryRoot, ["commit", "-m", "activate test filter"]);
  const filteredBase = await git(fixture.repositoryRoot, ["rev-parse", "HEAD"]);
  const { markerPath } = await installMarkerFilter(fixture, "preflight");
  await writeFile(
    path.join(fixture.repositoryRoot, "tracked.txt"),
    "force status conversion\n",
  );
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });

  await assert.rejects(
    workspace.createWorktree({
      task_id: "TASK-2026-0014",
      worktree_id: "WORKTREE-0014",
      base_commit_sha: filteredBase,
    }),
    (error) =>
      error instanceof WorkspaceError &&
      error.code === "GIT_EXECUTION_CONFIG_UNSAFE",
  );
  assert.equal(await exists(markerPath), false);
  assert.equal(
    await exists(path.join(fixture.worktreeRoot, "worktree-0014")),
    false,
  );
});

test("isolates Git commands from global external filters and attributes", async (t) => {
  const fixture = await repositoryFixture(t);
  await writeFile(
    path.join(fixture.repositoryRoot, ".gitattributes"),
    "tracked.txt filter=globaldanger\n",
  );
  await git(fixture.repositoryRoot, ["add", ".gitattributes"]);
  await git(fixture.repositoryRoot, ["commit", "-m", "activate global test filter"]);
  const filteredBase = await git(fixture.repositoryRoot, ["rev-parse", "HEAD"]);
  const filter = await markerFilter(fixture, "globaldanger");
  const globalConfigPath = path.join(fixture.root, "global.gitconfig");
  await git(fixture.repositoryRoot, [
    "config",
    "--file",
    globalConfigPath,
    "filter.globaldanger.clean",
    filter.command,
  ]);
  await git(fixture.repositoryRoot, [
    "config",
    "--file",
    globalConfigPath,
    "filter.globaldanger.smudge",
    filter.command,
  ]);
  const originalGlobalConfig = process.env.GIT_CONFIG_GLOBAL;
  process.env.GIT_CONFIG_GLOBAL = globalConfigPath;
  t.after(() => {
    if (originalGlobalConfig === undefined) {
      delete process.env.GIT_CONFIG_GLOBAL;
    } else {
      process.env.GIT_CONFIG_GLOBAL = originalGlobalConfig;
    }
  });
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });

  const created = await workspace.createWorktree({
    task_id: "TASK-2026-0024",
    worktree_id: "WORKTREE-0024",
    base_commit_sha: filteredBase,
  });
  assert.equal(await exists(filter.markerPath), false);
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0024" });
});

test("rejects repository-local include files before an included filter can execute", async (t) => {
  const fixture = await repositoryFixture(t);
  await writeFile(
    path.join(fixture.repositoryRoot, ".gitattributes"),
    "tracked.txt filter=included\n",
  );
  await git(fixture.repositoryRoot, ["add", ".gitattributes"]);
  await git(fixture.repositoryRoot, ["commit", "-m", "activate included filter"]);
  const filteredBase = await git(fixture.repositoryRoot, ["rev-parse", "HEAD"]);
  const filter = await markerFilter(fixture, "included");
  const includedConfigPath = path.join(fixture.root, "included.gitconfig");
  await git(fixture.repositoryRoot, [
    "config",
    "--file",
    includedConfigPath,
    "filter.included.clean",
    filter.command,
  ]);
  await git(fixture.repositoryRoot, [
    "config",
    "--local",
    "include.path",
    includedConfigPath,
  ]);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });

  await assert.rejects(
    workspace.createWorktree({
      task_id: "TASK-2026-0025",
      worktree_id: "WORKTREE-0025",
      base_commit_sha: filteredBase,
    }),
    (error) =>
      error instanceof WorkspaceError &&
      error.code === "GIT_EXECUTION_CONFIG_UNSAFE",
  );
  assert.equal(await exists(filter.markerPath), false);
  assert.equal(
    await exists(path.join(fixture.worktreeRoot, "worktree-0025")),
    false,
  );
});

test("rejects branch-conditional includes before task worktree checkout", async (t) => {
  const fixture = await repositoryFixture(t);
  await writeFile(
    path.join(fixture.repositoryRoot, ".gitattributes"),
    "tracked.txt filter=conditional\n",
  );
  await git(fixture.repositoryRoot, ["add", ".gitattributes"]);
  await git(fixture.repositoryRoot, [
    "commit",
    "-m",
    "activate conditional filter",
  ]);
  const filteredBase = await git(fixture.repositoryRoot, ["rev-parse", "HEAD"]);
  const filter = await markerFilter(fixture, "conditional");
  const includedConfigPath = path.join(fixture.root, "conditional.gitconfig");
  await git(fixture.repositoryRoot, [
    "config",
    "--file",
    includedConfigPath,
    "filter.conditional.smudge",
    filter.command,
  ]);
  await git(fixture.repositoryRoot, [
    "config",
    "--local",
    "includeIf.onbranch:lattice/task-2026-0026.path",
    includedConfigPath,
  ]);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });

  await assert.rejects(
    workspace.createWorktree({
      task_id: "TASK-2026-0026",
      worktree_id: "WORKTREE-0026",
      base_commit_sha: filteredBase,
    }),
    (error) =>
      error instanceof WorkspaceError &&
      error.code === "GIT_EXECUTION_CONFIG_UNSAFE",
  );
  assert.equal(await exists(filter.markerPath), false);
  assert.equal(
    await exists(path.join(fixture.worktreeRoot, "worktree-0026")),
    false,
  );
});

test("isolates Git commands from command-scope config injection", async (t) => {
  const fixture = await repositoryFixture(t);
  await writeFile(
    path.join(fixture.repositoryRoot, ".gitattributes"),
    "tracked.txt filter=envdanger\n",
  );
  await git(fixture.repositoryRoot, ["add", ".gitattributes"]);
  await git(fixture.repositoryRoot, ["commit", "-m", "activate env test filter"]);
  const filteredBase = await git(fixture.repositoryRoot, ["rev-parse", "HEAD"]);
  const filter = await markerFilter(fixture, "envdanger");
  const originalEnvironment = new Map(
    ["GIT_CONFIG_COUNT", "GIT_CONFIG_KEY_0", "GIT_CONFIG_VALUE_0"].map(
      (key) => [key, process.env[key]],
    ),
  );
  Object.assign(process.env, {
    GIT_CONFIG_COUNT: "1",
    GIT_CONFIG_KEY_0: "filter.envdanger.smudge",
    GIT_CONFIG_VALUE_0: filter.command,
  });
  t.after(() => {
    for (const [key, value] of originalEnvironment) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  });
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });

  const created = await workspace.createWorktree({
    task_id: "TASK-2026-0027",
    worktree_id: "WORKTREE-0027",
    base_commit_sha: filteredBase,
  });
  assert.equal(await exists(filter.markerPath), false);
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0027" });
});

test("rejects worktree-scoped filters added after ownership is established", async (t) => {
  const fixture = await repositoryFixture(t);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });
  const created = await workspace.createWorktree({
    task_id: "TASK-2026-0028",
    worktree_id: "WORKTREE-0028",
    base_commit_sha: fixture.baseCommit,
  });
  const filter = await markerFilter(fixture, "worktree");
  await git(fixture.repositoryRoot, [
    "config",
    "--local",
    "extensions.worktreeConfig",
    "true",
  ]);
  await git(created.path, [
    "config",
    "--worktree",
    "filter.worktree.clean",
    filter.command,
  ]);
  await writeFile(
    path.join(created.path, ".gitattributes"),
    "tracked.txt filter=worktree\n",
  );
  await writeFile(path.join(created.path, "tracked.txt"), "worktree filter probe\n");

  await assert.rejects(
    workspace.changedFiles({
      worktree_id: "WORKTREE-0028",
      worktreePath: created.path,
      base_commit_sha: fixture.baseCommit,
    }),
    (error) =>
      error instanceof WorkspaceError &&
      error.code === "GIT_EXECUTION_CONFIG_UNSAFE",
  );
  assert.equal(await exists(filter.markerPath), false);

  await git(created.path, [
    "config",
    "--worktree",
    "--unset",
    "filter.worktree.clean",
  ]);
  await rm(path.join(created.path, ".gitattributes"));
  await writeFile(path.join(created.path, "tracked.txt"), "base\n");
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0028" });
});

test("rejects a late-added clean filter before changed-file inspection executes it", async (t) => {
  const fixture = await repositoryFixture(t);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });
  const created = await workspace.createWorktree({
    task_id: "TASK-2026-0022",
    worktree_id: "WORKTREE-0022",
    base_commit_sha: fixture.baseCommit,
  });
  const { markerPath } = await installMarkerFilter(fixture, "late");
  await writeFile(
    path.join(created.path, ".gitattributes"),
    "tracked.txt filter=late\n",
  );
  await writeFile(path.join(created.path, "tracked.txt"), "late filter probe\n");

  await assert.rejects(
    workspace.changedFiles({
      worktree_id: "WORKTREE-0022",
      worktreePath: created.path,
      base_commit_sha: fixture.baseCommit,
    }),
    (error) =>
      error instanceof WorkspaceError &&
      error.code === "GIT_EXECUTION_CONFIG_UNSAFE",
  );
  assert.equal(await exists(markerPath), false);

  await git(fixture.repositoryRoot, ["config", "--unset", "filter.late.clean"]);
  await rm(path.join(created.path, ".gitattributes"));
  await writeFile(path.join(created.path, "tracked.txt"), "base\n");
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0022" });
});

test("reports ignored files as scope evidence and refuses to remove them", async (t) => {
  const fixture = await repositoryFixture(t);
  await writeFile(path.join(fixture.repositoryRoot, ".gitignore"), "ignored/\n");
  await git(fixture.repositoryRoot, ["add", ".gitignore"]);
  await git(fixture.repositoryRoot, ["commit", "-m", "ignore generated output"]);
  const ignoredBase = await git(fixture.repositoryRoot, ["rev-parse", "HEAD"]);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });
  const created = await workspace.createWorktree({
    task_id: "TASK-2026-0023",
    worktree_id: "WORKTREE-0023",
    base_commit_sha: ignoredBase,
  });
  const ignoredDirectory = path.join(created.path, "ignored");
  await mkdir(ignoredDirectory);
  await writeFile(path.join(ignoredDirectory, "outside.txt"), "ignored write\n");

  const changes = await workspace.changedFiles({
    worktree_id: "WORKTREE-0023",
    worktreePath: created.path,
    base_commit_sha: ignoredBase,
  });
  const ignoredEvidence = changes.find(
    (change) => change.path === "ignored/outside.txt",
  );
  assert.deepEqual(ignoredEvidence?.states, ["ignored"]);
  assert.equal(ignoredEvidence?.git_status, "!!");

  await assert.rejects(
    workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0023" }),
    (error) => {
      if (!(error instanceof WorkspaceError) || error.code !== "WORKTREE_DIRTY") {
        return false;
      }
      assert.deepEqual(error.details.ignored, ["ignored/outside.txt"]);
      return true;
    },
  );
  assert.equal(await exists(created.path), true);

  await rm(ignoredDirectory, { recursive: true });
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0023" });
});

test("reports Git symlink mode and untracked junction evidence as links", async (t) => {
  const fixture = await repositoryFixture(t);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });
  const created = await workspace.createWorktree({
    task_id: "TASK-2026-0015",
    worktree_id: "WORKTREE-0015",
    base_commit_sha: fixture.baseCommit,
  });
  const trackedLinkPath = path.join(created.path, "tracked-link");
  await writeFile(trackedLinkPath, "tracked.txt");
  const linkBlob = await git(created.path, [
    "hash-object",
    "-w",
    "tracked-link",
  ]);
  await git(created.path, [
    "update-index",
    "--add",
    "--cacheinfo",
    `120000,${linkBlob},tracked-link`,
  ]);

  const outsideDirectory = path.join(fixture.root, "outside-junction");
  await mkdir(outsideDirectory);
  await writeFile(path.join(outsideDirectory, "outside.txt"), "outside\n");
  const junctionPath = path.join(created.path, "untracked-junction");
  await symlink(
    outsideDirectory,
    junctionPath,
    process.platform === "win32" ? "junction" : "dir",
  );

  const changes = await workspace.changedFiles({
    worktree_id: "WORKTREE-0015",
    worktreePath: created.path,
    base_commit_sha: fixture.baseCommit,
  });
  const trackedLink = changes.find(
    (change) => change.path === "tracked-link",
  );
  assert.equal(trackedLink.kind, "link");
  assert.equal(trackedLink.old_mode, "000000");
  assert.equal(trackedLink.new_mode, "120000");
  const junctionEvidence = changes.filter((change) =>
    change.path.startsWith("untracked-junction"),
  );
  assert.equal(junctionEvidence.length > 0, true);
  assert.equal(
    junctionEvidence.every((change) => change.kind === "link"),
    true,
  );

  await unlink(junctionPath);
  await git(created.path, ["commit", "-m", "link evidence fixture"]);
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0015" });
});

test("reports a staged change even when the working tree is restored to base", async (t) => {
  const fixture = await repositoryFixture(t);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });
  const created = await workspace.createWorktree({
    task_id: "TASK-2026-0016",
    worktree_id: "WORKTREE-0016",
    base_commit_sha: fixture.baseCommit,
  });
  await writeFile(path.join(created.path, "tracked.txt"), "staged-sensitive\n");
  await git(created.path, ["add", "tracked.txt"]);
  await writeFile(path.join(created.path, "tracked.txt"), "base\n");

  const changes = await workspace.changedFiles({
    worktree_id: "WORKTREE-0016",
    worktreePath: created.path,
    base_commit_sha: fixture.baseCommit,
  });
  const trackedEvidence = changes.filter(
    (change) => change.path === "tracked.txt",
  );
  assert.equal(trackedEvidence.length > 0, true);
  assert.equal(
    trackedEvidence.some((change) => change.states.includes("staged")),
    true,
  );
  assert.equal(
    trackedEvidence.some((change) => change.states.includes("unstaged")),
    true,
  );

  await git(created.path, ["add", "tracked.txt"]);
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0016" });
});

test("recovers an owned task worktree when post-create identity inspection fails", async (t) => {
  const fixture = await repositoryFixture(t);
  let injectHeadFailure = true;
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
    executor: async (request) => {
      if (
        injectHeadFailure &&
        request.args.includes("rev-parse") &&
        request.args.at(-1) === "HEAD" &&
        request.cwd.includes("worktree-0017")
      ) {
        injectHeadFailure = false;
        return {
          exit_code: 92,
          stdout: "",
          stderr: "injected post-create HEAD failure",
        };
      }
      return defaultGitExecutor(request);
    },
  });

  await assert.rejects(
    workspace.createWorktree({
      task_id: "TASK-2026-0017",
      worktree_id: "WORKTREE-0017",
      base_commit_sha: fixture.baseCommit,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "GIT_COMMAND_FAILED",
  );
  assert.equal(
    await exists(path.join(fixture.worktreeRoot, "worktree-0017")),
    false,
  );
  assert.equal(
    await exists(
      path.join(
        fixture.worktreeRoot,
        ".lattice-ownership",
        "worktree-0017.json",
      ),
    ),
    false,
  );
  assert.equal(
    (await git(fixture.repositoryRoot, ["worktree", "list", "--porcelain"]))
      .includes("worktree-0017"),
    false,
  );
  await assert.rejects(
    git(fixture.repositoryRoot, [
      "show-ref",
      "--verify",
      "--quiet",
      "refs/heads/lattice/task-2026-0017",
    ]),
  );
  const retried = await workspace.createWorktree({
    task_id: "TASK-2026-0017",
    worktree_id: "WORKTREE-0017-RETRY",
    base_commit_sha: fixture.baseCommit,
  });
  assert.equal(await exists(retried.path), true);
  await workspace.removeOwnedWorktree({
    worktree_id: "WORKTREE-0017-RETRY",
  });
});

test("recovers an integration worktree when post-create identity inspection fails", async (t) => {
  const fixture = await repositoryFixture(t);
  let injectHeadFailure = false;
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
    executor: async (request) => {
      if (
        injectHeadFailure &&
        request.args.includes("rev-parse") &&
        request.args.at(-1) === "HEAD" &&
        request.cwd.includes("integration-verify-0100")
      ) {
        injectHeadFailure = false;
        return {
          exit_code: 93,
          stdout: "",
          stderr: "injected integration HEAD failure",
        };
      }
      return defaultGitExecutor(request);
    },
  });
  const featureWorktree = await workspace.createWorktree({
    task_id: "TASK-2026-0018",
    worktree_id: "WORKTREE-0018",
    base_commit_sha: fixture.baseCommit,
  });
  await appendFile(path.join(featureWorktree.path, "tracked.txt"), "feature\n");
  await git(featureWorktree.path, ["add", "tracked.txt"]);
  await git(featureWorktree.path, ["commit", "-m", "feature for recovery"]);
  const featureCommit = await git(featureWorktree.path, ["rev-parse", "HEAD"]);
  injectHeadFailure = true;

  await assert.rejects(
    workspace.verifyIntegration({
      integration_id: "VERIFY-0100",
      target_commit_sha: fixture.baseCommit,
      feature_commit_sha: featureCommit,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "GIT_COMMAND_FAILED",
  );
  assert.equal(
    await exists(path.join(fixture.worktreeRoot, "integration-verify-0100")),
    false,
  );
  assert.equal(
    await exists(
      path.join(
        fixture.worktreeRoot,
        ".lattice-ownership",
        "integration-verify-0100.json",
      ),
    ),
    false,
  );
  await workspace.removeOwnedWorktree({ worktree_id: "WORKTREE-0018" });
});

test("invalid method inputs fail before creating workspace control directories", async (t) => {
  const fixture = await repositoryFixture(t);
  const createRoot = path.join(fixture.root, "invalid-create-root");
  const createWorkspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: createRoot,
  });
  await assert.rejects(
    createWorkspace.createWorktree({
      task_id: "TASK-2026-0019",
      worktree_id: "bad",
      base_commit_sha: fixture.baseCommit,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "INVALID_WORKTREE_ID",
  );
  assert.equal(await exists(createRoot), false);

  const integrationRoot = path.join(fixture.root, "invalid-integration-root");
  const integrationWorkspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: integrationRoot,
  });
  await assert.rejects(
    integrationWorkspace.verifyIntegration({
      integration_id: "bad",
      target_commit_sha: fixture.baseCommit,
      feature_commit_sha: fixture.baseCommit,
    }),
    (error) =>
      error instanceof WorkspaceError &&
      error.code === "INVALID_INTEGRATION_ID",
  );
  assert.equal(await exists(integrationRoot), false);

  const inspectionRoot = path.join(fixture.root, "invalid-inspection-root");
  const inspectionWorkspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: inspectionRoot,
  });
  await assert.rejects(
    inspectionWorkspace.changedFiles({
      worktree_id: "WORKTREE-0020",
      worktreePath: "",
      base_commit_sha: fixture.baseCommit,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "WORKTREE_NOT_OWNED",
  );
  assert.equal(await exists(inspectionRoot), false);
});

test("failed creation never deletes a pre-existing task branch", async (t) => {
  const fixture = await repositoryFixture(t);
  const branch = "lattice/task-2026-0021";
  await git(fixture.repositoryRoot, ["branch", branch, fixture.baseCommit]);
  const workspace = new GitWorkspace({
    repositoryRoot: fixture.repositoryRoot,
    worktreeRoot: fixture.worktreeRoot,
  });

  await assert.rejects(
    workspace.createWorktree({
      task_id: "TASK-2026-0021",
      worktree_id: "WORKTREE-0021",
      base_commit_sha: fixture.baseCommit,
    }),
    (error) =>
      error instanceof WorkspaceError &&
      error.code === "WORKTREE_BRANCH_EXISTS",
  );
  assert.equal(
    await git(fixture.repositoryRoot, [
      "rev-parse",
      "refs/heads/lattice/task-2026-0021",
    ]),
    fixture.baseCommit,
  );
});
