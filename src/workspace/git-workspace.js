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

function validateExecutorRequest({ command, args, cwd, environment }) {
  if (
    command !== "git" ||
    !Array.isArray(args) ||
    args.some((argument) => typeof argument !== "string" || argument.includes("\0")) ||
    typeof cwd !== "string" ||
    cwd.includes("\0") ||
    environment === null ||
    typeof environment !== "object" ||
    Array.isArray(environment) ||
    Object.entries(environment).some(
      ([key, value]) =>
        typeof key !== "string" ||
        key.includes("\0") ||
        typeof value !== "string" ||
        value.includes("\0"),
    )
  ) {
    workspaceFailure(
      "INVALID_GIT_COMMAND",
      "Git execution requires command='git', argument array, explicit cwd, and a sanitized environment.",
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
        env: request.environment,
        shell: false,
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

function kindFromGitModes(oldMode, newMode) {
  const modes = [oldMode, newMode].filter(
    (mode) => typeof mode === "string" && mode !== "000000",
  );
  if (modes.some((mode) => mode === "120000")) {
    return "link";
  }
  if (modes.some((mode) => mode === "160000")) {
    return "gitlink";
  }
  if (modes.length > 0 && modes.every((mode) => /^100\d{3}$/.test(mode))) {
    return "file";
  }
  return "unknown";
}

function parseRawDiff(output) {
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
    const header = tokens[index++];
    const match =
      /^:(\d{6}) (\d{6}) ([0-9a-f]+) ([0-9a-f]+) ([A-Z])(\d*)$/.exec(
        header,
      );
    const firstPath = tokens[index++];
    if (!match || typeof firstPath !== "string") {
      workspaceFailure("GIT_STATUS_PARSE_FAILED", "Malformed Git raw diff output.");
    }
    const [, oldMode, newMode, , , code, score] = match;
    let changedPath = firstPath;
    let fromPath = null;
    if (code === "R" || code === "C") {
      changedPath = tokens[index++];
      fromPath = firstPath;
      if (typeof changedPath !== "string") {
        workspaceFailure(
          "GIT_STATUS_PARSE_FAILED",
          "Rename/copy destination is missing.",
        );
      }
    }
    const operation = {
      A: "create",
      C: "create",
      D: "delete",
      M: "modify",
      R: "rename",
      T: "typechange",
      U: "unmerged",
    }[code];
    changes.push({
      operation: operation ?? "unknown",
      path: changedPath,
      from_path: fromPath,
      git_status: `${code}${score}`,
      kind: kindFromGitModes(oldMode, newMode),
      old_mode: oldMode,
      new_mode: newMode,
    });
  }
  return changes;
}

async function worktreePathKind(worktreeRoot, repositoryRelativePath) {
  if (
    typeof repositoryRelativePath !== "string" ||
    repositoryRelativePath.length === 0 ||
    repositoryRelativePath.includes("\0") ||
    path.posix.isAbsolute(repositoryRelativePath) ||
    path.win32.isAbsolute(repositoryRelativePath)
  ) {
    return "escaped";
  }
  const components = repositoryRelativePath
    .replaceAll("\\", "/")
    .split("/")
    .filter((component) => component.length > 0);
  if (
    components.length === 0 ||
    components.some((component) => component === "." || component === "..")
  ) {
    return "escaped";
  }
  let cursor = worktreeRoot;
  let finalStat = null;
  for (const component of components) {
    cursor = path.join(cursor, component);
    finalStat = await optionalLstat(cursor);
    if (!finalStat) {
      return "missing";
    }
    if (finalStat.isSymbolicLink()) {
      return "link";
    }
  }
  let resolved;
  try {
    resolved = await realpath(cursor);
  } catch {
    return "unknown";
  }
  const worktreeReal = await realpath(worktreeRoot);
  if (!containedBy(worktreeReal, resolved)) {
    return "escaped";
  }
  if (finalStat.isFile()) {
    return "file";
  }
  if (finalStat.isDirectory()) {
    return "directory";
  }
  return "unknown";
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
  #emptyGitConfigPath;
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
    this.#emptyGitConfigPath = path.join(
      this.#worktreeRoot,
      ".lattice-empty-gitconfig",
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
    const emptyConfigStat = await optionalLstat(this.#emptyGitConfigPath);
    if (
      !emptyConfigStat ||
      !emptyConfigStat.isFile() ||
      emptyConfigStat.isSymbolicLink() ||
      emptyConfigStat.size !== 0 ||
      !samePath(await realpath(this.#emptyGitConfigPath), this.#emptyGitConfigPath)
    ) {
      workspaceFailure(
        "GIT_ENVIRONMENT_UNSAFE",
        "Git isolation config must be an empty canonical file.",
      );
    }
    const environment = Object.create(null);
    for (const [key, value] of Object.entries(process.env)) {
      if (
        typeof value === "string" &&
        !key.toUpperCase().startsWith("GIT_") &&
        key.toUpperCase() !== "GCM_INTERACTIVE"
      ) {
        environment[key] = value;
      }
    }
    Object.assign(environment, {
      GCM_INTERACTIVE: "Never",
      GIT_ATTR_NOSYSTEM: "1",
      GIT_CONFIG_GLOBAL: this.#emptyGitConfigPath,
      GIT_CONFIG_NOSYSTEM: "1",
      GIT_TERMINAL_PROMPT: "0",
    });
    const protectedArgs = [
      "-c",
      `core.hooksPath=${this.#hooksDirectory}`,
      "-c",
      "core.fsmonitor=false",
      "-c",
      `core.attributesFile=${this.#emptyGitConfigPath}`,
      ...args,
    ];
    validateExecutorRequest({
      command: "git",
      args: protectedArgs,
      cwd,
      environment,
    });
    const result = await this.#executor({
      command: "git",
      args: [...protectedArgs],
      cwd,
      environment: Object.freeze({ ...environment }),
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
    const emptyConfigStat = await optionalLstat(this.#emptyGitConfigPath);
    if (!emptyConfigStat) {
      try {
        await durableCreate(this.#emptyGitConfigPath, "");
      } catch (error) {
        if (error?.code !== "EEXIST") {
          throw error;
        }
      }
    }
    const verifiedEmptyConfig = await optionalLstat(this.#emptyGitConfigPath);
    if (
      !verifiedEmptyConfig ||
      !verifiedEmptyConfig.isFile() ||
      verifiedEmptyConfig.isSymbolicLink() ||
      verifiedEmptyConfig.size !== 0 ||
      !samePath(await realpath(this.#emptyGitConfigPath), this.#emptyGitConfigPath)
    ) {
      workspaceFailure(
        "GIT_ENVIRONMENT_UNSAFE",
        "Git isolation config must be an empty canonical file.",
      );
    }
  }

  async #assertNoExternalGitDrivers(cwd = this.#repositoryRoot) {
    const pattern =
      "^(include\\.path|includeif\\..*\\.path|filter\\..*\\.(clean|smudge|process)|merge\\..*\\.driver|diff\\.external|diff\\..*\\.(command|textconv))$";
    const worktreeConfig = await this.#git(
      [
        "config",
        "--local",
        "--includes",
        "--bool",
        "--get",
        "extensions.worktreeConfig",
      ],
      cwd,
      { allowFailure: true },
    );
    if (worktreeConfig.exit_code !== 0 && worktreeConfig.exit_code !== 1) {
      workspaceFailure(
        "GIT_CONFIG_INSPECTION_FAILED",
        "Git worktree configuration mode could not be inspected.",
        { exit_code: worktreeConfig.exit_code },
      );
    }
    const scopes =
      worktreeConfig.exit_code === 0 &&
      worktreeConfig.stdout.trim().toLowerCase() === "true"
        ? ["--local", "--worktree"]
        : ["--local"];
    const results = await Promise.all(
      scopes.map((scope) =>
        this.#git(
          ["config", scope, "--includes", "--null", "--get-regexp", pattern],
          cwd,
          { allowFailure: true },
        ),
      ),
    );
    const inspectionFailure = results.find(
      (result) => result.exit_code !== 0 && result.exit_code !== 1,
    );
    if (inspectionFailure) {
      workspaceFailure(
        "GIT_CONFIG_INSPECTION_FAILED",
        "Git execution configuration could not be inspected.",
        { exit_code: inspectionFailure.exit_code },
      );
    }
    const keys = [
      ...new Set(
        results.flatMap((result) =>
          result.exit_code === 0
            ? result.stdout
                .split("\0")
                .filter(Boolean)
                .map((entry) => entry.split("\n", 1)[0])
            : [],
        ),
      ),
    ].sort((left, right) => left.localeCompare(right));
    if (keys.length > 0) {
      workspaceFailure(
        "GIT_EXECUTION_CONFIG_UNSAFE",
        "External Git filters, merge drivers, or diff drivers are disabled in Phase 1.",
        { keys },
      );
    }
  }

  async #driverSafeGit(
    args,
    cwd = this.#repositoryRoot,
    options = undefined,
  ) {
    await this.#assertNoExternalGitDrivers(cwd);
    return this.#git(args, cwd, options);
  }

  async #ignoredFiles(cwd) {
    return (
      await this.#git(
        ["ls-files", "--others", "--ignored", "--exclude-standard", "-z"],
        cwd,
      )
    ).stdout
      .split("\0")
      .filter(Boolean)
      .sort((left, right) => left.localeCompare(right));
  }

  #ownershipPath(worktreeId) {
    return path.join(
      this.#ownershipDirectory,
      `${safeWorktreeId(worktreeId).toLowerCase()}.json`,
    );
  }

  async createWorktree({ task_id, worktree_id, base_commit_sha }) {
    const taskId = task_id;
    const worktreeId = safeWorktreeId(worktree_id);
    const baseCommit = safeHash(base_commit_sha, "base_commit_sha");
    const branch = safeTaskBranch(taskId);
    await this.#ensureRoots();
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
      await this.#driverSafeGit([
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
    const branchRef = `refs/heads/${branch}`;
    const existingBranch = await this.#git(
      ["show-ref", "--verify", "--quiet", branchRef],
      this.#repositoryRoot,
      { allowFailure: true },
    );
    if (existingBranch.exit_code === 0) {
      workspaceFailure(
        "WORKTREE_BRANCH_EXISTS",
        "Task branch already exists and is not owned by this create attempt.",
        { branch },
      );
    }
    if (existingBranch.exit_code !== 1) {
      workspaceFailure(
        "GIT_BRANCH_INSPECTION_FAILED",
        "Task branch existence could not be verified safely.",
        { branch, exit_code: existingBranch.exit_code },
      );
    }

    const target = path.join(this.#worktreeRoot, worktreeId.toLowerCase());
    if (!containedBy(this.#worktreeRoot, target) || (await optionalLstat(target))) {
      workspaceFailure("WORKTREE_TARGET_UNSAFE", "Worktree target is unsafe or exists.");
    }
    const ownershipPath = this.#ownershipPath(worktreeId);
    if (await optionalLstat(ownershipPath)) {
      workspaceFailure("WORKTREE_OWNERSHIP_EXISTS", "Ownership marker already exists.");
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
    let primaryError = null;
    let worktreeAdded = false;
    try {
      const created = await this.#driverSafeGit(
        ["worktree", "add", "-b", branch, target, baseCommit],
        this.#repositoryRoot,
        { allowFailure: true },
      );
      if (created.exit_code !== 0) {
        workspaceFailure(
          "GIT_WORKTREE_CREATE_FAILED",
          "Git worktree creation failed.",
          {
            stderr: created.stderr,
            branch,
          },
        );
      }
      worktreeAdded = true;
      const actualHead = (
        await this.#git(["rev-parse", "HEAD"], target)
      ).stdout.trim().toLowerCase();
      if (actualHead !== baseCommit) {
        workspaceFailure(
          "WORKTREE_BASE_MISMATCH",
          "Created worktree has the wrong HEAD.",
        );
      }
      await this.#verifyOwnedWorktreeIdentity(ownership);
      const createdStatus = (
        await this.#driverSafeGit(
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
      const createdIgnored = await this.#ignoredFiles(target);
      if (createdIgnored.length > 0) {
        workspaceFailure(
          "WORKTREE_DIRTY",
          "Created worktree contains ignored files before ownership is accepted.",
          { ignored: createdIgnored },
        );
      }
      await durableCreate(ownershipPath, `${JSON.stringify(ownership)}\n`);
      return deepFreeze({
        ...ownership,
        path: target,
      });
    } catch (error) {
      primaryError = error;
    }
    if (!worktreeAdded) {
      throw primaryError;
    }
    try {
      await this.#recoverCreatedWorktree({
        target,
        expectedHead: baseCommit,
        expectedBranch: branch,
        deleteBranch: worktreeAdded,
        markerPath: ownershipPath,
      });
    } catch (cleanupError) {
      throw new WorkspaceError(
        primaryError.code ?? "GIT_WORKTREE_CREATE_FAILED",
        primaryError.message ?? "Git worktree creation failed.",
        {
          ...(primaryError.details && typeof primaryError.details === "object"
            ? primaryError.details
            : {}),
          cleanup_error: {
            code: cleanupError.code ?? "WORKTREE_RECOVERY_FAILED",
            message: cleanupError.message,
          },
        },
      );
    }
    throw primaryError;
  }

  async changedFiles({ worktree_id, worktreePath, base_commit_sha }) {
    const worktreeId = safeWorktreeId(worktree_id);
    if (
      typeof worktreePath !== "string" ||
      worktreePath.trim().length === 0 ||
      worktreePath.includes("\0")
    ) {
      workspaceFailure(
        "WORKTREE_NOT_OWNED",
        "Changed-file target is invalid.",
      );
    }
    const baseCommit = safeHash(base_commit_sha, "base_commit_sha");
    const { marker } = await this.#readOwnership(worktreeId);
    if (!samePath(worktreePath, marker.worktree_path)) {
      workspaceFailure(
        "WORKTREE_NOT_OWNED",
        "Changed-file target does not match an owned task worktree.",
      );
    }
    if (baseCommit !== marker.base_commit_sha) {
      workspaceFailure(
        "WORKTREE_BASE_MISMATCH",
        "Changed-file base does not match worktree ownership.",
      );
    }
    await this.#verifyOwnedWorktreeIdentity(marker);
    const target = marker.worktree_path;
    const stagedOutput = (
      await this.#driverSafeGit(
        [
          "diff",
          "--no-ext-diff",
          "--no-textconv",
          "--cached",
          "--raw",
          "-z",
          "-M",
          baseCommit,
          "--",
        ],
        target,
      )
    ).stdout;
    const unstagedOutput = (
      await this.#driverSafeGit(
        ["diff", "--no-ext-diff", "--no-textconv", "--raw", "-z", "-M", "--"],
        target,
      )
    ).stdout;
    const untrackedOutput = (
      await this.#git(
        ["ls-files", "--others", "--exclude-standard", "-z"],
        target,
      )
    ).stdout;
    const ignoredOutput = await this.#ignoredFiles(target);
    const changes = [
      ...parseRawDiff(stagedOutput).map((change) => ({
        ...change,
        states: ["staged"],
      })),
      ...parseRawDiff(unstagedOutput).map((change) => ({
        ...change,
        states: ["unstaged"],
      })),
    ];
    for (const change of changes) {
      if (change.operation !== "delete") {
        const pathKind = await worktreePathKind(target, change.path);
        if (change.kind === "file" && pathKind !== "file") {
          change.kind = pathKind;
        }
      }
    }
    for (const untracked of untrackedOutput.split("\0").filter(Boolean)) {
      changes.push({
        operation: "create",
        path: untracked,
        from_path: null,
        git_status: "??",
        kind: await worktreePathKind(target, untracked),
        old_mode: null,
        new_mode: null,
        states: ["untracked"],
      });
    }
    for (const ignored of ignoredOutput) {
      changes.push({
        operation: "create",
        path: ignored,
        from_path: null,
        git_status: "!!",
        kind: await worktreePathKind(target, ignored),
        old_mode: null,
        new_mode: null,
        states: ["ignored"],
      });
    }
    changes.sort(
      (left, right) =>
        left.path.localeCompare(right.path) ||
        (left.from_path ?? "").localeCompare(right.from_path ?? "") ||
        left.states[0].localeCompare(right.states[0]) ||
        left.operation.localeCompare(right.operation),
    );
    return deepFreeze(changes);
  }

  async verifyIntegration({
    integration_id,
    target_commit_sha,
    feature_commit_sha,
  }) {
    const integrationId = safeIntegrationId(integration_id);
    const targetCommit = safeHash(target_commit_sha, "target_commit_sha");
    const featureCommit = safeHash(feature_commit_sha, "feature_commit_sha");
    await this.#ensureRoots();
    const topLevel = (
      await this.#git(["rev-parse", "--show-toplevel"])
    ).stdout.trim();
    if (!samePath(topLevel, this.#repositoryRoot)) {
      workspaceFailure("REPOSITORY_ROOT_MISMATCH", "Git top level is not repositoryRoot.");
    }
    const repositoryStatus = (
      await this.#driverSafeGit([
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

    const integrationMarker = deepFreeze({
      version: 1,
      kind: "integration-verification",
      integration_id: integrationId,
      repository_root: this.#repositoryRoot,
      worktree_path: target,
      target_commit_sha: targetCommit,
      feature_commit_sha: featureCommit,
    });
    let setupError = null;
    let integrationWorktreeAdded = false;
    try {
      const created = await this.#driverSafeGit(
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
      integrationWorktreeAdded = true;
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
        `${JSON.stringify(integrationMarker)}\n`,
      );
    } catch (error) {
      setupError = error;
    }
    if (setupError) {
      if (!integrationWorktreeAdded) {
        throw setupError;
      }
      try {
        await this.#recoverCreatedWorktree({
          target,
          expectedHead: targetCommit,
          expectedBranch: null,
          markerPath,
        });
      } catch (cleanupError) {
        throw new WorkspaceError(
          setupError.code ?? "GIT_INTEGRATION_WORKTREE_FAILED",
          setupError.message ?? "Git integration worktree creation failed.",
          {
            ...(setupError.details && typeof setupError.details === "object"
              ? setupError.details
              : {}),
            cleanup_error: {
              code: cleanupError.code ?? "WORKTREE_RECOVERY_FAILED",
              message: cleanupError.message,
            },
          },
        );
      }
      throw setupError;
    }
    let result;
    let primaryError = null;
    try {
      const mergeResult = await this.#driverSafeGit(
        ["merge", "--no-commit", "--no-ff", featureCommit],
        target,
        { allowFailure: true },
      );
      const conflicts = (
        await this.#driverSafeGit(
          [
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--name-only",
            "--diff-filter=U",
            "-z",
            "--",
          ],
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
                await this.#driverSafeGit(
                  [
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--name-status",
                    "-z",
                    "-M",
                    targetCommit,
                    "--",
                  ],
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

  async #recoverCreatedWorktree({
    target,
    expectedHead,
    expectedBranch,
    deleteBranch = false,
    markerPath,
  }) {
    const targetStat = await optionalLstat(target);
    if (!targetStat) {
      if (deleteBranch) {
        await this.#deleteOwnedRecoveryBranch(expectedBranch, expectedHead);
      }
      const markerStat = await optionalLstat(markerPath);
      if (markerStat) {
        if (!markerStat.isFile() || markerStat.isSymbolicLink()) {
          workspaceFailure(
            "WORKTREE_RECOVERY_FAILED",
            "Recovery marker is unsafe.",
          );
        }
        await unlink(markerPath);
      }
      return deepFreeze({ cleaned: true, path: target });
    }
    if (!targetStat.isDirectory() || targetStat.isSymbolicLink()) {
      workspaceFailure(
        "WORKTREE_RECOVERY_FAILED",
        "Recovery target is not a canonical worktree directory.",
      );
    }
    const targetReal = await realpath(target);
    const [topLevel, commonDirectory, repositoryCommonDirectory, actualHead, branch] =
      await Promise.all([
        this.#git(
          ["rev-parse", "--show-toplevel"],
          target,
          { allowFailure: true },
        ),
        this.#git(
          ["rev-parse", "--git-common-dir"],
          target,
          { allowFailure: true },
        ),
        this.#git(
          ["rev-parse", "--git-common-dir"],
          this.#repositoryRoot,
          { allowFailure: true },
        ),
        this.#git(
          ["rev-parse", "HEAD"],
          target,
          { allowFailure: true },
        ),
        this.#git(
          ["symbolic-ref", "--quiet", "--short", "HEAD"],
          target,
          { allowFailure: true },
        ),
      ]);
    const targetCommonPath = path.resolve(
      target,
      commonDirectory.stdout.trim(),
    );
    const repositoryCommonPath = path.resolve(
      this.#repositoryRoot,
      repositoryCommonDirectory.stdout.trim(),
    );
    const branchMatches =
      expectedBranch === null
        ? branch.exit_code !== 0 && branch.stdout.trim().length === 0
        : branch.exit_code === 0 && branch.stdout.trim() === expectedBranch;
    if (
      !samePath(targetReal, target) ||
      topLevel.exit_code !== 0 ||
      commonDirectory.exit_code !== 0 ||
      repositoryCommonDirectory.exit_code !== 0 ||
      actualHead.exit_code !== 0 ||
      !samePath(topLevel.stdout.trim(), target) ||
      !samePath(targetCommonPath, repositoryCommonPath) ||
      actualHead.stdout.trim().toLowerCase() !== expectedHead ||
      !branchMatches
    ) {
      workspaceFailure(
        "WORKTREE_RECOVERY_FAILED",
        "Recovery target Git identity does not match the create attempt.",
      );
    }
    const status = (
      await this.#driverSafeGit(
        ["status", "--porcelain=v1", "--untracked-files=all"],
        target,
      )
    ).stdout;
    if (status.trim().length > 0) {
      workspaceFailure(
        "WORKTREE_RECOVERY_FAILED",
        "Recovery refuses to remove a dirty worktree.",
        { status },
      );
    }
    const ignored = await this.#ignoredFiles(target);
    if (ignored.length > 0) {
      workspaceFailure(
        "WORKTREE_RECOVERY_FAILED",
        "Recovery refuses to remove a worktree containing ignored files.",
        { ignored },
      );
    }
    const removed = await this.#driverSafeGit(
      ["worktree", "remove", target],
      this.#repositoryRoot,
      { allowFailure: true },
    );
    if (removed.exit_code !== 0 || (await optionalLstat(target))) {
      workspaceFailure(
        "WORKTREE_RECOVERY_FAILED",
        "Git did not remove the exact recovery worktree.",
        { stderr: removed.stderr },
      );
    }
    if (deleteBranch) {
      await this.#deleteOwnedRecoveryBranch(expectedBranch, expectedHead);
    }
    const markerStat = await optionalLstat(markerPath);
    if (markerStat) {
      if (!markerStat.isFile() || markerStat.isSymbolicLink()) {
        workspaceFailure(
          "WORKTREE_RECOVERY_FAILED",
          "Recovery marker is unsafe.",
        );
      }
      await unlink(markerPath);
    }
    return deepFreeze({ cleaned: true, path: target });
  }

  async #deleteOwnedRecoveryBranch(expectedBranch, expectedHead) {
    if (expectedBranch === null) {
      return;
    }
    const branchRef = `refs/heads/${expectedBranch}`;
    const worktreeList = await this.#git(
      ["worktree", "list", "--porcelain"],
      this.#repositoryRoot,
    );
    if (
      worktreeList.stdout
        .split(/\r?\n/)
        .some((line) => line === `branch ${branchRef}`)
    ) {
      workspaceFailure(
        "WORKTREE_RECOVERY_FAILED",
        "Recovery branch is still checked out by a worktree.",
        { branch: expectedBranch },
      );
    }
    const deleted = await this.#git(
      ["update-ref", "-d", branchRef, expectedHead],
      this.#repositoryRoot,
      { allowFailure: true },
    );
    const remaining = await this.#git(
      ["show-ref", "--verify", "--quiet", branchRef],
      this.#repositoryRoot,
      { allowFailure: true },
    );
    if (
      deleted.exit_code !== 0 ||
      (remaining.exit_code !== 0 && remaining.exit_code !== 1) ||
      remaining.exit_code === 0
    ) {
      workspaceFailure(
        "WORKTREE_RECOVERY_FAILED",
        "Recovery could not compare-and-delete the exact task branch.",
        {
          branch: expectedBranch,
          delete_exit_code: deleted.exit_code,
          remaining_exit_code: remaining.exit_code,
        },
      );
    }
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
      const aborted = await this.#driverSafeGit(
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
      await this.#driverSafeGit(
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
    const ignored = await this.#ignoredFiles(marker.worktree_path);
    if (ignored.length > 0) {
      workspaceFailure(
        "INTEGRATION_WORKTREE_DIRTY",
        "Integration cleanup refuses a worktree containing ignored files.",
        { ignored },
      );
    }
    const removed = await this.#driverSafeGit(
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
      await this.#driverSafeGit(
        ["status", "--porcelain=v1", "--untracked-files=all"],
        marker.worktree_path,
      )
    ).stdout;
    if (status.trim().length > 0) {
      workspaceFailure("WORKTREE_DIRTY", "Dirty worktree is not removed.", {
        status,
      });
    }
    const ignored = await this.#ignoredFiles(marker.worktree_path);
    if (ignored.length > 0) {
      workspaceFailure(
        "WORKTREE_DIRTY",
        "Worktree containing ignored files is not removed.",
        { ignored },
      );
    }
    const removed = await this.#driverSafeGit(
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
