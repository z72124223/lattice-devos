import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { readFile } from "node:fs/promises";
import { promisify } from "node:util";

import { ManagedWorktreeOwner } from "../apps/lattice-control/src/managed-worktree.mjs";
import { executionEnvironmentIdentity } from "../apps/lattice-control/src/wsl2-execution-domain.mjs";

const exec = promisify(execFile);
const canonical = (value) => Array.isArray(value) ? value.map(canonical)
  : value && typeof value === "object"
    ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]))
    : value;
const typedDigest = (prefix, value) => `${prefix}:sha256:${createHash("sha256")
  .update(JSON.stringify(canonical(value)), "utf8").digest("hex")}`;
const wsl = async (descriptor, executable, args) => (await exec(descriptor.gateway.windows_path, [
  "-d", descriptor.distribution, "--exec", executable, ...args,
], { encoding: "utf8", timeout: 30_000, windowsHide: true })).stdout.trimEnd();

const descriptor = JSON.parse(process.env.LATTICE_WSL2_PROBE_DESCRIPTOR ?? "null");
if (!descriptor || descriptor.kind !== "WSL2_LINUX") throw new Error("WSL2_DESCRIPTOR_REQUIRED");
const [topLevel, commonGitDir, head, status, linuxHeadSha] = await Promise.all([
  wsl(descriptor, descriptor.linux.git_path, ["-C", descriptor.linux.cwd, "rev-parse", "--show-toplevel"]),
  wsl(descriptor, descriptor.linux.git_path, ["-C", descriptor.linux.cwd, "rev-parse", "--git-common-dir"]),
  wsl(descriptor, descriptor.linux.git_path, ["-C", descriptor.linux.cwd, "rev-parse", "HEAD"]),
  wsl(descriptor, descriptor.linux.git_path, ["-C", descriptor.linux.cwd, "status", "--porcelain=v1"]),
  wsl(descriptor, "/usr/bin/sha256sum", [`${descriptor.linux.cwd}/.git/HEAD`]),
]);
if (topLevel !== descriptor.linux.cwd) throw new Error("WSL2_REPOSITORY_IDENTITY_MISMATCH");
const windowsHead = await readFile(`${descriptor.path_mapping.windows_path}\\.git\\HEAD`);
const windowsHeadSha = createHash("sha256").update(windowsHead).digest("hex");
if (windowsHeadSha !== linuxHeadSha.split(/\s+/u)[0]) throw new Error("WSL2_PATH_MAPPING_MISMATCH");
descriptor.linux.repository_identity = typedDigest("repository", {
  distribution: descriptor.distribution,
  cwd: descriptor.linux.cwd,
  top_level: topLevel,
  common_git_dir: commonGitDir,
  head,
  status,
  git_path: descriptor.linux.git_path,
  git_version: descriptor.linux.git_version,
  git_sha256: descriptor.linux.git_sha256,
});
descriptor.path_mapping.digest = typedDigest("path-mapping", {
  distribution: descriptor.distribution,
  windows_path: descriptor.path_mapping.windows_path,
  linux_path: descriptor.path_mapping.linux_path,
  shared_git_head_sha256: windowsHeadSha,
});
descriptor.identity_digest = executionEnvironmentIdentity(descriptor);
process.env.LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON = JSON.stringify(descriptor);

const taskRef = "d7b64e5f6e48f8cc9ab64d6f30c3a214826ecb365da6ff1f73f9f94c6c6d5b1a";
const owner = new ManagedWorktreeOwner({
  repositoryRoot: descriptor.path_mapping.windows_path,
  worktreeRoot: String.raw`\\wsl.localhost\Ubuntu\home\zk\lattice-phase4-wsl2-acceptance-20260828\managed-worktrees`,
  gitExecutable: String.raw`C:\Program Files\Git\cmd\git.exe`,
});
const prepared = await owner.prepare({
  task_ref: taskRef,
  task_id: "TASK-WSL2-CONNECTOR-PROBE",
  base_commit: "5a93f0f060a1c64d2c3bf81bf81bed6085a463ec",
});
process.stdout.write(`${JSON.stringify({
  schema: "lattice.phase4-wsl2-worktree-preflight/1.0",
  status: "PASS",
  execution_environment_ref: descriptor.identity_digest,
  baseline_sha256: prepared.baseline_sha256,
  worktree_path: prepared.worktree_path,
  head_commit: prepared.baseline.head_commit,
  head_tree: prepared.baseline.head_tree,
  initial_worktree_state: prepared.baseline.initial_worktree_state,
  replayed: prepared.replayed,
})}\n`);
