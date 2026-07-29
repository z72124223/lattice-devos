import { execFile } from "node:child_process";
import {
  lstat,
  mkdir,
  open,
  readdir,
  readFile,
  realpath,
  unlink,
} from "node:fs/promises";
import path from "node:path";

import { deepFreeze } from "../domain/canonical-json.js";
import { WorkspaceError, workspaceFailure } from "./errors.js";

const HASH_PATTERN = /^[a-f0-9]{40,64}$/;
const TASK_ID_PATTERN = /^TASK-[A-Z0-9][A-Z0-9_-]{2,63}$/;
const WORKTREE_ID_PATTERN = /^[A-Z0-9][A-Z0-9_-]{2,63}$/;
const INTEGRATION_ID_PATTERN = /^[A-Z0-9][A-Z0-9_-]{2,63}$/;
const OWNED_CONTROL_PATHS = new Set([
  ".lattice/locks/fencing-initialized",
  ".lattice/locks/fencing-token",
  ".lattice/locks/project.lock",
]);

function normalizedPath(value) {
  const resolved = path.resolve(value);
  return process.platform === "win32" ? resolved.toLowerCase() : resolved;
}

function samePath(left, right) {
  return normalizedPath(left) === normalizedPath(right);
}

function containedBy(parent, candidate) {
  const relative = path.relative(parent, candidate);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

async function optionalLstat(file) {
  try {
    return await lstat(file);
  } catch (error) {
    if (error?.code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

async function canonicalDirectory(directory, code, message) {
  const stat = await optionalLstat(directory);
  if (!stat) {
    return null;
  }
  if (!stat.isDirectory() || stat.isSymbolicLink()) {
    workspaceFailure(code, message);
  }
  const actual = await realpath(directory);
  if (!samePath(actual, directory)) {
    workspaceFailure(code, message);
  }
  return actual;
}

async function ensureCanonicalDirectory(directory, code, message) {
  const target = path.resolve(directory);
  const missing = [];
  let cursor = target;
  let existing = await canonicalDirectory(cursor, code, message);
  while (existing === null) {
    missing.push(cursor);
    const parent = path.dirname(cursor);
    if (samePath(parent, cursor)) {
      workspaceFailure(code, message);
    }
    cursor = parent;
    existing = await canonicalDirectory(cursor, code, message);
  }
  for (const candidate of missing.reverse()) {
    try {
      await mkdir(candidate);
    } catch (error) {
      if (error?.code !== "EEXIST") {
        throw error;
      }
    }
    await canonicalDirectory(candidate, code, message);
  }
  return realpath(target);
}

function validateExecutorRequest({ command, args, cwd }) {
  if (
    command !== "git" ||
    !Array.isArray(args) ||
    args.some((argument) => typeof argument !== "string" || argument.includes("\0")) ||
    typeof cwd !== "string" ||
    cwd.includes("\0")
  ) {
    workspaceFailure(
      "INVALID_GIT_COMMAND",
      "Git execution requires command='git', argument array, and explicit cwd.",
    );
  }
}

export async function defaultGitExecutor(request) {
  validateExecutorRequest(request);
  return new Promise((resolve) => {
    execFile(
      request.command,
      request.args,
      {
        cwd: request.cwd,
        encoding: "utf8",
        windowsHide: true,
        maxBuffer: 4 * 1024 * 1024,
      },
      (error, stdout, stderr) => {
        resolve({
          exit_code:
            error === null
              ? 0
              : Number.isInteger(error.code)
                ? error.code
                : 1,
          stdout: stdout ?? "",
          stderr: stderr ?? error?.message ?? "",
        });
      },
    );
  });
}

async function durableCreate(file, content) {
  const handle = await open(file, "wx");
  try {
    await handle.writeFile(content, "utf8");
    await handle.sync();
  } finally {
    await handle.close();
  }
}

function safeTaskBranch(taskId) {
  if (!TASK_ID_PATTERN.test(taskId)) {
    workspaceFailure("INVALID_TASK_ID", "Task ID cannot form a Git branch.");
  }
  return `lattice/${taskId.toLowerCase()}`;
}

function safeWorktreeId(worktreeId) {
  if (!WORKTREE_ID_PATTERN.test(worktreeId)) {
    workspaceFailure("INVALID_WORKTREE_ID", "worktree_id is unsafe.");
  }
  return worktreeId;
}

function safeIntegrationId(integrationId) {
  if (!INTEGRATION_ID_PATTERN.test(integrationId)) {
    workspaceFailure("INVALID_INTEGRATION_ID", "integration_id is unsafe.");
  }
  return integrationId;
}

function safeHash(value, field) {
  if (typeof value !== "string" || !HASH_PATTERN.test(value.toLowerCase())) {
    workspaceFailure("INVALID_GIT_HASH", `${field} must be a Git object hash.`);
  }
  return value.toLowerCase();
}

function parseNameStatus(output) {
  if (output.length === 0) {
    return [];
  }
  const tokens = output.split("\0");
  if (tokens.at(-1) === "") {
    tokens.pop();
  }
  const changes = [];
  let index = 0;
  while (index < tokens.length) {
    let statusToken = tokens[index++];
    let firstPath;
    if (statusToken.includes("\t")) {
      const separator = statusToken.indexOf("\t");
      firstPath = statusToken.slice(separator + 1);
      statusToken = statusToken.slice(0, separator);
    } else {
      firstPath = tokens[index++];
    }
    if (!statusToken || typeof firstPath !== "string") {
      workspaceFailure("GIT_STATUS_PARSE_FAILED", "Malformed Git name-status output.");
    }
    const code = statusToken[0];
    if (code === "R" || code === "C") {
      const destination = tokens[index++];
      if (typeof destination !== "string") {
        workspaceFailure("GIT_STATUS_PARSE_FAILED", "Rename/copy destination is missing.");
      }
      changes.push({
        operation: code === "R" ? "rename" : "create",
        path: destination,
        from_path: firstPath,
        git_status: statusToken,
        kind: "file",
      });
      continue;
    }
    const operation = {
      A: "create",
      M: "modify",
      D: "delete",
      T: "typechange",
      U: "unmerged",
    }[code];
    changes.push({
      operation: operation ?? "unknown",
      path: firstPath,
      from_path: null,
      git_status: statusToken,
      kind: "file",
    });
  }
  return changes;
}

function unownedRepositoryStatus(output) {
  const tokens = output.split("\0").filter(Boolean);
  const dirty = [];
  for (let index = 0; index < tokens.length; index += 1) {
    const record = tokens[index];
    const status = record.slice(0, 2);
    const changedPath = record.slice(3).replaceAll("\\", "/");
    if (status === "??" && OWNED_CONTROL_PATHS.has(changedPath)) {
      continue;
    }
    dirty.push(record);
    if (status[0] === "R" || status[0] === "C") {
      index += 1;
      if (index < tokens.length) {
        dirty.push(tokens[index]);
      }
    }
  }
  return dirty;
}

export class GitWorkspace {
  #repositoryRoot;
  #worktreeRoot;
  #ownershipDirectory;
  #hooksDirectory;
  #executor;

  constructor({
    repositoryRoot,
    worktreeRoot,
    executor = defaultGitExecutor,
  }) {
    if (
      typeof repositoryRoot !== "string" ||
      typeof worktreeRoot !== "string" ||
      repositoryRoot.trim().length === 0 ||
      worktreeRoot.trim().length === 0 ||
      repositoryRoot !== repositoryRoot.trim() ||
      worktreeRoot !== worktreeRoot.trim() ||
      repositoryRoot.includes("\0") ||
      worktreeRoot.includes("\0") ||
      typeof executor !== "function"
    ) {
      workspaceFailure("INVALID_GIT_WORKSPACE", "Git workspace dependencies are invalid.");
    }
    this.#repositoryRoot = path.resolve(repositoryRoot);
    this.#worktreeRoot = path.resolve(worktreeRoot);
    if (
      containedBy(this.#repositoryRoot, this.#worktreeRoot) ||
      containedBy(this.#worktreeRoot, this.#repositoryRoot)
    ) {
      workspaceFailure(
        "WORKTREE_ROOT_OVERLAP",
        "worktreeRoot must be separate from repositoryRoot.",
      );
    }
    this.#ownershipDirectory = path.join(
      this.#worktreeRoot,
      ".lattice-ownership",
    );
    this.#hooksDirectory = path.join(
      this.#worktreeRoot,
      ".lattice-hooks-empty",
    );
    this.#executor = executor;
  }

  async #git(args, cwd = this.#repositoryRoot, { allowFailure = false } = {}) {
    const hooksDirectory = await canonicalDirectory(
      this.#hooksDirectory,
      "WORKTREE_HOOKS_UNSAFE",
      "Git hooks directory is missing or unsafe.",
    );
    if (
      hooksDirectory === null ||
      (await readdir(this.#hooksDirectory)).length > 0
    ) {
      workspaceFailure(
        "WORKTREE_HOOKS_UNSAFE",
        "Git hooks directory must be an empty canonical directory.",
      );
    }
    const protectedArgs = [
      "-c",
      `core.hooksPath=${this.#hooksDirectory}`,
      "-c",
      "core.fsmonitor=false",
      ...args,
    ];
    validateExecutorRequest({ command: "git", args: protectedArgs, cwd });
    const result = await this.#executor({
      command: "git",
      args: [...protectedArgs],
      cwd,
    });
    if (
      result === null ||
      typeof result !== "object" ||
      !Number.isInteger(result.exit_code)
    ) {
      workspaceFailure("GIT_EXECUTOR_INVALID", "Git executor returned an invalid result.");
    }
    if (result.exit_code !== 0 && !allowFailure) {
      workspaceFailure("GIT_COMMAND_FAILED", "Git command failed.", {
        args: protectedArgs,
        cwd,
        exit_code: result.exit_code,
        stderr: result.stderr,
      });
    }
    return result;
  }

  async #ensureRoots() {
    const repositoryReal = await canonicalDirectory(
      this.#repositoryRoot,
      "INVALID_REPOSITORY_ROOT",
      "Repository root is not a canonical real directory.",
    );
    if (repositoryReal === null) {
      workspaceFailure("INVALID_REPOSITORY_ROOT", "Repository root is not a real directory.");
    }
    const worktreeReal = await ensureCanonicalDirectory(
      this.#worktreeRoot,
      "WORKTREE_PATH_ESCAPE",
      "Worktree root or an ancestor traverses a link or junction.",
    );
    if (
      containedBy(repositoryReal, worktreeReal) ||
      containedBy(worktreeReal, repositoryReal)
    ) {
      workspaceFailure(
        "WORKTREE_ROOT_OVERLAP",
        "Canonical worktreeRoot must be separate from repositoryRoot.",
      );
    }
    await ensureCanonicalDirectory(
      this.#ownershipDirectory,
      "WORKTREE_PATH_ESCAPE",
      "Ownership directory traverses a link or junction.",
    );
    await ensureCanonicalDirectory(
      this.#hooksDirectory,
      "WORKTREE_PATH_ESCAPE",
      "Hooks directory traverses a link or junction.",
    );
    if ((await readdir(this.#hooksDirectory)).length > 0) {
      workspaceFailure(
        "WORKTREE_HOOKS_UNSAFE",
        "Git hooks directory must remain empty.",
      );
    }
  }

  async #assertNoExternalGitDrivers() {
    const result = await this.#git(
      [
        "config",
        "--local",
        "--null",
        "--get-regexp",
        "^(filter\\..*\\.(clean|smudge|process)|merge\\..*\\.driver|diff\\..*\\.(command|textconv))$",
      ],
      this.#repositoryRoot,
      { allowFailure: true },
    );
    if (result.exit_code !== 0 && result.exit_code !== 1) {
      workspaceFailure(
        "GIT_CONFIG_INSPECTION_FAILED",
        "Git execution configuration could not be inspected.",
        { exit_code: result.exit_code },
      );
    }
    if (result.exit_code === 0 && result.stdout.length > 0) {
      const keys = result.stdout
        .split("\0")
        .filter(Boolean)
        .map((entry) => entry.split("\n", 1)[0])
        .sort((left, right) => left.localeCompare(right));
      workspaceFailure(
        "GIT_EXECUTION_CONFIG_UNSAFE",
        "External Git filters, merge drivers, or diff drivers are disabled in Phase 1.",
        { keys },
      );
    }
  }

  #ownershipPath(worktreeId) {
    return path.join(
      this.#ownershipDirectory,
      `${safeWorktreeId(worktreeId).toLowerCase()}.json`,
    );
  }

  async createWorktree({ task_id, worktree_id, base_commit_sha }) {
    await this.#ensureRoots();
    const taskId = task_id;
    const worktreeId = safeWorktreeId(worktree_id);
    const baseCommit = safeHash(base_commit_sha, "base_commit_sha");
    const topLevel = (
      await this.#git(["rev-parse", "--show-toplevel"])
    ).stdout.trim();
    if (!samePath(topLevel, this.#repositoryRoot)) {
      workspaceFailure("REPOSITORY_ROOT_MISMATCH", "Git top level is not repositoryRoot.");
    }
    const verifiedBase = (
      await this.#git(["rev-parse", "--verify", `${baseCommit}^{commit}`])
    ).stdout.trim().toLowerCase();
    if (verifiedBase !== baseCommit) {
      workspaceFailure("BASE_COMMIT_MISMATCH", "Base commit did not resolve exactly.");
    }
    const repositoryStatus = (
      await this.#git([
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
      ])
    ).stdout;
    const unownedStatus = unownedRepositoryStatus(repositoryStatus);
    if (unownedStatus.length > 0) {
      workspaceFailure("REPOSITORY_DIRTY", "Repository root must be clean.", {
        status: unownedStatus,
      });
    }
    await this.#assertNoExternalGitDrivers();

    const target = path.join(this.#worktreeRoot, worktreeId.toLowerCase());
    if (!containedBy(this.#worktreeRoot, target) || (await optionalLstat(target))) {
      workspaceFailure("WORKTREE_TARGET_UNSAFE", "Worktree target is unsafe or exists.");
    }
    const ownershipPath = this.#ownershipPath(worktreeId);
    if (await optionalLstat(ownershipPath)) {
      workspaceFailure("WORKTREE_OWNERSHIP_EXISTS", "Ownership marker already exists.");
    }
    const branch = safeTaskBranch(taskId);
    const created = await this.#git(
      ["worktree", "add", "-b", branch, target, baseCommit],
      this.#repositoryRoot,
      { allowFailure: true },
    );
    if (created.exit_code !== 0) {
      workspaceFailure("GIT_WORKTREE_CREATE_FAILED", "Git worktree creation failed.", {
        stderr: created.stderr,
        branch,
      });
    }
    const actualHead = (
      await this.#git(["rev-parse", "HEAD"], target)
    ).stdout.trim().toLowerCase();
    if (actualHead !== baseCommit) {
      workspaceFailure("WORKTREE_BASE_MISMATCH", "Created worktree has the wrong HEAD.");
    }
    const ownership = {
      version: 1,
      worktree_id: worktreeId,
      task_id: taskId,
      repository_root: this.#repositoryRoot,
      worktree_path: target,
      branch,
      base_commit_sha: baseCommit,
    };
    await this.#verifyOwnedWorktreeIdentity(ownership);
    const createdStatus = (
      await this.#git(
        ["status", "--porcelain=v1", "--untracked-files=all"],
        target,
      )
    ).stdout;
    if (createdStatus.trim().length > 0) {
      workspaceFailure(
        "WORKTREE_DIRTY",
        "Created worktree must be clean before ownership is accepted.",
        { status: createdStatus },
      );
    }
    await durableCreate(ownershipPath, `${JSON.stringify(ownership)}\n`);
    return deepFreeze({
      ...ownership,
      path: target,
    });
  }

  async changedFiles({ worktree_id, worktreePath, base_commit_sha }) {
    const worktreeId = safeWorktreeId(worktree_id);
    const { marker } = await this.#readOwnership(worktreeId);
    if (
      typeof worktreePath !== "string" ||
      worktreePath.includes("\0") ||
      !samePath(worktreePath, marker.worktree_path)
    ) {
      workspaceFailure(
        "WORKTREE_NOT_OWNED",
        "Changed-file target does not match an owned task worktree.",
      );
    }
    const baseCommit = safeHash(base_commit_sha, "base_commit_sha");
    if (baseCommit !== marker.base_commit_sha) {
      workspaceFailure(
        "WORKTREE_BASE_MISMATCH",
        "Changed-file base does not match worktree ownership.",
      );
    }
    await this.#verifyOwnedWorktreeIdentity(marker);
    const target = marker.worktree_path;
    const trackedOutput = (
      await this.#git(
        ["diff", "--name-status", "-z", "-M", baseCommit, "--"],
        target,
      )
    ).stdout;
    const stagedPaths = new Set(
      (
        await this.#git(
          ["diff", "--cached", "--name-only", "-z", "-M", baseCommit, "--"],
          target,
        )
      ).stdout
        .split("\0")
        .filter(Boolean),
    );
    const unstagedPaths = new Set(
      (
        await this.#git(
          ["diff", "--name-only", "-z", "-M", "--"],
          target,
        )
      ).stdout
        .split("\0")
        .filter(Boolean),
    );
    const untrackedOutput = (
      await this.#git(
        ["ls-files", "--others", "--exclude-standard", "-z"],
        target,
      )
    ).stdout;
    const changes = parseNameStatus(trackedOutput);
    for (const change of changes) {
      const paths = [change.path, change.from_path].filter(Boolean);
      change.states = [];
      if (paths.some((changedPath) => stagedPaths.has(changedPath))) {
        change.states.push("staged");
      }
      if (paths.some((changedPath) => unstagedPaths.has(changedPath))) {
        change.states.push("unstaged");
      }
    }
    for (const untracked of untrackedOutput.split("\0").filter(Boolean)) {
      changes.push({
        operation: "create",
        path: untracked,
        from_path: null,
        git_status: "??",
        kind: "file",
        states: ["untracked"],
      });
    }
    changes.sort(
      (left, right) =>
        left.path.localeCompare(right.path) ||
        (left.from_path ?? "").localeCompare(right.from_path ?? ""),
    );
    return deepFreeze(changes);
  }

  async verifyIntegration({
    integration_id,
    target_commit_sha,
    feature_commit_sha,
  }) {
    await this.#ensureRoots();
    const integrationId = safeIntegrationId(integration_id);
    const targetCommit = safeHash(target_commit_sha, "target_commit_sha");
    const featureCommit = safeHash(feature_commit_sha, "feature_commit_sha");
    const topLevel = (
      await this.#git(["rev-parse", "--show-toplevel"])
    ).stdout.trim();
    if (!samePath(topLevel, this.#repositoryRoot)) {
      workspaceFailure("REPOSITORY_ROOT_MISMATCH", "Git top level is not repositoryRoot.");
    }
    const repositoryStatus = (
      await this.#git([
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
      ])
    ).stdout;
    const unownedStatus = unownedRepositoryStatus(repositoryStatus);
    if (unownedStatus.length > 0) {
      workspaceFailure("REPOSITORY_DIRTY", "Repository root must be clean.", {
        status: unownedStatus,
      });
    }
    for (const [field, expected] of [
      ["target_commit_sha", targetCommit],
      ["feature_commit_sha", featureCommit],
    ]) {
      const verified = (
        await this.#git(["rev-parse", "--verify", `${expected}^{commit}`])
      ).stdout.trim().toLowerCase();
      if (verified !== expected) {
        workspaceFailure("GIT_COMMIT_MISMATCH", `${field} did not resolve exactly.`);
      }
    }
    await this.#assertNoExternalGitDrivers();

    const verificationName = `integration-${integrationId.toLowerCase()}`;
    const target = path.join(this.#worktreeRoot, verificationName);
    if (!containedBy(this.#worktreeRoot, target) || (await optionalLstat(target))) {
      workspaceFailure(
        "INTEGRATION_TARGET_UNSAFE",
        "Integration verification target is unsafe or exists.",
      );
    }
    const markerPath = path.join(
      this.#ownershipDirectory,
      `${verificationName}.json`,
    );
    if (await optionalLstat(markerPath)) {
      workspaceFailure(
        "INTEGRATION_OWNERSHIP_EXISTS",
        "Integration ownership marker already exists.",
      );
    }

    const created = await this.#git(
      ["worktree", "add", "--detach", target, targetCommit],
      this.#repositoryRoot,
      { allowFailure: true },
    );
    if (created.exit_code !== 0) {
      workspaceFailure(
        "GIT_INTEGRATION_WORKTREE_FAILED",
        "Git integration worktree creation failed.",
        { stderr: created.stderr },
      );
    }
    const [targetStat, targetReal] = await Promise.all([
      lstat(target),
      realpath(target),
    ]);
    const actualHead = (
      await this.#git(["rev-parse", "HEAD"], target)
    ).stdout.trim().toLowerCase();
    if (
      !targetStat.isDirectory() ||
      targetStat.isSymbolicLink() ||
      !samePath(targetReal, target) ||
      actualHead !== targetCommit
    ) {
      workspaceFailure(
        "INTEGRATION_WORKTREE_MISMATCH",
        "Integration worktree identity does not match the requested target.",
      );
    }
    await durableCreate(
      markerPath,
      `${JSON.stringify({
        version: 1,
        kind: "integration-verification",
        integration_id: integrationId,
        repository_root: this.#repositoryRoot,
        worktree_path: target,
        target_commit_sha: targetCommit,
        feature_commit_sha: featureCommit,
      })}\n`,
    );

    const integrationMarker = deepFreeze({
      version: 1,
      kind: "integration-verification",
      integration_id: integrationId,
      repository_root: this.#repositoryRoot,
      worktree_path: target,
      target_commit_sha: targetCommit,
      feature_commit_sha: featureCommit,
    });
    let result;
    let primaryError = null;
    try {
      const mergeResult = await this.#git(
        ["merge", "--no-commit", "--no-ff", featureCommit],
        target,
        { allowFailure: true },
      );
      const conflicts = (
        await this.#git(
          ["diff", "--name-only", "--diff-filter=U", "-z", "--"],
          target,
        )
      ).stdout
        .split("\0")
        .filter(Boolean)
        .sort((left, right) => left.localeCompare(right));
      const changes =
        mergeResult.exit_code === 0
          ? parseNameStatus(
              (
                await this.#git(
                  ["diff", "--name-status", "-z", "-M", targetCommit, "--"],
                  target,
                )
              ).stdout,
            )
          : [];
      if (mergeResult.exit_code !== 0 && conflicts.length === 0) {
        workspaceFailure(
          "GIT_INTEGRATION_FAILED",
          "Git integration verification failed without conflict evidence.",
          { stderr: mergeResult.stderr },
        );
      }
      if (mergeResult.exit_code === 0 && conflicts.length > 0) {
        workspaceFailure(
          "GIT_INTEGRATION_FAILED",
          "Git reported conflict evidence after a successful merge.",
        );
      }
      result = deepFreeze({
        integration_id: integrationId,
        target_commit_sha: targetCommit,
        feature_commit_sha: featureCommit,
        can_integrate: mergeResult.exit_code === 0,
        outcome: mergeResult.exit_code === 0 ? "clean" : "conflict",
        conflicts,
        changes,
      });
    } catch (error) {
      primaryError = error;
    }

    let cleanupError = null;
    try {
      await this.#cleanupIntegrationVerification({
        marker: integrationMarker,
        markerPath,
      });
    } catch (error) {
      cleanupError = error;
    }
    if (primaryError) {
      if (cleanupError) {
        throw new WorkspaceError(
          primaryError.code ?? "GIT_INTEGRATION_FAILED",
          primaryError.message ?? "Git integration verification failed.",
          {
            ...(primaryError.details &&
            typeof primaryError.details === "object"
              ? primaryError.details
              : {}),
            cleanup_error: {
              code: cleanupError.code ?? "INTEGRATION_CLEANUP_FAILED",
              message: cleanupError.message,
            },
          },
        );
      }
      throw primaryError;
    }
    if (cleanupError) {
      throw cleanupError;
    }
    return result;
  }

  async #cleanupIntegrationVerification({ marker, markerPath }) {
    const markerStat = await optionalLstat(markerPath);
    if (!markerStat || !markerStat.isFile() || markerStat.isSymbolicLink()) {
      workspaceFailure(
        "INTEGRATION_OWNERSHIP_INVALID",
        "Integration ownership marker is missing or unsafe.",
      );
    }
    let storedMarker;
    try {
      storedMarker = JSON.parse(await readFile(markerPath, "utf8"));
    } catch {
      workspaceFailure(
        "INTEGRATION_OWNERSHIP_INVALID",
        "Integration ownership marker is invalid.",
      );
    }
    for (const field of [
      "version",
      "kind",
      "integration_id",
      "repository_root",
      "worktree_path",
      "target_commit_sha",
      "feature_commit_sha",
    ]) {
      const matches =
        field === "repository_root" || field === "worktree_path"
          ? typeof storedMarker[field] === "string" &&
            samePath(storedMarker[field], marker[field])
          : storedMarker[field] === marker[field];
      if (!matches) {
        workspaceFailure(
          "INTEGRATION_OWNERSHIP_INVALID",
          "Integration ownership marker does not match the cleanup target.",
        );
      }
    }
    const targetStat = await optionalLstat(marker.worktree_path);
    if (!targetStat || !targetStat.isDirectory() || targetStat.isSymbolicLink()) {
      workspaceFailure(
        "INTEGRATION_OWNERSHIP_INVALID",
        "Integration worktree is missing or unsafe.",
      );
    }
    const targetReal = await realpath(marker.worktree_path);
    const [topLevel, commonDirectory, repositoryCommonDirectory, actualHead] =
      await Promise.all([
        this.#git(
          ["rev-parse", "--show-toplevel"],
          marker.worktree_path,
          { allowFailure: true },
        ),
        this.#git(
          ["rev-parse", "--git-common-dir"],
          marker.worktree_path,
          { allowFailure: true },
        ),
        this.#git(
          ["rev-parse", "--git-common-dir"],
          this.#repositoryRoot,
          { allowFailure: true },
        ),
        this.#git(
          ["rev-parse", "HEAD"],
          marker.worktree_path,
          { allowFailure: true },
        ),
      ]);
    const targetCommonPath = path.resolve(
      marker.worktree_path,
      commonDirectory.stdout.trim(),
    );
    const repositoryCommonPath = path.resolve(
      this.#repositoryRoot,
      repositoryCommonDirectory.stdout.trim(),
    );
    if (
      !samePath(targetReal, marker.worktree_path) ||
      topLevel.exit_code !== 0 ||
      commonDirectory.exit_code !== 0 ||
      repositoryCommonDirectory.exit_code !== 0 ||
      actualHead.exit_code !== 0 ||
      !samePath(topLevel.stdout.trim(), marker.worktree_path) ||
      !samePath(targetCommonPath, repositoryCommonPath) ||
      actualHead.stdout.trim().toLowerCase() !== marker.target_commit_sha
    ) {
      workspaceFailure(
        "INTEGRATION_OWNERSHIP_INVALID",
        "Integration Git identity does not match the cleanup target.",
      );
    }
    const mergeHead = await this.#git(
      ["rev-parse", "-q", "--verify", "MERGE_HEAD"],
      marker.worktree_path,
      { allowFailure: true },
    );
    if (mergeHead.exit_code === 0) {
      const aborted = await this.#git(
        ["merge", "--abort"],
        marker.worktree_path,
        { allowFailure: true },
      );
      if (aborted.exit_code !== 0) {
        workspaceFailure(
          "GIT_INTEGRATION_ABORT_FAILED",
          "Git could not abort the verification merge safely.",
          { stderr: aborted.stderr },
        );
      }
    }
    const status = (
      await this.#git(
        ["status", "--porcelain=v1", "--untracked-files=all"],
        marker.worktree_path,
      )
    ).stdout;
    if (status.trim().length > 0) {
      workspaceFailure(
        "INTEGRATION_WORKTREE_DIRTY",
        "Integration cleanup refuses a dirty non-merge worktree.",
        { status },
      );
    }
    const removed = await this.#git(
      ["worktree", "remove", marker.worktree_path],
      this.#repositoryRoot,
      { allowFailure: true },
    );
    if (
      removed.exit_code !== 0 ||
      (await optionalLstat(marker.worktree_path))
    ) {
      workspaceFailure(
        "INTEGRATION_WORKTREE_REMOVE_FAILED",
        "Git did not safely remove the integration worktree.",
        { stderr: removed.stderr },
      );
    }
    await unlink(markerPath);
    return deepFreeze({
      cleaned: true,
      integration_id: marker.integration_id,
      path: marker.worktree_path,
    });
  }

  async #readOwnership(worktreeId) {
    await this.#ensureRoots();
    const markerPath = this.#ownershipPath(worktreeId);
    const expectedWorktreePath = path.join(
      this.#worktreeRoot,
      worktreeId.toLowerCase(),
    );
    const markerStat = await optionalLstat(markerPath);
    if (!markerStat || !markerStat.isFile() || markerStat.isSymbolicLink()) {
      workspaceFailure("WORKTREE_NOT_OWNED", "Ownership marker is missing or unsafe.");
    }
    let marker;
    try {
      marker = JSON.parse(await readFile(markerPath, "utf8"));
    } catch {
      workspaceFailure("WORKTREE_NOT_OWNED", "Ownership marker is invalid.");
    }
    if (
      marker.version !== 1 ||
      marker.worktree_id !== worktreeId ||
      typeof marker.task_id !== "string" ||
      !TASK_ID_PATTERN.test(marker.task_id) ||
      typeof marker.repository_root !== "string" ||
      typeof marker.worktree_path !== "string" ||
      typeof marker.branch !== "string" ||
      marker.branch !== safeTaskBranch(marker.task_id) ||
      typeof marker.base_commit_sha !== "string" ||
      !HASH_PATTERN.test(marker.base_commit_sha) ||
      !samePath(marker.repository_root, this.#repositoryRoot) ||
      !samePath(marker.worktree_path, expectedWorktreePath)
    ) {
      workspaceFailure("WORKTREE_NOT_OWNED", "Ownership marker does not match roots.");
    }
    return { marker: deepFreeze(marker), markerPath };
  }

  async #verifyOwnedWorktreeIdentity(marker) {
    const targetStat = await optionalLstat(marker.worktree_path);
    if (!targetStat || !targetStat.isDirectory() || targetStat.isSymbolicLink()) {
      workspaceFailure("WORKTREE_NOT_OWNED", "Owned worktree path is missing or unsafe.");
    }
    const targetRealPath = await realpath(marker.worktree_path);
    if (!samePath(targetRealPath, marker.worktree_path)) {
      workspaceFailure(
        "WORKTREE_NOT_OWNED",
        "Owned worktree path traverses a link or junction.",
      );
    }
    const [
      targetTopLevel,
      targetBranch,
      targetCommonDirectory,
      repositoryCommonDirectory,
      baseAncestor,
    ] = await Promise.all([
      this.#git(
        ["rev-parse", "--show-toplevel"],
        marker.worktree_path,
        { allowFailure: true },
      ),
      this.#git(
        ["symbolic-ref", "--quiet", "--short", "HEAD"],
        marker.worktree_path,
        { allowFailure: true },
      ),
      this.#git(
        ["rev-parse", "--git-common-dir"],
        marker.worktree_path,
        { allowFailure: true },
      ),
      this.#git(
        ["rev-parse", "--git-common-dir"],
        this.#repositoryRoot,
        { allowFailure: true },
      ),
      this.#git(
        ["merge-base", "--is-ancestor", marker.base_commit_sha, "HEAD"],
        marker.worktree_path,
        { allowFailure: true },
      ),
    ]);
    const targetCommonPath = path.resolve(
      marker.worktree_path,
      targetCommonDirectory.stdout.trim(),
    );
    const repositoryCommonPath = path.resolve(
      this.#repositoryRoot,
      repositoryCommonDirectory.stdout.trim(),
    );
    if (
      targetTopLevel.exit_code !== 0 ||
      targetBranch.exit_code !== 0 ||
      targetCommonDirectory.exit_code !== 0 ||
      repositoryCommonDirectory.exit_code !== 0 ||
      baseAncestor.exit_code !== 0 ||
      !samePath(targetTopLevel.stdout.trim(), marker.worktree_path) ||
      targetBranch.stdout.trim() !== marker.branch ||
      !samePath(targetCommonPath, repositoryCommonPath)
    ) {
      workspaceFailure(
        "WORKTREE_NOT_OWNED",
        "Worktree Git identity does not match its ownership marker.",
      );
    }
  }

  async removeOwnedWorktree({ worktree_id }) {
    const worktreeId = safeWorktreeId(worktree_id);
    const { marker, markerPath } = await this.#readOwnership(worktreeId);
    await this.#verifyOwnedWorktreeIdentity(marker);
    const status = (
      await this.#git(
        ["status", "--porcelain=v1", "--untracked-files=all"],
        marker.worktree_path,
      )
    ).stdout;
    if (status.trim().length > 0) {
      workspaceFailure("WORKTREE_DIRTY", "Dirty worktree is not removed.");
    }
    const removed = await this.#git(
      ["worktree", "remove", marker.worktree_path],
      this.#repositoryRoot,
      { allowFailure: true },
    );
    if (removed.exit_code !== 0 || (await optionalLstat(marker.worktree_path))) {
      workspaceFailure("WORKTREE_REMOVE_FAILED", "Git did not safely remove worktree.", {
        stderr: removed.stderr,
      });
    }
    await unlink(markerPath);
    return deepFreeze({
      removed: true,
      worktree_id: worktreeId,
      path: marker.worktree_path,
      branch: marker.branch,
    });
  }
}

export { WorkspaceError };
