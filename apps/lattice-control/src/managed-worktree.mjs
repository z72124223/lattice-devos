import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import {
  lstat,
  readFile,
  realpath,
  stat,
} from "node:fs/promises";
import path from "node:path";

import { canonicalize, deepFreeze } from "../../../src/domain/canonical-json.js";
import { WorkspaceError } from "../../../src/workspace/errors.js";
import { GitWorkspace } from "../../../src/workspace/git-workspace.js";
import {
  validateWsl2ExecutionEnvironment,
  windowsWslPathToLinux,
} from "./wsl2-execution-domain.mjs";

export const MANAGED_WORKTREE_BASELINE_SCHEMA =
  "lattice.managed-worktree-baseline/1.0";
export const MANAGED_WORKTREE_PRODUCER_ID =
  "lattice-control-managed-worktree";
export const MANAGED_WORKTREE_PRODUCER_VERSION = "1.0";

const TASK_REF = /^[a-f0-9]{64}$/u;
const TASK_ID = /^TASK-[A-Z0-9][A-Z0-9_-]{2,63}$/u;
const OID = /^[a-f0-9]{40}$/u;
const EXECUTION_ENVIRONMENT_REF = /^execution-environment:sha256:[a-f0-9]{64}$/u;
const MAX_GIT_OUTPUT_BYTES = 65_536;
const MAX_CONTROL_FILE_BYTES = 32 * 1024 * 1024;
const HARDENED_GIT_ARGUMENTS = Object.freeze([
  "-c", "core.hooksPath=/dev/null",
  "-c", "core.fsmonitor=false",
  "-c", "core.untrackedCache=false",
  "-c", "protocol.allow=never",
  "-c", "protocol.file.allow=never",
  "-c", "protocol.ext.allow=never",
]);

function managedFailure(code, message) {
  throw new WorkspaceError(code, message);
}

function sha256Bytes(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function framedDigest(domain, parts) {
  const hash = createHash("sha256");
  hash.update(domain, "utf8");
  hash.update(Buffer.from([0]));
  for (const part of parts) {
    const bytes = Buffer.isBuffer(part) ? part : Buffer.from(part, "utf8");
    const length = Buffer.alloc(8);
    length.writeBigUInt64BE(BigInt(bytes.length));
    hash.update(length);
    hash.update(bytes);
  }
  return hash.digest("hex");
}

function samePath(left, right) {
  const normalize = (value) => {
    const resolved = path.resolve(value);
    return process.platform === "win32" ? resolved.toLowerCase() : resolved;
  };
  return normalize(left) === normalize(right);
}

function containedBy(root, candidate) {
  const normalize = (value) => {
    const resolved = path.resolve(value);
    return process.platform === "win32" ? resolved.toLowerCase() : resolved;
  };
  const relative = path.relative(normalize(root), normalize(candidate));
  return relative.length > 0 && relative !== ".."
    && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

function managedIds(taskRef) {
  // Existing GitWorkspace identifiers are deliberately bounded at 64 bytes.
  // The complete task_ref remains in the durable baseline; this 236-bit name
  // is only a collision-resistant local locator.
  const suffix = taskRef.slice(0, 59).toUpperCase();
  return Object.freeze({
    task_id: `TASK-${suffix}`,
    worktree_id: `WORK-${suffix}`,
  });
}

async function canonicalFile(file, code) {
  const metadata = await lstat(file).catch(() => null);
  if (!metadata || !metadata.isFile() || metadata.isSymbolicLink()) {
    managedFailure(code, "Managed worktree control file is missing or unsafe.");
  }
  const canonical = await realpath(file);
  if (!samePath(canonical, file)) {
    managedFailure(code, "Managed worktree control file is not canonical.");
  }
  return canonical;
}

async function canonicalDirectory(directory, code) {
  const metadata = await lstat(directory).catch(() => null);
  if (!metadata || !metadata.isDirectory() || metadata.isSymbolicLink()) {
    managedFailure(code, "Managed worktree directory is missing or unsafe.");
  }
  const canonical = await realpath(directory);
  if (!samePath(canonical, directory)) {
    managedFailure(code, "Managed worktree directory is not canonical.");
  }
  return canonical;
}

async function boundedRead(file, code) {
  const metadata = await stat(file).catch(() => null);
  if (!metadata || !metadata.isFile() || metadata.size > MAX_CONTROL_FILE_BYTES) {
    managedFailure(code, "Managed worktree control file exceeds its closed bound.");
  }
  return readFile(file);
}

function absoluteExecutable(value) {
  return typeof value === "string"
    && value.length > 0
    && value.length <= 1_024
    && !value.includes("\0")
    && path.isAbsolute(value);
}

async function executeGit(gitExecutable, request) {
  if (
    request?.command !== "git"
    || !Array.isArray(request.args)
    || typeof request.cwd !== "string"
  ) {
    managedFailure("MANAGED_WORKTREE_GIT_REQUEST_REJECTED", "Git request is malformed.");
  }
  const wslEnvironment = configuredWslEnvironment();
  const command = wslEnvironment?.gateway.windows_path ?? gitExecutable;
  const mapWslArgument = (argument) => {
    if (typeof argument !== "string") return argument;
    const prefix = `\\\\wsl.localhost\\${wslEnvironment.distribution}\\`;
    const index = argument.toLowerCase().indexOf(prefix.toLowerCase());
    if (index < 0) return argument;
    const windowsValue = argument.slice(index);
    const linuxValue = windowsWslPathToLinux(windowsValue, wslEnvironment.distribution);
    return `${argument.slice(0, index)}${linuxValue}`;
  };
  const args = wslEnvironment === null
    ? [...HARDENED_GIT_ARGUMENTS, ...request.args]
    : [
        "-d", wslEnvironment.distribution,
        "--exec", "env", "-i",
        `HOME=${wslEnvironment.linux.codex_home}`,
        "PATH=/usr/bin:/bin",
        "GIT_CONFIG_NOSYSTEM=1",
        "GIT_CONFIG_GLOBAL=/dev/null",
        "GIT_TERMINAL_PROMPT=0",
        wslEnvironment.linux.git_path,
        "-C", windowsWslPathToLinux(request.cwd, wslEnvironment.distribution),
        ...HARDENED_GIT_ARGUMENTS,
        ...request.args.map(mapWslArgument),
      ];
  return new Promise((resolve) => {
    execFile(
      command,
      args,
      {
        cwd: wslEnvironment === null ? request.cwd : undefined,
        encoding: "utf8",
        windowsHide: true,
        maxBuffer: MAX_GIT_OUTPUT_BYTES,
        env: wslEnvironment === null ? {
          SystemRoot: process.env.SystemRoot,
          WINDIR: process.env.WINDIR,
          PATH: path.dirname(gitExecutable),
          GIT_CONFIG_NOSYSTEM: "1",
          GIT_CONFIG_GLOBAL: process.platform === "win32" ? "NUL" : "/dev/null",
          GIT_TERMINAL_PROMPT: "0",
        } : {
          SystemRoot: process.env.SystemRoot,
          WINDIR: process.env.WINDIR,
        },
      },
      (error, stdout, stderr) => resolve({
        exit_code: error === null ? 0 : Number.isInteger(error.code) ? error.code : 1,
        stdout: wslEnvironment === null ? stdout ?? "" : (stdout ?? "")
          .split(/(\r?\n)/u)
          .map((line) => line.startsWith("/")
            ? `\\\\wsl.localhost\\${wslEnvironment.distribution}${line.replaceAll("/", "\\")}`
            : line)
          .join(""),
        stderr: stderr ?? error?.message ?? "",
      }),
    );
  });
}

function configuredWslEnvironment() {
  const serialized = process.env.LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON;
  if (!serialized) return null;
  try {
    return validateWsl2ExecutionEnvironment(JSON.parse(serialized));
  } catch {
    managedFailure(
      "MANAGED_WORKTREE_EXECUTION_ENVIRONMENT_REJECTED",
      "Managed WSL2 execution environment is malformed.",
    );
  }
}

function assertExpectedExecutionEnvironmentRef(expectedExecutionEnvironmentRef) {
  const environment = configuredWslEnvironment();
  const accepted = environment === null
    ? expectedExecutionEnvironmentRef === null
    : typeof expectedExecutionEnvironmentRef === "string"
      && EXECUTION_ENVIRONMENT_REF.test(expectedExecutionEnvironmentRef)
      && expectedExecutionEnvironmentRef === environment.identity_digest;
  if (!accepted) {
    managedFailure(
      "MANAGED_WORKTREE_EXECUTION_ENVIRONMENT_BINDING_REJECTED",
      "Managed worktree execution environment does not match its independently expected ref.",
    );
  }
  return environment;
}

async function git(gitExecutable, cwd, args) {
  const result = await executeGit(gitExecutable, {
    command: "git",
    args,
    cwd,
  });
  if (result.exit_code !== 0 || Buffer.byteLength(result.stdout, "utf8") > MAX_GIT_OUTPUT_BYTES) {
    managedFailure("MANAGED_WORKTREE_GIT_REJECTED", "Git identity inspection failed.");
  }
  return result.stdout.trim();
}

async function gitAllowFailure(gitExecutable, cwd, args) {
  return executeGit(gitExecutable, { command: "git", args, cwd });
}

async function assertWslOperationBinding({
  environment,
  repositoryRoot,
  worktreeRoot,
  gitExecutable,
  taskRef,
  worktreeId,
  baseCommit,
  operation,
}) {
  if (environment === null) return;
  const bindingCode = "MANAGED_WORKTREE_EXECUTION_ENVIRONMENT_BINDING_REJECTED";
  const reject = () => managedFailure(
    bindingCode,
    "Managed WSL2 execution environment is not bound to this worktree operation.",
  );
  if (
    environment.verification_toolchain.task_ref !== taskRef
    || environment.linux.repository_head !== baseCommit
  ) {
    reject();
  }
  const source = await canonicalDirectory(repositoryRoot, bindingCode).catch(reject);
  const managedRoot = await canonicalDirectory(worktreeRoot, bindingCode).catch(reject);
  const descriptorRoot = await canonicalDirectory(
    environment.path_mapping.windows_path,
    bindingCode,
  ).catch(reject);
  const expectedManaged = path.join(managedRoot, worktreeId.toLowerCase());
  if (operation !== "prepare" || samePath(descriptorRoot, expectedManaged)) {
    if (!samePath(descriptorRoot, expectedManaged)) reject();
    return;
  }
  if (
    samePath(source, descriptorRoot)
    || containedBy(source, descriptorRoot)
    || containedBy(descriptorRoot, source)
    || samePath(managedRoot, descriptorRoot)
    || containedBy(managedRoot, descriptorRoot)
    || containedBy(descriptorRoot, managedRoot)
  ) {
    reject();
  }
  await canonicalDirectory(path.join(source, ".git"), bindingCode).catch(reject);
  await canonicalFile(path.join(descriptorRoot, ".git"), bindingCode).catch(reject);
  const sourceTop = await git(gitExecutable, source, ["rev-parse", "--show-toplevel"]);
  const descriptorTop = await git(
    gitExecutable,
    descriptorRoot,
    ["rev-parse", "--show-toplevel"],
  );
  const sourceCommonText = await git(
    gitExecutable,
    source,
    ["rev-parse", "--path-format=absolute", "--git-common-dir"],
  );
  const descriptorCommonText = await git(
    gitExecutable,
    descriptorRoot,
    ["rev-parse", "--path-format=absolute", "--git-common-dir"],
  );
  const descriptorGitText = await git(
    gitExecutable,
    descriptorRoot,
    ["rev-parse", "--path-format=absolute", "--git-dir"],
  );
  const sourceHead = await git(gitExecutable, source, ["rev-parse", "--verify", "HEAD^{commit}"]);
  const descriptorHead = await git(
    gitExecutable,
    descriptorRoot,
    ["rev-parse", "--verify", "HEAD^{commit}"],
  );
  const sourceCommon = await canonicalDirectory(sourceCommonText, bindingCode).catch(reject);
  const descriptorCommon = await canonicalDirectory(
    descriptorCommonText,
    bindingCode,
  ).catch(reject);
  const descriptorGit = await canonicalDirectory(descriptorGitText, bindingCode).catch(reject);
  if (
    !samePath(sourceTop, source)
    || !samePath(descriptorTop, descriptorRoot)
    || !samePath(sourceCommon, path.join(source, ".git"))
    || !samePath(descriptorCommon, sourceCommon)
    || !containedBy(path.join(sourceCommon, "worktrees"), descriptorGit)
    || sourceHead !== baseCommit
    || descriptorHead !== baseCommit
    || samePath(descriptorGit, descriptorCommon)
  ) {
    reject();
  }
}

async function hashOptionalControlFile(hash, rootName, root, relative) {
  const file = path.join(root, ...relative.split("/"));
  const metadata = await lstat(file).catch((error) => {
    if (error?.code === "ENOENT") return null;
    throw error;
  });
  hash.update(rootName, "utf8");
  hash.update(Buffer.from([0]));
  hash.update(relative, "utf8");
  hash.update(Buffer.from([0]));
  if (metadata !== null) {
    if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > MAX_CONTROL_FILE_BYTES) {
      managedFailure(
        "MANAGED_WORKTREE_CONTROL_UNSAFE",
        "Git control state contains an unsafe or oversized file.",
      );
    }
    hash.update(await readFile(file));
  }
  hash.update(Buffer.from([0xff]));
}

async function gitControlDigest(
  gitExecutable,
  worktree,
  gitDirectory,
  commonDirectory,
  taskRef,
) {
  const hash = createHash("sha256");
  hash.update("LATTICE_MANAGED_WORKTREE_CONTROL_V1", "utf8");
  hash.update(Buffer.from([0]));
  const roots = samePath(gitDirectory, commonDirectory)
    ? [["common", commonDirectory]]
    : [["worktree", gitDirectory], ["common", commonDirectory]];
  for (const [rootName, root] of roots) {
    for (const relative of [
      "HEAD",
      "commondir",
      "gitdir",
      "config",
      "config.worktree",
      "shallow",
      "info/attributes",
      "info/exclude",
      "info/grafts",
      "objects/info/alternates",
      "objects/info/http-alternates",
    ]) {
      await hashOptionalControlFile(hash, rootName, root, relative);
    }
  }
  const refs = await git(gitExecutable, worktree, [
    "for-each-ref",
    "--format=%(refname)%00%(objectname)%00%(objecttype)",
  ]);
  const protectedPrefix = `refs/lattice/managed/${taskRef}/attempt-`;
  const retainedRefs = refs.length === 0 ? [] : refs.split(/\r?\n/u);
  hash.update("refs", "utf8");
  hash.update(Buffer.from([0]));
  for (const record of retainedRefs) {
    const [ref, object, type, ...extra] = record.split("\0");
    if (
      extra.length !== 0
      || typeof ref !== "string"
      || !ref.startsWith("refs/")
      || ref.includes("..")
      || !OID.test(object ?? "")
      || !/^(blob|commit|tag|tree)$/u.test(type ?? "")
    ) {
      managedFailure(
        "MANAGED_WORKTREE_CONTROL_UNSAFE",
        "Git ref control state is malformed or unsafe.",
      );
    }
    if (
      ref.startsWith(protectedPrefix)
      && /^[1-3]$/u.test(ref.slice(protectedPrefix.length))
    ) {
      continue;
    }
    hash.update(ref, "utf8");
    hash.update(Buffer.from([0]));
    hash.update(object, "utf8");
    hash.update(Buffer.from([0]));
    hash.update(type, "utf8");
    hash.update(Buffer.from([0xff]));
  }
  return hash.digest("hex");
}

async function captureBaseline({
  repositoryRoot,
  worktreeRoot,
  gitExecutable,
  taskRef,
  ownership,
  baseCommit,
}) {
  const repository = await canonicalDirectory(
    repositoryRoot,
    "MANAGED_WORKTREE_REPOSITORY_UNSAFE",
  );
  const root = await canonicalDirectory(
    worktreeRoot,
    "MANAGED_WORKTREE_ROOT_UNSAFE",
  );
  const worktree = await canonicalDirectory(
    ownership.path,
    "MANAGED_WORKTREE_PATH_UNSAFE",
  );
  const markerPath = path.join(
    root,
    ".lattice-ownership",
    `${ownership.worktree_id.toLowerCase()}.json`,
  );
  await canonicalFile(markerPath, "MANAGED_WORKTREE_OWNERSHIP_UNSAFE");
  const ownershipBytes = await boundedRead(
    markerPath,
    "MANAGED_WORKTREE_OWNERSHIP_UNSAFE",
  );
  const dotGit = path.join(worktree, ".git");
  await canonicalFile(dotGit, "MANAGED_WORKTREE_GIT_POINTER_UNSAFE");
  const gitPointer = await boundedRead(dotGit, "MANAGED_WORKTREE_GIT_POINTER_UNSAFE");
  if (!gitPointer.toString("utf8").startsWith("gitdir: ")) {
    managedFailure(
      "MANAGED_WORKTREE_GIT_POINTER_UNSAFE",
      "Managed worktree .git pointer is malformed.",
    );
  }
  const gitDirectoryText = await git(gitExecutable, worktree, ["rev-parse", "--git-dir"]);
  const commonDirectoryText = await git(
    gitExecutable,
    worktree,
    ["rev-parse", "--git-common-dir"],
  );
  const gitDirectory = await canonicalDirectory(
    path.resolve(worktree, gitDirectoryText),
    "MANAGED_WORKTREE_GIT_DIRECTORY_UNSAFE",
  );
  const commonDirectory = await canonicalDirectory(
    path.resolve(worktree, commonDirectoryText),
    "MANAGED_WORKTREE_COMMON_DIRECTORY_UNSAFE",
  );
  const repositoryCommon = await canonicalDirectory(
    path.resolve(
      repository,
      await git(gitExecutable, repository, ["rev-parse", "--git-common-dir"]),
    ),
    "MANAGED_WORKTREE_COMMON_DIRECTORY_UNSAFE",
  );
  if (!samePath(commonDirectory, repositoryCommon)) {
    managedFailure(
      "MANAGED_WORKTREE_COMMON_DIRECTORY_MISMATCH",
      "Managed worktree does not share the registered repository Git directory.",
    );
  }
  const indexPath = path.join(gitDirectory, "index");
  await canonicalFile(indexPath, "MANAGED_WORKTREE_INDEX_UNSAFE");
  const indexBytes = await boundedRead(indexPath, "MANAGED_WORKTREE_INDEX_UNSAFE");
  const [headCommit, headTree, baseTree, branch] = await Promise.all([
    git(gitExecutable, worktree, ["rev-parse", "--verify", "HEAD^{commit}"]),
    git(gitExecutable, worktree, ["rev-parse", "--verify", "HEAD^{tree}"]),
    git(gitExecutable, worktree, ["rev-parse", "--verify", `${baseCommit}^{tree}`]),
    git(gitExecutable, worktree, ["symbolic-ref", "--quiet", "--short", "HEAD"]),
  ]);
  if (
    headCommit !== baseCommit
    || !OID.test(headCommit)
    || !OID.test(headTree)
    || !OID.test(baseTree)
    || headTree !== baseTree
    || branch !== ownership.branch
  ) {
    managedFailure(
      "MANAGED_WORKTREE_BASELINE_DRIFT",
      "Managed worktree branch, HEAD, or tree drifted from the retained base.",
    );
  }
  const baseline = {
    schema: MANAGED_WORKTREE_BASELINE_SCHEMA,
    task_ref: taskRef,
    ownership_digest: framedDigest(
      "LATTICE_MANAGED_WORKTREE_OWNERSHIP_V1",
      [ownershipBytes],
    ),
    repository_locator_digest: framedDigest(
      "LATTICE_MANAGED_REPOSITORY_LOCATOR_V1",
      [repository],
    ),
    worktree_locator_digest: framedDigest(
      "LATTICE_MANAGED_WORKTREE_LOCATOR_V1",
      [worktree],
    ),
    base_commit: baseCommit,
    base_tree: baseTree,
    task_branch: branch,
    head_commit: headCommit,
    head_tree: headTree,
    git_pointer_digest: sha256Bytes(gitPointer),
    git_directory_locator_digest: framedDigest(
      "LATTICE_MANAGED_GIT_DIRECTORY_LOCATOR_V1",
      [gitDirectory],
    ),
    common_git_directory_locator_digest: framedDigest(
      "LATTICE_MANAGED_COMMON_GIT_DIRECTORY_LOCATOR_V1",
      [commonDirectory],
    ),
    index_digest: sha256Bytes(indexBytes),
    git_control_digest: await gitControlDigest(
      gitExecutable,
      worktree,
      gitDirectory,
      commonDirectory,
      taskRef,
    ),
    initial_worktree_state: "CLEAN",
  };
  const baselineJson = canonicalize(baseline);
  return deepFreeze({
    baseline,
    baseline_json: baselineJson,
    baseline_sha256: sha256Bytes(Buffer.from(baselineJson, "utf8")),
    worktree_path: worktree,
  });
}

/** Existing GitWorkspace owner plus a closed Phase-4 baseline projection. */
export class ManagedWorktreeOwner {
  constructor({ repositoryRoot, worktreeRoot, gitExecutable }) {
    if (
      !absoluteExecutable(repositoryRoot)
      || !absoluteExecutable(worktreeRoot)
      || !absoluteExecutable(gitExecutable)
    ) {
      managedFailure(
        "MANAGED_WORKTREE_CONFIGURATION_REJECTED",
        "Managed repository, worktree root, and Git executable must be absolute.",
      );
    }
    this.repositoryRoot = path.resolve(repositoryRoot);
    this.worktreeRoot = path.resolve(worktreeRoot);
    this.gitExecutable = path.resolve(gitExecutable);
  }

  async prepare({
    task_ref,
    task_id,
    base_commit,
    expected_baseline_sha256 = null,
    expected_execution_environment_ref,
    operation = expected_baseline_sha256 === null ? "prepare" : "verify",
  }) {
    if (
      !TASK_REF.test(task_ref)
      || !TASK_ID.test(task_id)
      || !OID.test(base_commit)
      || (expected_baseline_sha256 !== null && !TASK_REF.test(expected_baseline_sha256))
      || (expected_baseline_sha256 === null
        ? operation !== "prepare"
        : !["verify", "protect"].includes(operation))
    ) {
      managedFailure(
        "MANAGED_WORKTREE_REQUEST_REJECTED",
        "Managed worktree request identities are malformed.",
      );
    }
    const environment = assertExpectedExecutionEnvironmentRef(
      expected_execution_environment_ref,
    );
    const ids = managedIds(task_ref);
    await canonicalFile(this.gitExecutable, "MANAGED_WORKTREE_GIT_EXECUTABLE_UNSAFE");
    await assertWslOperationBinding({
      environment,
      repositoryRoot: this.repositoryRoot,
      worktreeRoot: this.worktreeRoot,
      gitExecutable: this.gitExecutable,
      taskRef: task_ref,
      worktreeId: ids.worktree_id,
      baseCommit: base_commit,
      operation,
    });
    const workspace = new GitWorkspace({
      repositoryRoot: this.repositoryRoot,
      worktreeRoot: this.worktreeRoot,
      executor: (request) => executeGit(this.gitExecutable, request),
    });
    const ownership = await workspace.createOrReplayWorktree({
      task_id: ids.task_id,
      worktree_id: ids.worktree_id,
      base_commit_sha: base_commit,
      require_clean: expected_baseline_sha256 === null,
      create_if_missing: expected_baseline_sha256 === null,
    });
    const captured = await captureBaseline({
      repositoryRoot: this.repositoryRoot,
      worktreeRoot: this.worktreeRoot,
      gitExecutable: this.gitExecutable,
      taskRef: task_ref,
      ownership,
      baseCommit: base_commit,
    });
    if (
      expected_baseline_sha256 !== null
      && captured.baseline_sha256 !== expected_baseline_sha256
    ) {
      managedFailure(
        "MANAGED_WORKTREE_BASELINE_SUBSTITUTION",
        "Actual managed worktree control state does not match durable baseline evidence.",
      );
    }
    return deepFreeze({
      ...captured,
      task_id,
      local_task_id: ids.task_id,
      worktree_id: ids.worktree_id,
      branch: ownership.branch,
      replayed: ownership.replayed,
    });
  }

  async protectVerifiedResult({
    task_ref,
    task_id,
    attempt,
    writer_fence,
    base_commit,
    result_commit,
    expected_baseline_sha256,
    expected_execution_environment_ref,
    require_existing = false,
  }) {
    if (
      !Number.isSafeInteger(attempt)
      || attempt < 1
      || attempt > 3
      || !Number.isSafeInteger(writer_fence)
      || writer_fence < 1
      || !OID.test(result_commit)
      || !TASK_REF.test(expected_baseline_sha256)
      || typeof require_existing !== "boolean"
    ) {
      managedFailure(
        "MANAGED_WORKTREE_PROTECTED_REF_REJECTED",
        "Protected result ref request is malformed.",
      );
    }
    assertExpectedExecutionEnvironmentRef(expected_execution_environment_ref);
    const retained = await this.prepare({
      task_ref,
      task_id,
      base_commit,
      expected_baseline_sha256,
      expected_execution_environment_ref,
      operation: "protect",
    });
    const type = await git(this.gitExecutable, retained.worktree_path, [
      "cat-file",
      "-t",
      result_commit,
    ]);
    const lineage = await git(this.gitExecutable, retained.worktree_path, [
      "rev-list",
      "--parents",
      "-n",
      "1",
      result_commit,
    ]);
    if (type !== "commit" || lineage !== `${result_commit} ${base_commit}`) {
      managedFailure(
        "MANAGED_WORKTREE_RESULT_LINEAGE_REJECTED",
        "Verified result must be one exact child commit of the retained base.",
      );
    }
    const ref = `refs/lattice/managed/${task_ref}/attempt-${attempt}`;
    const observed = await git(this.gitExecutable, retained.worktree_path, [
      "for-each-ref",
      "--format=%(objectname)",
      "--",
      ref,
    ]);
    let replayed = false;
    if (observed.length > 0) {
      if (observed !== result_commit) {
        managedFailure(
          "MANAGED_WORKTREE_PROTECTED_REF_SUBSTITUTION",
          "Existing protected result ref points to a different commit.",
        );
      }
      replayed = true;
    } else {
      if (require_existing) {
        managedFailure(
          "MANAGED_WORKTREE_PROTECTED_REF_REQUIRED",
          "A post-advance protected result ref must already exist for exact replay.",
        );
      }
      const created = await gitAllowFailure(this.gitExecutable, retained.worktree_path, [
        "update-ref",
        ref,
        result_commit,
        "0".repeat(40),
      ]);
      if (created.exit_code !== 0) {
        const raced = await git(this.gitExecutable, retained.worktree_path, [
          "for-each-ref",
          "--format=%(objectname)",
          "--",
          ref,
        ]);
        if (raced !== result_commit) {
          managedFailure(
            "MANAGED_WORKTREE_PROTECTED_REF_REJECTED",
            "Protected result ref could not be atomically created or reconciled.",
          );
        }
        replayed = true;
      }
    }
    const exact = await git(this.gitExecutable, retained.worktree_path, [
      "for-each-ref",
      "--format=%(objectname)",
      "--",
      ref,
    ]);
    if (exact !== result_commit) {
      managedFailure(
        "MANAGED_WORKTREE_PROTECTED_REF_SUBSTITUTION",
        "Protected result ref replay is not exact.",
      );
    }
    return deepFreeze({
      task_ref,
      attempt,
      writer_fence,
      worktree_path: retained.worktree_path,
      protected_ref: ref,
      result_commit,
      baseline_sha256: retained.baseline_sha256,
      replayed,
      protected_ref_digest: framedDigest(
        "LATTICE_MANAGED_PROTECTED_RESULT_REF_V1",
        [
          task_ref,
          String(attempt),
          String(writer_fence),
          ref,
          result_commit,
          retained.baseline_sha256,
        ],
      ),
    });
  }
}
