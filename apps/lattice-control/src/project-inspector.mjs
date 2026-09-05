import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { constants as filesystemConstants } from "node:fs";
import { access, lstat, open, opendir, realpath } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const disabledHooksPath = process.platform === "win32" ? "NUL" : "/dev/null";
const maximumGitOutputBytes = 4 * 1024 * 1024;
const maximumGitConfigBytes = 1024 * 1024;
const maximumGitMetadataEntries = 50_000;
const maximumGitMetadataScanMs = 15_000;
const maximumRuleBytes = 1024 * 1024;
const maximumRuleTotalBytes = 64 * 1024 * 1024;
const maximumRuleDocuments = 4_096;
const maximumScanEntries = 25_000;
const maximumScanDepth = 16;
const maximumScanDurationMs = 30_000;
const maximumObservationFailures = 256;
const maximumRemoteUrlsPerDirection = 16;
const maximumGitObservationMs = 30_000;
const trustedGitWorkingDirectory = path.dirname(process.execPath);
const ignoredDirectories = new Set([
  ".git",
  ".gradle",
  ".idea",
  ".next",
  ".nuxt",
  ".venv",
  ".vscode",
  "build",
  "coverage",
  "dist",
  "node_modules",
  "out",
  "target",
  "venv",
]);
const standardRuleDocuments = ["AGENTS.md", "PROJECT_STATE.md", "PLANS.md"];
const rulePurposes = new Map([
  ["agents.md", "Codex and agent working rules"],
  ["project_state.md", "Current project state and accepted scope"],
  ["plans.md", "Current implementation plan and trajectory"],
  ["plan.md", "Current implementation plan and trajectory"],
  ["module_constitution.md", "Module mission, ownership, and invariants"],
  ["project_charter.md", "Project charter and governance boundaries"],
  ["contributing.md", "Contribution and verification workflow"],
  ["claude.md", "Assistant working instructions"],
  ["gemini.md", "Assistant working instructions"],
  ["copilot-instructions.md", "Assistant working instructions"],
  ["done_criteria.md", "Completion acceptance criteria"],
  ["verify.md", "Verification commands and evidence rules"],
  ["roadmap.md", "Project roadmap and milestones"],
  ["dirty_worktree_index.md", "Recorded working tree state index"],
]);
const allowedRemoteProtocols = new Set(["file:", "git:", "http:", "https:", "ssh:"]);
const redirectedGitMetadataFiles = new Set([
  "commondir",
  "objects/info/alternates",
  "objects/info/http-alternates",
]);
const WSL2_PROJECT_PATH_SCHEMA = "lattice.wsl2-project-path/1.0";

export class ProjectInspectionError extends Error {
  constructor(code, message, options = {}) {
    super(message, options);
    this.name = "ProjectInspectionError";
    this.code = code;
  }
}

function inspectionFailure(code, message, options) {
  throw new ProjectInspectionError(code, message, options);
}

function normalizedPath(value) {
  const resolved = path.normalize(path.resolve(value));
  return process.platform === "win32" ? resolved.toLowerCase() : resolved;
}

function samePath(left, right) {
  return normalizedPath(left) === normalizedPath(right);
}

function containedBy(parent, candidate) {
  const relative = path.relative(parent, candidate);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function sameFileIdentity(left, right) {
  return left.dev === right.dev && left.ino === right.ino;
}

function hasUnsafeControlCharacters(value) {
  return /[\u0000-\u001f\u007f-\u009f]/u.test(value);
}

function projectPathDigest(value) {
  return `wsl2-project-path:sha256:${createHash("sha256")
    .update(JSON.stringify(value), "utf8").digest("hex")}`;
}

export function parseWsl2ProjectPath(value) {
  if (process.platform !== "win32" || typeof value !== "string"
    || hasUnsafeControlCharacters(value) || value.length > 32_767) return null;
  const normalized = path.win32.normalize(value);
  const match = /^\\\\wsl\.localhost\\([A-Za-z0-9._-]{1,128})\\(.+)$/iu.exec(normalized);
  if (match === null) return null;
  const components = match[2].split("\\");
  if (components.length < 3 || components[0] !== "home"
    || components.some((component) => component.length === 0 || component === "."
      || component === ".." || /[\\/\0]/u.test(component))) return null;
  const linuxPath = `/${components.join("/")}`;
  const subject = {
    schema: WSL2_PROJECT_PATH_SCHEMA,
    distribution: match[1],
    linux_path: linuxPath,
    windows_path: `\\\\wsl.localhost\\${match[1]}\\${components.join("\\")}`,
  };
  return Object.freeze({ ...subject, identity_ref: projectPathDigest(subject) });
}

function boundedFailures(failures, stage) {
  if (failures.length <= maximumObservationFailures) return failures;
  return [
    ...failures.slice(0, maximumObservationFailures - 1),
    {
      stage,
      code: `${stage.toUpperCase()}_FAILURE_LIMIT_EXCEEDED`,
      message: "Additional bounded observation failures were not retained",
    },
  ];
}

function safeObservationTime(clock) {
  const value = clock();
  const timestamp = value instanceof Date ? value : new Date(value);
  if (!Number.isFinite(timestamp.getTime())) {
    inspectionFailure("OBSERVATION_TIME_INVALID", "project observation clock returned an invalid time");
  }
  return timestamp.toISOString();
}

export function normalizeRequestedProjectPath(value) {
  if (typeof value !== "string" || value.trim() === "") {
    inspectionFailure("PROJECT_PATH_REQUIRED", "project path is required");
  }
  if (
    hasUnsafeControlCharacters(value)
    || value.length > 32_767
    || !path.isAbsolute(value)
  ) {
    inspectionFailure("PROJECT_PATH_INVALID", "project path must be a safe absolute path");
  }
  if (process.platform === "win32") {
    const wsl2 = parseWsl2ProjectPath(value);
    if (wsl2 !== null) return wsl2.windows_path;
    if (
      value.startsWith("\\\\")
      || value.startsWith("\\\\?\\")
      || value.startsWith("\\\\.\\")
      || !/^[a-zA-Z]:[\\/]/u.test(value)
    ) {
      inspectionFailure(
        "PROJECT_PATH_UNSAFE_NAMESPACE",
        "project path must use a local Windows drive-letter namespace",
      );
    }
  }
  return path.resolve(value);
}

async function canonicalProjectDirectory(value) {
  const requested = normalizeRequestedProjectPath(value);
  const parsed = path.parse(requested);
  const segments = path.relative(parsed.root, requested).split(path.sep).filter(Boolean);
  let cursor = parsed.root;
  const pathIdentities = [];
  for (const segment of segments) {
    cursor = path.join(cursor, segment);
    let details;
    try {
      details = await lstat(cursor, { bigint: true });
    } catch (error) {
      if (error?.code === "ENOENT") {
        inspectionFailure("PROJECT_PATH_NOT_FOUND", "project path does not exist", { cause: error });
      }
      inspectionFailure("PROJECT_PATH_UNREADABLE", "project path cannot be inspected", { cause: error });
    }
    if (details.isSymbolicLink()) {
      inspectionFailure(
        "PROJECT_PATH_REDIRECTED",
        "project path must not traverse a symbolic link or junction",
      );
    }
    pathIdentities.push({ path: cursor, dev: details.dev, ino: details.ino });
  }

  let details;
  let canonical;
  try {
    [details, canonical] = await Promise.all([
      lstat(requested, { bigint: true }),
      realpath(requested),
    ]);
  } catch (error) {
    inspectionFailure("PROJECT_PATH_UNREADABLE", "project path cannot be inspected", { cause: error });
  }
  if (!details.isDirectory()) {
    inspectionFailure("PROJECT_PATH_NOT_DIRECTORY", "project path must name a directory");
  }
  if (details.isSymbolicLink() || !samePath(requested, canonical)) {
    inspectionFailure(
      "PROJECT_PATH_REDIRECTED",
      "project path must not resolve through a symbolic link or junction",
    );
  }
  return { canonical_path: canonical, path_identities: pathIdentities };
}

export async function canonicalizeProjectPath(value) {
  const projectDirectory = await canonicalProjectDirectory(value);
  await verifyProjectDirectoryIdentity(projectDirectory);
  return projectDirectory.canonical_path;
}

async function verifyProjectDirectoryIdentity(projectDirectory) {
  for (const identity of projectDirectory.path_identities) {
    let details;
    try {
      details = await lstat(identity.path, { bigint: true });
    } catch (error) {
      inspectionFailure("PROJECT_PATH_CHANGED", "project path changed during inspection", {
        cause: error,
      });
    }
    if (
      details.isSymbolicLink()
      || details.dev !== identity.dev
      || details.ino !== identity.ino
    ) {
      inspectionFailure("PROJECT_PATH_CHANGED", "project path changed during inspection");
    }
  }
  let canonical;
  try {
    canonical = await realpath(projectDirectory.canonical_path);
  } catch (error) {
    inspectionFailure("PROJECT_PATH_CHANGED", "project path changed during inspection", {
      cause: error,
    });
  }
  if (!samePath(canonical, projectDirectory.canonical_path)) {
    inspectionFailure("PROJECT_PATH_CHANGED", "project path changed during inspection");
  }
}

function safeGitFailure(stage, code, message, extra = {}) {
  return { stage, code, message, ...extra };
}

const gitCandidateCache = new Map();

async function resolvedGitCandidates(pathValue = process.env.PATH ?? "") {
  const cacheKey = `${process.platform}\0${pathValue}`;
  if (gitCandidateCache.has(cacheKey)) return gitCandidateCache.get(cacheKey);
  const candidates = [];
  const names = process.platform === "win32" ? ["git.exe"] : ["git"];
  for (let entry of pathValue.split(path.delimiter)) {
    entry = entry.trim().replace(/^"|"$/gu, "");
    if (!entry || !path.isAbsolute(entry)) continue;
    for (const name of names) {
      const candidate = path.join(entry, name);
      try {
        await access(candidate, filesystemConstants.X_OK);
        const canonical = await realpath(candidate);
        const details = await lstat(canonical);
        if (details.isFile() && !candidates.some((value) => samePath(value, canonical))) {
          candidates.push(canonical);
        }
      } catch {
        // PATH entries are untrusted configuration; unavailable candidates are ignored.
      }
    }
  }
  gitCandidateCache.set(cacheKey, candidates);
  return candidates;
}

async function nearestGitMarkerRoot(cwd) {
  let cursor = path.resolve(cwd);
  while (true) {
    try {
      await lstat(path.join(cursor, ".git"));
      return cursor;
    } catch (error) {
      if (error?.code !== "ENOENT" && error?.code !== "ENOTDIR") return cursor;
    }
    const parent = path.dirname(cursor);
    if (parent === cursor) return null;
    cursor = parent;
  }
}

function gitMetadataError(code, message, cause) {
  return new ProjectInspectionError(code, message, cause ? { cause } : {});
}

function requireGitObservationBudget(budget) {
  if (budget.remaining() < 1) {
    throw gitMetadataError(
      "GIT_OBSERVATION_TIMEOUT",
      "Git observation exceeded its total time limit",
    );
  }
}

async function inspectGitConfigFile(configPath, { required, budget, includeContent = false }) {
  requireGitObservationBudget(budget);
  let before;
  let handle;
  try {
    before = await lstat(configPath, { bigint: true });
  } catch (error) {
    if (!required && error?.code === "ENOENT") return null;
    throw gitMetadataError(
      "GIT_METADATA_UNREADABLE",
      "Git metadata configuration could not be safely inspected",
      error,
    );
  }
  try {
    if (before.isSymbolicLink() || !before.isFile()) {
      throw gitMetadataError(
        "GIT_METADATA_REDIRECTED",
        "Git metadata configuration must be a local regular file",
      );
    }
    if (before.size > BigInt(maximumGitConfigBytes)) {
      throw gitMetadataError(
        "GIT_METADATA_LIMIT_EXCEEDED",
        "Git metadata configuration exceeded the safe read limit",
      );
    }
    const canonical = await realpath(configPath);
    if (!samePath(canonical, configPath)) {
      throw gitMetadataError(
        "GIT_METADATA_REDIRECTED",
        "Git metadata configuration resolved outside its declared path",
      );
    }
    handle = await open(configPath, "r");
    requireGitObservationBudget(budget);
    const opened = await handle.stat({ bigint: true });
    if (
      !sameFileIdentity(before, opened)
      || before.size !== opened.size
      || before.mtimeMs !== opened.mtimeMs
    ) {
      throw gitMetadataError(
        "GIT_METADATA_CHANGED",
        "Git metadata configuration changed while it was opened",
      );
    }
    const { content, exceeded } = await readBoundedHandle(handle, maximumGitConfigBytes);
    requireGitObservationBudget(budget);
    if (exceeded) {
      throw gitMetadataError(
        "GIT_METADATA_LIMIT_EXCEEDED",
        "Git metadata configuration exceeded the safe read limit",
      );
    }
    const after = await handle.stat({ bigint: true });
    const [afterLink, afterCanonical] = await Promise.all([
      lstat(configPath, { bigint: true }),
      realpath(configPath),
    ]);
    if (
      afterLink.isSymbolicLink()
      || !samePath(afterCanonical, canonical)
      || !sameFileIdentity(afterLink, after)
      || !sameFileIdentity(opened, after)
      || opened.size !== after.size
      || opened.mtimeMs !== after.mtimeMs
    ) {
      throw gitMetadataError(
        "GIT_METADATA_CHANGED",
        "Git metadata configuration changed while it was inspected",
      );
    }
    const text = content.toString("latin1");
    if (/^[\t ]*\[[\t ]*include(?:if\b[^\]]*)?\]/imu.test(text)) {
      throw gitMetadataError(
        "GIT_CONFIG_INCLUDE_UNSAFE",
        "Repository-local Git configuration includes are not followed during observation",
      );
    }
    return {
      path: canonical,
      dev: after.dev.toString(),
      ino: after.ino.toString(),
      size: after.size.toString(),
      mtime_ns: after.mtimeNs?.toString() ?? after.mtimeMs.toString(),
      sha256: createHash("sha256").update(content).digest("hex"),
      ...(includeContent ? { content: content.toString("utf8") } : {}),
    };
  } catch (error) {
    if (error instanceof ProjectInspectionError) throw error;
    throw gitMetadataError(
      "GIT_METADATA_UNREADABLE",
      "Git metadata configuration could not be safely inspected",
      error,
    );
  } finally {
    await handle?.close().catch(() => {});
  }
}

async function inspectGitMetadataGuardPaths(metadataRoot, budget, { linkedWorktree = false } = {}) {
  const guardPaths = [
    "HEAD",
    "commondir",
    "gitdir",
    "config",
    "config.worktree",
    "index",
    "info",
    "objects",
    "objects/info",
    "objects/info/alternates",
    "objects/info/http-alternates",
    "packed-refs",
    "refs",
    "shallow",
  ];
  const fingerprint = [];
  for (const relativePath of guardPaths) {
    requireGitObservationBudget(budget);
    const absolutePath = path.join(metadataRoot, ...relativePath.split("/"));
    let details;
    try {
      details = await lstat(absolutePath, { bigint: true });
    } catch (error) {
      if (error?.code === "ENOENT" || error?.code === "ENOTDIR") {
        fingerprint.push([relativePath, "missing"]);
        continue;
      }
      throw gitMetadataError(
        "GIT_METADATA_UNREADABLE",
        "Git metadata guard path could not be safely inspected",
        error,
      );
    }
    requireGitObservationBudget(budget);
    if (details.isSymbolicLink()) {
      throw gitMetadataError(
        "GIT_METADATA_REDIRECTED",
        "Git metadata contains a symbolic link or junction",
      );
    }
    if (redirectedGitMetadataFiles.has(relativePath.toLowerCase())
      && !(linkedWorktree && relativePath === "commondir")) {
      throw gitMetadataError(
        "GIT_METADATA_REDIRECTED",
        "Git metadata contains an external metadata pointer",
      );
    }
    if (details.isDirectory()) {
      const canonical = await realpath(absolutePath);
      requireGitObservationBudget(budget);
      if (!containedBy(metadataRoot, canonical) || !samePath(canonical, absolutePath)) {
        throw gitMetadataError(
          "GIT_METADATA_REDIRECTED",
          "Git metadata directory resolved outside the repository metadata root",
        );
      }
    }
    fingerprint.push([
      relativePath,
      details.isDirectory() ? "directory" : details.isFile() ? "file" : "other",
      details.dev.toString(),
      details.ino.toString(),
      details.size.toString(),
      details.mtimeNs?.toString() ?? details.mtimeMs.toString(),
    ]);
  }
  return fingerprint;
}

async function scanGitMetadataTree(metadataRoot, budget) {
  const startedAt = performance.now();
  const fingerprint = createHash("sha256");
  let entriesObserved = 0;

  function requireScanBudget() {
    requireGitObservationBudget(budget);
    if (performance.now() - startedAt > maximumGitMetadataScanMs) {
      throw gitMetadataError(
        "GIT_METADATA_LIMIT_EXCEEDED",
        "Git metadata safety scan exceeded its time limit",
      );
    }
  }

  async function visit(directory, relativeDirectory) {
    requireScanBudget();
    let directoryHandle;
    const entries = [];
    try {
      directoryHandle = await opendir(directory);
      for await (const entry of directoryHandle) {
        requireScanBudget();
        entriesObserved += 1;
        if (entriesObserved > maximumGitMetadataEntries) {
          throw gitMetadataError(
            "GIT_METADATA_LIMIT_EXCEEDED",
            "Git metadata safety scan exceeded its entry limit",
          );
        }
        entries.push(entry.name);
      }
    } catch (error) {
      if (error instanceof ProjectInspectionError) throw error;
      throw gitMetadataError(
        "GIT_METADATA_UNREADABLE",
        "Git metadata tree could not be safely enumerated",
        error,
      );
    } finally {
      await directoryHandle?.close().catch(() => {});
    }

    for (const name of entries.sort((left, right) => left.localeCompare(right, "en"))) {
      requireScanBudget();
      const absolutePath = path.join(directory, name);
      const relativePath = relativeDirectory
        ? path.posix.join(relativeDirectory, name)
        : name;
      const normalizedRelativePath = relativePath.replaceAll("\\", "/").toLowerCase();
      let details;
      try {
        details = await lstat(absolutePath, { bigint: true });
        requireScanBudget();
      } catch (error) {
        if (error instanceof ProjectInspectionError) throw error;
        throw gitMetadataError(
          "GIT_METADATA_CHANGED",
          "Git metadata changed during its safety scan",
          error,
        );
      }
      if (details.isSymbolicLink()) {
        throw gitMetadataError(
          "GIT_METADATA_REDIRECTED",
          "Git metadata contains a symbolic link or junction",
        );
      }
      if (redirectedGitMetadataFiles.has(normalizedRelativePath)) {
        throw gitMetadataError(
          "GIT_METADATA_REDIRECTED",
          "Git metadata contains an external metadata pointer",
        );
      }
      const kind = details.isDirectory() ? "directory" : details.isFile() ? "file" : "other";
      fingerprint.update(JSON.stringify([
        normalizedRelativePath,
        kind,
        details.dev.toString(),
        details.ino.toString(),
        details.size.toString(),
        details.mtimeNs?.toString() ?? details.mtimeMs.toString(),
      ]));
      if (!details.isDirectory()) continue;
      let canonical;
      try {
        canonical = await realpath(absolutePath);
        requireScanBudget();
      } catch (error) {
        if (error instanceof ProjectInspectionError) throw error;
        throw gitMetadataError(
          "GIT_METADATA_CHANGED",
          "Git metadata directory changed during its safety scan",
          error,
        );
      }
      if (!containedBy(metadataRoot, canonical) || !samePath(canonical, absolutePath)) {
        throw gitMetadataError(
          "GIT_METADATA_REDIRECTED",
          "Git metadata directory resolved outside the repository metadata root",
        );
      }
      await visit(absolutePath, normalizedRelativePath);
    }
  }

  await visit(metadataRoot, "");
  return {
    entries: entriesObserved,
    sha256: fingerprint.digest("hex"),
  };
}

function gitMetadataGuardFingerprint(boundary) {
  const parsed = JSON.parse(boundary);
  delete parsed.tree;
  return JSON.stringify(parsed);
}

async function linkedWorktreeBoundary(root, markerPath, budget, scanTree) {
  const invalid = (cause) => gitMetadataError(
    "GIT_METADATA_REDIRECTED",
    "Git file must belong to a canonical linked worktree with matching metadata back-links",
    cause,
  );
  function requireAbsolutePointer(value) {
    if (!path.isAbsolute(value) || value.split(/[\\/]/u).some((part) => part === "." || part === "..")) {
      throw invalid();
    }
    try { normalizeRequestedProjectPath(value); } catch (error) { throw invalid(error); }
  }
  async function pointer(pointerPath, prefix = "") {
    const { content, ...identity } = await inspectGitConfigFile(pointerPath, {
      required: true, budget, includeContent: true,
    });
    const value = content.replace(/\r?\n$/u, "");
    if (!value.startsWith(prefix) || hasUnsafeControlCharacters(value)
      || value.length > 32_767 || value.length === prefix.length) throw invalid();
    return { value: value.slice(prefix.length), identity };
  }
  async function directory(value) {
    try {
      requireGitObservationBudget(budget);
      const result = await canonicalProjectDirectory(value);
      await verifyProjectDirectoryIdentity(result);
      requireGitObservationBudget(budget);
      return {
        path: result.canonical_path,
        identities: result.path_identities.map(({ path: entryPath, dev, ino }) => (
          [entryPath, dev.toString(), ino.toString()]
        )),
      };
    } catch (error) {
      if (error?.code === "GIT_OBSERVATION_TIMEOUT") throw error;
      throw invalid(error);
    }
  }

  const marker = await pointer(markerPath, "gitdir: ");
  requireAbsolutePointer(marker.value);
  const metadata = await directory(marker.value);
  const commonPointer = await pointer(path.join(metadata.path, "commondir"));
  if (commonPointer.value.replaceAll("\\", "/") !== "../..") {
    requireAbsolutePointer(commonPointer.value);
  }
  const common = await directory(path.resolve(metadata.path, commonPointer.value));
  if (!samePath(path.dirname(metadata.path), path.join(common.path, "worktrees"))) {
    throw invalid();
  }
  const backlink = await pointer(path.join(metadata.path, "gitdir"));
  requireAbsolutePointer(backlink.value);
  if (!samePath(backlink.value, markerPath)) throw invalid();

  const configs = [
    await inspectGitConfigFile(path.join(common.path, "config"), { required: true, budget }),
    await inspectGitConfigFile(path.join(metadata.path, "config.worktree"), {
      required: false, budget,
    }),
  ].filter(Boolean);
  const guard_paths = {
    common: await inspectGitMetadataGuardPaths(common.path, budget),
    worktree: await inspectGitMetadataGuardPaths(metadata.path, budget, { linkedWorktree: true }),
  };
  // The shared metadata tree contains this worktree's metadata as well. Only the
  // three checked pointer files above may connect it to this working directory.
  const tree = scanTree ? await scanGitMetadataTree(common.path, budget) : null;
  return JSON.stringify({
    kind: "linked-worktree", root, metadata, common,
    pointers: [marker.identity, commonPointer.identity, backlink.identity],
    configs, guard_paths, tree,
  });
}

async function gitMetadataBoundary(canonicalPath, budget, { scanTree = true } = {}) {
  requireGitObservationBudget(budget);
  let cursor = canonicalPath;
  while (true) {
    requireGitObservationBudget(budget);
    const markerPath = path.join(cursor, ".git");
    let marker;
    try {
      marker = await lstat(markerPath, { bigint: true });
    } catch (error) {
      if (error?.code !== "ENOENT" && error?.code !== "ENOTDIR") {
        throw gitMetadataError(
          "GIT_METADATA_UNREADABLE",
          "Git metadata boundary could not be safely inspected",
          error,
        );
      }
    }
    if (marker) {
      if (marker.isSymbolicLink()) {
        throw gitMetadataError(
          "GIT_METADATA_REDIRECTED",
          "Redirected Git metadata is not followed during project observation",
        );
      }
      if (marker.isFile()) {
        try {
          return await linkedWorktreeBoundary(cursor, markerPath, budget, scanTree);
        } catch (error) {
          if (error?.code === "GIT_METADATA_UNREADABLE") {
            throw gitMetadataError(
              "GIT_METADATA_REDIRECTED",
              "Git file does not resolve to complete linked-worktree metadata",
              error,
            );
          }
          throw error;
        }
      }
      if (!marker.isDirectory()) {
        throw gitMetadataError(
          "GIT_METADATA_INVALID",
          "Git metadata marker is not a local directory",
        );
      }
      const canonicalMarker = await realpath(markerPath);
      requireGitObservationBudget(budget);
      if (!samePath(canonicalMarker, markerPath)) {
        throw gitMetadataError(
          "GIT_METADATA_REDIRECTED",
          "Git metadata directory resolved outside its declared path",
        );
      }
      const configs = [
        await inspectGitConfigFile(path.join(markerPath, "config"), { required: true, budget }),
        await inspectGitConfigFile(path.join(markerPath, "config.worktree"), {
          required: false,
          budget,
        }),
      ].filter(Boolean);
      const guard_paths = await inspectGitMetadataGuardPaths(markerPath, budget);
      const tree = scanTree ? await scanGitMetadataTree(markerPath, budget) : null;
      return JSON.stringify({
        kind: "directory",
        root: cursor,
        marker: {
          path: canonicalMarker,
          dev: marker.dev.toString(),
          ino: marker.ino.toString(),
        },
        configs,
        guard_paths,
        tree,
      });
    }
    const parent = path.dirname(cursor);
    if (parent === cursor) break;
    cursor = parent;
  }
  let bareHead = null;
  let bareObjects = null;
  try {
    [bareHead, bareObjects] = await Promise.all([
      lstat(path.join(canonicalPath, "HEAD"), { bigint: true }),
      lstat(path.join(canonicalPath, "objects"), { bigint: true }),
    ]);
  } catch (error) {
    if (error?.code !== "ENOENT" && error?.code !== "ENOTDIR") {
      throw gitMetadataError(
        "GIT_METADATA_UNREADABLE",
        "Potential bare Git metadata could not be safely inspected",
        error,
      );
    }
  }
  if (bareHead && bareObjects) {
    if (
      bareHead.isSymbolicLink()
      || !bareHead.isFile()
      || bareObjects.isSymbolicLink()
      || !bareObjects.isDirectory()
    ) {
      throw gitMetadataError(
        "GIT_METADATA_REDIRECTED",
        "Potential bare Git metadata contains a redirected path",
      );
    }
    const canonicalObjects = await realpath(path.join(canonicalPath, "objects"));
    if (!containedBy(canonicalPath, canonicalObjects)) {
      throw gitMetadataError(
        "GIT_METADATA_REDIRECTED",
        "Potential bare Git metadata resolved outside the project path",
      );
    }
    const config = await inspectGitConfigFile(path.join(canonicalPath, "config"), {
      required: true,
      budget,
    });
    const guard_paths = await inspectGitMetadataGuardPaths(canonicalPath, budget);
    const tree = scanTree ? await scanGitMetadataTree(canonicalPath, budget) : null;
    return JSON.stringify({
      kind: "bare",
      root: canonicalPath,
      head: {
        dev: bareHead.dev.toString(),
        ino: bareHead.ino.toString(),
      },
      objects: {
        path: canonicalObjects,
        dev: bareObjects.dev.toString(),
        ino: bareObjects.ino.toString(),
      },
      configs: [config],
      guard_paths,
      tree,
    });
  }
  return JSON.stringify({ kind: "none", root: canonicalPath });
}

export async function resolveGitExecutable({ cwd, pathValue = process.env.PATH ?? "" }) {
  const projectPath = path.resolve(cwd);
  const repositoryBoundary = await nearestGitMarkerRoot(projectPath);
  for (const candidate of await resolvedGitCandidates(pathValue)) {
    if (containedBy(projectPath, candidate)) continue;
    if (repositoryBoundary && containedBy(repositoryBoundary, candidate)) continue;
    return candidate;
  }
  throw new ProjectInspectionError(
    "GIT_EXECUTABLE_UNAVAILABLE",
    "a trusted absolute Git executable could not be resolved",
  );
}

function safeGitEnvironment() {
  const environment = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (/^(?:GIT|GCM)_/iu.test(key)) continue;
    environment[key] = value;
  }
  return {
    ...environment,
    GCM_INTERACTIVE: "Never",
    GIT_ATTR_NOSYSTEM: "1",
    GIT_ALLOW_PROTOCOL: "",
    GIT_CONFIG_GLOBAL: disabledHooksPath,
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_SYSTEM: disabledHooksPath,
    GIT_OPTIONAL_LOCKS: "0",
    GIT_PAGER: "",
    GIT_TERMINAL_PROMPT: "0",
    LC_ALL: "C",
  };
}

export async function defaultGitExecutor({ cwd, args, timeoutMs = maximumGitObservationMs }) {
  if (
    typeof cwd !== "string"
    || !Array.isArray(args)
    || args.some((argument) => typeof argument !== "string" || argument.includes("\0"))
    || !Number.isSafeInteger(timeoutMs)
    || timeoutMs < 1
    || timeoutMs > maximumGitObservationMs
  ) {
    throw new TypeError("Git execution requires an explicit cwd and argument array");
  }
  try {
    const executable = await resolveGitExecutable({ cwd });
    const result = await execFileAsync(
      executable,
      [
        "--no-optional-locks",
        "-c",
        `core.hooksPath=${disabledHooksPath}`,
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.untrackedCache=false",
        "-c",
        `core.attributesFile=${disabledHooksPath}`,
        "-c",
        `core.excludesFile=${disabledHooksPath}`,
        "-c",
        "status.submoduleSummary=false",
        "-C",
        cwd,
        ...args,
      ],
      {
        cwd: trustedGitWorkingDirectory,
        encoding: "utf8",
        env: safeGitEnvironment(),
        maxBuffer: maximumGitOutputBytes,
        timeout: timeoutMs,
        windowsHide: true,
      },
    );
    return { exit_code: 0, stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
  } catch (error) {
    return {
      exit_code: Number.isInteger(error?.code) ? error.code : 1,
      error_code: error?.killed
        ? "GIT_COMMAND_TIMEOUT"
        : typeof error?.code === "string"
          ? error.code
          : error?.code ?? null,
      stdout: typeof error?.stdout === "string" ? error.stdout : "",
      stderr: typeof error?.stderr === "string" ? error.stderr : "",
    };
  }
}

function containedLinuxPath(parent, candidate) {
  const relative = path.posix.relative(parent, candidate);
  return relative === "" || (!relative.startsWith("../") && relative !== ".."
    && !path.posix.isAbsolute(relative));
}

function canonicalLinuxProjectPath(value) {
  return typeof value === "string" && value.startsWith("/home/")
    && path.posix.normalize(value) === value && !value.includes("\\")
    && !value.includes("\0") && !value.includes("/../") && !value.endsWith("/..")
    && !value.includes("/./") && !value.endsWith("/.");
}

function wsl2UncFromLinux(distribution, linuxPath) {
  return `\\\\wsl.localhost\\${distribution}${linuxPath.replaceAll("/", "\\")}`;
}

function closedWindowsGatewayEnvironment() {
  return Object.fromEntries(["SystemRoot", "WINDIR"].flatMap((key) => (
    process.env[key] === undefined ? [] : [[key, process.env[key]]]
  )));
}

export function createWsl2ProjectGitExecutor(untrusted, {
  executeFile = execFileAsync,
  systemRoot = process.env.SystemRoot ?? process.env.WINDIR,
} = {}) {
  const parsed = parseWsl2ProjectPath(untrusted?.windows_path);
  if (parsed === null || parsed.schema !== untrusted?.schema
    || parsed.distribution !== untrusted.distribution
    || parsed.linux_path !== untrusted.linux_path
    || parsed.identity_ref !== untrusted.identity_ref
    || typeof executeFile !== "function" || typeof systemRoot !== "string"
    || !/^[A-Za-z]:[\\/]Windows$/iu.test(path.win32.normalize(systemRoot))) {
    throw new TypeError("WSL2 project Git execution requires an exact local path identity");
  }
  const gateway = path.win32.join(systemRoot, "System32", "wsl.exe");
  const linuxHome = parsed.linux_path.split("/").slice(0, 3).join("/");
  return async ({ cwd, args, timeoutMs = maximumGitObservationMs }) => {
    if (typeof cwd !== "string" || !Array.isArray(args)
      || args.some((argument) => typeof argument !== "string" || argument.includes("\0"))
      || !Number.isSafeInteger(timeoutMs) || timeoutMs < 1
      || timeoutMs > maximumGitObservationMs) {
      throw new TypeError("Git execution requires an explicit cwd and argument array");
    }
    const cwdIdentity = parseWsl2ProjectPath(cwd);
    if (cwdIdentity === null
      || cwdIdentity.distribution.toLowerCase() !== parsed.distribution.toLowerCase()
      || !containedLinuxPath(cwdIdentity.linux_path, parsed.linux_path)
      || !containedLinuxPath(linuxHome, cwdIdentity.linux_path)) {
      throw new TypeError("WSL2 Git cwd must remain in the bound Linux project domain");
    }
    try {
      const result = await executeFile(gateway, [
        "-d", parsed.distribution, "--exec", "/usr/bin/env", "-i",
        `HOME=${linuxHome}`, "PATH=/usr/bin:/bin", "LANG=C.UTF-8", "LC_ALL=C.UTF-8",
        "GCM_INTERACTIVE=Never", "GIT_ATTR_NOSYSTEM=1", "GIT_ALLOW_PROTOCOL=",
        "GIT_CONFIG_GLOBAL=/dev/null", "GIT_CONFIG_NOSYSTEM=1",
        "GIT_CONFIG_SYSTEM=/dev/null", "GIT_OPTIONAL_LOCKS=0", "GIT_PAGER=",
        "GIT_TERMINAL_PROMPT=0", "/usr/bin/git", "--no-optional-locks",
        "-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false",
        "-c", "core.untrackedCache=false", "-c", "core.attributesFile=/dev/null",
        "-c", "core.excludesFile=/dev/null", "-c", "status.submoduleSummary=false",
        "-C", cwdIdentity.linux_path, ...args,
      ], {
        cwd: trustedGitWorkingDirectory,
        encoding: "utf8",
        env: closedWindowsGatewayEnvironment(),
        maxBuffer: maximumGitOutputBytes,
        timeout: timeoutMs,
        windowsHide: true,
      });
      let stdout = result.stdout ?? "";
      if (args.length === 2 && args[0] === "rev-parse" && args[1] === "--show-toplevel") {
        const lines = stdout.replaceAll("\r", "").split("\n").filter(Boolean);
        if (lines.length !== 1 || !canonicalLinuxProjectPath(lines[0])
          || !containedLinuxPath(lines[0], cwdIdentity.linux_path)
          || !containedLinuxPath(linuxHome, lines[0])) {
          return {
            exit_code: 1,
            error_code: "GIT_REPOSITORY_ROOT_INVALID",
            stdout: "",
            stderr: "",
          };
        }
        stdout = `${wsl2UncFromLinux(parsed.distribution, lines[0])}\n`;
      }
      return { exit_code: 0, stdout, stderr: result.stderr ?? "" };
    } catch (error) {
      return {
        exit_code: Number.isInteger(error?.code) ? error.code : 1,
        error_code: error?.killed
          ? "GIT_COMMAND_TIMEOUT"
          : typeof error?.code === "string" ? error.code : error?.code ?? null,
        stdout: typeof error?.stdout === "string" ? error.stdout : "",
        stderr: typeof error?.stderr === "string" ? error.stderr : "",
      };
    }
  };
}

function gitObservationBudget({
  maximumDurationMs = maximumGitObservationMs,
  monotonicClock = () => performance.now(),
} = {}) {
  if (
    !Number.isSafeInteger(maximumDurationMs)
    || maximumDurationMs < 1
    || maximumDurationMs > maximumGitObservationMs
  ) {
    throw new TypeError(`maximum Git duration must be 1 to ${maximumGitObservationMs} milliseconds`);
  }
  const startedAt = monotonicClock();
  return {
    remaining() {
      return Math.floor(maximumDurationMs - (monotonicClock() - startedAt));
    },
  };
}

function gitObservationExecutor(executor, budget) {
  return async (request) => {
    const remaining = budget.remaining();
    if (remaining < 1) {
      return {
        exit_code: 1,
        error_code: "GIT_OBSERVATION_TIMEOUT",
        stdout: "",
        stderr: "",
      };
    }
    let timeout;
    const timeoutResult = new Promise((resolve) => {
      timeout = setTimeout(() => resolve({
        exit_code: 1,
        error_code: "GIT_OBSERVATION_TIMEOUT",
        stdout: "",
        stderr: "",
      }), remaining);
      timeout.unref?.();
    });
    try {
      return await Promise.race([
        executor({ ...request, timeoutMs: Math.min(remaining, maximumGitObservationMs) }),
        timeoutResult,
      ]);
    } finally {
      clearTimeout(timeout);
    }
  };
}

function gitTimedOut(result) {
  return ["GIT_COMMAND_TIMEOUT", "GIT_OBSERVATION_TIMEOUT"].includes(result?.error_code);
}

async function runGit(executor, cwd, args) {
  const result = await executor({ cwd, args });
  if (!result || !Number.isInteger(result.exit_code)) {
    throw new TypeError("Git executor returned an invalid result");
  }
  return {
    exit_code: result.exit_code,
    error_code: result.error_code ?? null,
    stdout: typeof result.stdout === "string" ? result.stdout : "",
    stderr: typeof result.stderr === "string" ? result.stderr : "",
  };
}

function parseGitStatus(output, failures) {
  const observation = {
    branch: null,
    detached: null,
    head_sha: null,
    dirty: false,
    upstream: null,
    ahead: null,
    behind: null,
  };
  let sawBranchHead = false;
  for (const line of output.split(/\r?\n/gu)) {
    if (!line) continue;
    if (!line.startsWith("# ")) {
      observation.dirty = true;
      continue;
    }
    if (line.startsWith("# branch.oid ")) {
      const oid = line.slice("# branch.oid ".length);
      if (oid !== "(initial)" && !/^[a-f0-9]{40,64}$/u.test(oid)) {
        failures.push(safeGitFailure(
          "status",
          "GIT_HEAD_INVALID",
          "Git returned an invalid HEAD object ID",
        ));
      } else if (oid !== "(initial)") {
        observation.head_sha = oid;
      }
    } else if (line.startsWith("# branch.head ")) {
      sawBranchHead = true;
      const branch = line.slice("# branch.head ".length);
      if (branch === "(detached)") {
        observation.detached = true;
      } else {
        observation.branch = branch;
        observation.detached = false;
      }
    } else if (line.startsWith("# branch.upstream ")) {
      observation.upstream = line.slice("# branch.upstream ".length);
    } else if (line.startsWith("# branch.ab ")) {
      const match = line.match(/^# branch\.ab \+([0-9]+) -([0-9]+)$/u);
      if (!match) {
        failures.push(safeGitFailure(
          "status",
          "GIT_UPSTREAM_COUNTS_INVALID",
          "Git returned invalid ahead/behind counts",
        ));
      } else {
        observation.ahead = Number(match[1]);
        observation.behind = Number(match[2]);
      }
    }
  }
  if (!sawBranchHead) {
    failures.push(safeGitFailure(
      "status",
      "GIT_BRANCH_MISSING",
      "Git status did not report a branch or detached state",
    ));
  }
  return observation;
}

export function sanitizeRemoteUrl(value, repositoryRoot) {
  const raw = String(value ?? "").trim();
  if (!raw || raw.length > 4_096 || hasUnsafeControlCharacters(raw)) {
    throw new TypeError("Git remote URL is empty, oversized, or malformed");
  }

  const isWindowsPath = /^[a-zA-Z]:[\\/]/u.test(raw) || raw.startsWith("\\\\");
  const isLocalPath = isWindowsPath || path.isAbsolute(raw) || /^[.]{1,2}[\\/]/u.test(raw);
  if (isLocalPath) {
    return {
      url: path.normalize(path.resolve(repositoryRoot, raw)),
      credentials_redacted: false,
    };
  }

  if (!raw.includes("://")) {
    const scp = raw.match(
      /^(?:([^@/:?#\s]+)@)?([a-zA-Z0-9.-]+):([a-zA-Z0-9._~%+/-]+)$/u,
    );
    if (scp) {
      return {
        url: `${scp[2]}:${scp[3]}`,
        credentials_redacted: Boolean(scp[1]),
      };
    }
    if (!raw.includes(":") && !/[?#]/u.test(raw)) {
      return {
        url: path.normalize(path.resolve(repositoryRoot, raw)),
        credentials_redacted: false,
      };
    }
    throw new TypeError("Git remote URL uses an unsupported transport");
  }

  const parsed = new URL(raw);
  if (!allowedRemoteProtocols.has(parsed.protocol)) {
    throw new TypeError("Git remote URL uses an unsupported transport");
  }
  if (parsed.protocol !== "file:" && !parsed.hostname) {
    throw new TypeError("Git remote URL is missing a host");
  }
  const credentialsRedacted = Boolean(
    parsed.username || parsed.password || parsed.search || parsed.hash,
  );
  parsed.username = "";
  parsed.password = "";
  parsed.search = "";
  parsed.hash = "";
  return { url: parsed.toString(), credentials_redacted: credentialsRedacted };
}

async function observeRemotes(repositoryRoot, executor, failures) {
  const remoteList = await runGit(executor, repositoryRoot, ["remote"]);
  if (remoteList.exit_code !== 0) {
    failures.push(safeGitFailure(
      "remotes",
      gitTimedOut(remoteList) ? "GIT_OBSERVATION_TIMEOUT" : "GIT_REMOTE_LIST_FAILED",
      gitTimedOut(remoteList)
        ? "Git observation exceeded its total time limit"
        : "Git remote names could not be read",
    ));
    return [];
  }
  const names = remoteList.stdout.split(/\r?\n/gu).filter(Boolean).sort();
  if (names.length > 64) {
    failures.push(safeGitFailure(
      "remotes",
      "GIT_REMOTE_LIMIT_EXCEEDED",
      "Git repository has more than 64 remotes",
    ));
    names.length = 64;
  }

  const remotes = [];
  for (const name of names) {
    if (/[^\u0021-\u007e]/u.test(name) || name.length > 256) {
      failures.push(safeGitFailure(
        "remotes",
        "GIT_REMOTE_NAME_INVALID",
        "Git returned an unsafe remote name",
      ));
      continue;
    }
    for (const direction of ["fetch", "push"]) {
      const args = ["remote", "get-url"];
      if (direction === "push") args.push("--push");
      args.push("--all", name);
      const result = await runGit(executor, repositoryRoot, args);
      if (result.exit_code !== 0) {
        failures.push(safeGitFailure(
          "remotes",
          gitTimedOut(result) ? "GIT_OBSERVATION_TIMEOUT" : "GIT_REMOTE_URL_FAILED",
          gitTimedOut(result)
            ? "Git observation exceeded its total time limit"
            : `Git ${direction} URL could not be read`,
          { remote: name, direction },
        ));
        if (gitTimedOut(result)) return remotes;
        continue;
      }
      const allUrls = result.stdout.split(/\r?\n/gu).filter(Boolean);
      if (allUrls.length > maximumRemoteUrlsPerDirection) {
        failures.push(safeGitFailure(
          "remotes",
          "GIT_REMOTE_URL_LIMIT_EXCEEDED",
          `Git ${direction} URLs exceeded the retained observation limit`,
          { remote: name, direction },
        ));
      }
      const urls = allUrls.slice(0, maximumRemoteUrlsPerDirection);
      for (const url of urls) {
        try {
          const sanitized = { name, direction, ...sanitizeRemoteUrl(url, repositoryRoot) };
          const duplicate = remotes.find((remote) => (
            remote.name === sanitized.name
            && remote.direction === sanitized.direction
            && remote.url === sanitized.url
          ));
          if (duplicate) {
            duplicate.credentials_redacted ||= sanitized.credentials_redacted;
          } else {
            remotes.push(sanitized);
          }
        } catch {
          failures.push(safeGitFailure(
            "remotes",
            "GIT_REMOTE_URL_INVALID",
            `Git ${direction} URL is unsupported and was not retained`,
            { remote: name, direction },
          ));
        }
      }
    }
  }
  return remotes;
}

async function filterNeutralizers(repositoryRoot, executor, failures) {
  const result = await runGit(executor, repositoryRoot, [
    "config",
    "--no-includes",
    "--null",
    "--name-only",
    "--get-regexp",
    "^filter\\..*\\.(clean|process|required)$",
  ]);
  if (result.exit_code !== 0 && !(result.exit_code === 1 && result.stdout === "")) {
    failures.push(safeGitFailure(
      "status",
      gitTimedOut(result) ? "GIT_OBSERVATION_TIMEOUT" : "GIT_FILTER_DISCOVERY_FAILED",
      gitTimedOut(result)
        ? "Git observation exceeded its total time limit"
        : "Git filter configuration could not be safely bounded",
    ));
    return null;
  }
  const drivers = new Set();
  for (const key of result.stdout.split("\0").filter(Boolean)) {
    const match = key.match(/^filter\.(.+)\.(?:clean|process|required)$/u);
    if (!match || !/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$/u.test(match[1])) {
      failures.push(safeGitFailure(
        "status",
        "GIT_FILTER_DRIVER_INVALID",
        "Git filter configuration contains an unsafe driver name",
      ));
      return null;
    }
    drivers.add(match[1]);
    if (drivers.size > 128) {
      failures.push(safeGitFailure(
        "status",
        "GIT_FILTER_DRIVER_LIMIT_EXCEEDED",
        "Git filter configuration exceeded the safe driver limit",
      ));
      return null;
    }
  }
  const arguments_ = [];
  for (const driver of [...drivers].sort()) {
    arguments_.push(
      "-c",
      `filter.${driver}.clean=`,
      "-c",
      `filter.${driver}.process=`,
      "-c",
      `filter.${driver}.required=false`,
    );
  }
  return arguments_;
}

async function observeSubmoduleBoundary(repositoryRoot, executor, failures) {
  const result = await runGit(executor, repositoryRoot, ["ls-files", "--stage", "-z"]);
  if (result.exit_code !== 0) {
    failures.push(safeGitFailure(
      "status",
      gitTimedOut(result) ? "GIT_OBSERVATION_TIMEOUT" : "GIT_SUBMODULE_CHECK_FAILED",
      gitTimedOut(result)
        ? "Git observation exceeded its total time limit"
        : "Git submodule boundaries could not be safely observed",
    ));
    return null;
  }
  const hasSubmodule = result.stdout.split("\0").some((entry) => entry.startsWith("160000 "));
  if (hasSubmodule) {
    failures.push(safeGitFailure(
      "status",
      "GIT_SUBMODULE_STATE_NOT_OBSERVED",
      "Nested submodule working tree state was not executed or observed",
    ));
  }
  return hasSubmodule;
}

async function inspectGit(canonicalPath, executor, observedAt, budget) {
  const failures = [];
  let expectedMetadataBoundary;
  try {
    expectedMetadataBoundary = await gitMetadataBoundary(canonicalPath, budget);
  } catch (error) {
    const code = error instanceof ProjectInspectionError
      ? error.code
      : "GIT_METADATA_UNREADABLE";
    return {
      repo_root: null,
      observation: {
        status: "partial",
        is_repository: null,
        branch: null,
        detached: null,
        head_sha: null,
        dirty: null,
        upstream: null,
        ahead: null,
        behind: null,
        remotes: [],
        observed_at: observedAt,
        failures: [safeGitFailure(
          "repository",
          code,
          error instanceof ProjectInspectionError
            ? error.message
            : "Git metadata boundary could not be safely inspected",
        )],
      },
    };
  }
  const expectedMetadataGuard = gitMetadataGuardFingerprint(expectedMetadataBoundary);
  const guardedExecutor = async (request) => {
    try {
      const currentBoundary = await gitMetadataBoundary(
        canonicalPath,
        budget,
        { scanTree: false },
      );
      if (gitMetadataGuardFingerprint(currentBoundary) !== expectedMetadataGuard) {
        return {
          exit_code: 1,
          error_code: "GIT_METADATA_CHANGED",
          stdout: "",
          stderr: "",
        };
      }
    } catch (error) {
      return {
        exit_code: 1,
        error_code: error instanceof ProjectInspectionError
          ? error.code
          : "GIT_METADATA_UNREADABLE",
        stdout: "",
        stderr: "",
      };
    }
    return executor(request);
  };
  const repositoryCheck = await runGit(
    guardedExecutor,
    canonicalPath,
    ["rev-parse", "--show-toplevel"],
  );
  if (repositoryCheck.exit_code !== 0) {
    if (/not a git repository/iu.test(repositoryCheck.stderr)) {
      return {
        repo_root: null,
        observation: {
          status: "complete",
          is_repository: false,
          branch: null,
          detached: null,
          head_sha: null,
          dirty: null,
          upstream: null,
          ahead: null,
          behind: null,
          remotes: [],
          observed_at: observedAt,
          failures: [],
        },
      };
    }
    const unavailable = ["ENOENT", "GIT_EXECUTABLE_UNAVAILABLE"].includes(
      repositoryCheck.error_code,
    );
    const timedOut = gitTimedOut(repositoryCheck);
    const unsafeMetadata = typeof repositoryCheck.error_code === "string"
      && /^(?:GIT_METADATA_|GIT_CONFIG_INCLUDE_UNSAFE)/u.test(repositoryCheck.error_code);
    failures.push(safeGitFailure(
      "repository",
      timedOut
        ? "GIT_OBSERVATION_TIMEOUT"
        : unsafeMetadata
          ? repositoryCheck.error_code
        : unavailable
          ? "GIT_UNAVAILABLE"
          : "GIT_REPOSITORY_CHECK_FAILED",
      timedOut
        ? "Git observation exceeded its total time limit"
        : unsafeMetadata
          ? "Git metadata boundary changed or became unsafe during observation"
        : unavailable
        ? "Git executable is unavailable"
        : "Git repository status could not be determined",
    ));
    return {
      repo_root: null,
      observation: {
        status: "partial",
        is_repository: null,
        branch: null,
        detached: null,
        head_sha: null,
        dirty: null,
        upstream: null,
        ahead: null,
        behind: null,
        remotes: [],
        observed_at: observedAt,
        failures,
      },
    };
  }

  let repositoryRoot = null;
  const reportedRoot = repositoryCheck.stdout.trim();
  try {
    if (!reportedRoot || /[\0\r\n]/u.test(reportedRoot) || !path.isAbsolute(reportedRoot)) {
      throw new Error("invalid repository root");
    }
    const candidateRoot = await realpath(reportedRoot);
    const metadataRoot = JSON.parse(expectedMetadataBoundary).root;
    if (
      !containedBy(candidateRoot, canonicalPath)
      || (metadataRoot && !samePath(candidateRoot, metadataRoot))
    ) {
      throw new Error("project path is outside repository root");
    }
    repositoryRoot = candidateRoot;
  } catch {
    failures.push(safeGitFailure(
      "repository",
      "GIT_REPOSITORY_ROOT_INVALID",
      "Git repository root is invalid or does not contain the project path",
    ));
  }

  let statusFields = {
    branch: null,
    detached: null,
    head_sha: null,
    dirty: null,
    upstream: null,
    ahead: null,
    behind: null,
  };
  let remotes = [];
  if (repositoryRoot) {
    const hasSubmodule = await observeSubmoduleBoundary(repositoryRoot, guardedExecutor, failures);
    const neutralizers = await filterNeutralizers(repositoryRoot, guardedExecutor, failures);
    if (neutralizers && !failures.some((failure) => failure.code === "GIT_OBSERVATION_TIMEOUT")) {
      const statusResult = await runGit(
        guardedExecutor,
        repositoryRoot,
        [
          ...neutralizers,
          "status",
          "--porcelain=v2",
          "--branch",
          "--untracked-files=normal",
          "--ignore-submodules=all",
          "--no-renames",
        ],
      );
      if (statusResult.exit_code === 0) {
        statusFields = parseGitStatus(statusResult.stdout, failures);
        if (hasSubmodule && statusFields.dirty === false) statusFields.dirty = null;
      } else {
        failures.push(safeGitFailure(
          "status",
          gitTimedOut(statusResult) ? "GIT_OBSERVATION_TIMEOUT" : "GIT_STATUS_FAILED",
          gitTimedOut(statusResult)
            ? "Git observation exceeded its total time limit"
            : "Git working tree status could not be read after external filters were disabled",
        ));
      }
    }
    if (!failures.some((failure) => failure.code === "GIT_OBSERVATION_TIMEOUT")) {
      remotes = await observeRemotes(repositoryRoot, guardedExecutor, failures);
    }
  }

  try {
    if (await gitMetadataBoundary(canonicalPath, budget) !== expectedMetadataBoundary) {
      throw gitMetadataError(
        "GIT_METADATA_CHANGED",
        "Git metadata boundary changed during observation",
      );
    }
  } catch (error) {
    failures.push(safeGitFailure(
      "repository",
      error instanceof ProjectInspectionError ? error.code : "GIT_METADATA_UNREADABLE",
      error instanceof ProjectInspectionError
        ? error.message
        : "Git metadata boundary could not be revalidated",
    ));
    repositoryRoot = null;
    statusFields = {
      branch: null,
      detached: null,
      head_sha: null,
      dirty: null,
      upstream: null,
      ahead: null,
      behind: null,
    };
    remotes = [];
  }

  return {
    repo_root: repositoryRoot,
    observation: {
      status: failures.length === 0 ? "complete" : "partial",
      is_repository: repositoryRoot ? true : null,
      ...statusFields,
      remotes,
      observed_at: observedAt,
      failures: boundedFailures(failures, "git"),
    },
  };
}

function rulePurpose(relativePath) {
  const normalized = relativePath.replaceAll("\\", "/").toLowerCase();
  if (normalized === ".github/copilot-instructions.md") {
    return "Assistant working instructions";
  }
  if (normalized === ".codex/project_controller.md") {
    return "Project control and routing rules";
  }
  const basename = path.posix.basename(normalized);
  const exact = rulePurposes.get(basename);
  if (exact) return exact;
  if (/(?:^|[_-])rules?(?:[_-]|\.md$)/u.test(basename)) {
    return "Project rules and validation constraints";
  }
  return null;
}

function normalizedRuleLimits(overrides = {}) {
  const values = {
    maximumFileBytes: overrides.maximumFileBytes ?? maximumRuleBytes,
    maximumTotalBytes: overrides.maximumTotalBytes ?? maximumRuleTotalBytes,
    maximumDocuments: overrides.maximumDocuments ?? maximumRuleDocuments,
    maximumDurationMs: overrides.maximumDurationMs ?? maximumScanDurationMs,
  };
  for (const [name, value] of Object.entries(values)) {
    if (!Number.isSafeInteger(value) || value < 1) {
      throw new TypeError(`${name} must be a positive safe integer`);
    }
  }
  if (values.maximumDocuments > maximumRuleDocuments) {
    throw new TypeError(`maximumDocuments cannot exceed ${maximumRuleDocuments}`);
  }
  if (values.maximumFileBytes > maximumRuleBytes) {
    throw new TypeError(`maximumFileBytes cannot exceed ${maximumRuleBytes}`);
  }
  if (values.maximumTotalBytes > maximumRuleTotalBytes) {
    throw new TypeError(`maximumTotalBytes cannot exceed ${maximumRuleTotalBytes}`);
  }
  return values;
}

async function readBoundedHandle(handle, maximumBytes) {
  const chunks = [];
  let total = 0;
  while (total <= maximumBytes) {
    const remaining = maximumBytes + 1 - total;
    const buffer = Buffer.allocUnsafe(Math.min(64 * 1024, remaining));
    const { bytesRead } = await handle.read(buffer, 0, buffer.length, null);
    if (bytesRead === 0) break;
    if (!Number.isSafeInteger(bytesRead) || bytesRead < 0 || bytesRead > buffer.length) {
      throw new Error("rule file handle returned an invalid read length");
    }
    chunks.push(buffer.subarray(0, bytesRead));
    total += bytesRead;
  }
  return { content: Buffer.concat(chunks, total), exceeded: total > maximumBytes };
}

async function hashRuleDocument(
  root,
  absolutePath,
  relativePath,
  purpose,
  observedAt,
  failures,
  limits,
  budget,
  fileOpener,
) {
  let before;
  let handle;
  try {
    before = await lstat(absolutePath, { bigint: true });
    if (before.isSymbolicLink()) {
      failures.push({
        stage: "rules",
        code: "RULE_PATH_REDIRECTED",
        message: "A rule document symbolic link or junction was not followed",
        relative_path: relativePath,
      });
      return null;
    }
    if (!before.isFile()) return null;
    if (before.size > BigInt(limits.maximumFileBytes)) {
      failures.push({
        stage: "rules",
        code: "RULE_DOCUMENT_TOO_LARGE",
        message: "A rule document exceeded the configured hashing limit",
        relative_path: relativePath,
      });
      return null;
    }
    const canonical = await realpath(absolutePath);
    if (!containedBy(root, canonical)) {
      failures.push({
        stage: "rules",
        code: "RULE_PATH_ESCAPE",
        message: "A rule document resolved outside the project root and was not read",
        relative_path: relativePath,
      });
      return null;
    }
    if (budget.totalBytes + Number(before.size) > limits.maximumTotalBytes) {
      budget.exhausted = true;
      failures.push({
        stage: "rules",
        code: "RULE_TOTAL_BYTES_EXCEEDED",
        message: "Rule documents exceeded the aggregate hashing byte limit",
        relative_path: relativePath,
      });
      return null;
    }
    handle = await fileOpener(absolutePath, "r");
    const opened = await handle.stat({ bigint: true });
    if (
      !sameFileIdentity(before, opened)
      || before.size !== opened.size
      || before.mtimeMs !== opened.mtimeMs
    ) {
      failures.push({
        stage: "rules",
        code: "RULE_DOCUMENT_CHANGED",
        message: "A rule document changed while it was being opened",
        relative_path: relativePath,
      });
      return null;
    }
    if (opened.size > BigInt(limits.maximumFileBytes)) {
      failures.push({
        stage: "rules",
        code: "RULE_DOCUMENT_TOO_LARGE",
        message: "A rule document exceeded the configured hashing limit",
        relative_path: relativePath,
      });
      return null;
    }
    const { content, exceeded } = await readBoundedHandle(handle, limits.maximumFileBytes);
    if (exceeded) {
      failures.push({
        stage: "rules",
        code: "RULE_DOCUMENT_TOO_LARGE",
        message: "A rule document grew beyond the configured hashing limit",
        relative_path: relativePath,
      });
      return null;
    }
    const after = await handle.stat({ bigint: true });
    const [afterLink, afterCanonical] = await Promise.all([
      lstat(absolutePath, { bigint: true }),
      realpath(absolutePath),
    ]);
    if (
      afterLink.isSymbolicLink()
      || !samePath(canonical, afterCanonical)
      || !sameFileIdentity(opened, after)
      || before.size !== after.size
      || before.mtimeMs !== after.mtimeMs
      || before.dev !== after.dev
      || before.ino !== after.ino
    ) {
      failures.push({
        stage: "rules",
        code: "RULE_DOCUMENT_CHANGED",
        message: "A rule document changed while its hash was being calculated",
        relative_path: relativePath,
      });
      return null;
    }
    budget.totalBytes += content.length;
    return {
      relative_path: relativePath,
      sha256: createHash("sha256").update(content).digest("hex"),
      observed_at: observedAt,
      purpose,
    };
  } catch (error) {
    failures.push({
      stage: "rules",
      code: error?.code === "EACCES" ? "RULE_DOCUMENT_UNREADABLE" : "RULE_DOCUMENT_FAILED",
      message: "A rule document could not be inspected",
      relative_path: relativePath,
    });
    return null;
  } finally {
    await handle?.close().catch(() => {});
  }
}

async function inspectRules(root, observedAt, {
  ruleLimits,
  monotonicClock,
  ruleFileOpener,
}) {
  const limits = normalizedRuleLimits(ruleLimits);
  const documents = [];
  const failures = [];
  const budget = { totalBytes: 0, exhausted: false };
  const presentRootFiles = new Set();
  let scannedEntries = 0;
  let scanLimitReached = false;
  let depthLimitReported = false;
  let timeLimitReported = false;
  const startedAt = monotonicClock();

  for (const standardDocument of standardRuleDocuments) {
    try {
      await lstat(path.join(root, standardDocument), { bigint: true });
      presentRootFiles.add(standardDocument.toLowerCase());
    } catch (error) {
      if (error?.code === "ENOENT") continue;
      failures.push({
        stage: "rules",
        code: "RULE_STANDARD_DOCUMENT_UNREADABLE",
        message: "A standard rule document could not be checked for existence",
        relative_path: standardDocument,
      });
      presentRootFiles.add(standardDocument.toLowerCase());
    }
  }

  function timeLimitReached(relativePath) {
    if (monotonicClock() - startedAt <= limits.maximumDurationMs) return false;
    if (!timeLimitReported) {
      timeLimitReported = true;
      failures.push({
        stage: "rules",
        code: "RULE_SCAN_TIMEOUT",
        message: "Rule discovery exceeded its configured time limit",
        relative_path: relativePath || ".",
      });
    }
    scanLimitReached = true;
    return true;
  }

  async function visit(directory, relativeDirectory, depth) {
    if (depth > maximumScanDepth) {
      if (!depthLimitReported) {
        depthLimitReported = true;
        failures.push({
          stage: "rules",
          code: "RULE_SCAN_DEPTH_EXCEEDED",
          message: "Rule discovery stopped at the configured directory depth",
          relative_path: relativeDirectory || ".",
        });
      }
      return;
    }
    if (scanLimitReached) return;
    let entries;
    try {
      entries = await opendir(directory);
    } catch {
      failures.push({
        stage: "rules",
        code: "RULE_DIRECTORY_UNREADABLE",
        message: "A project directory could not be scanned for rule documents",
        relative_path: relativeDirectory || ".",
      });
      return;
    }
    for await (const entry of entries) {
      if (timeLimitReached(relativeDirectory)) return;
      scannedEntries += 1;
      if (scannedEntries > maximumScanEntries) {
        scanLimitReached = true;
        failures.push({
          stage: "rules",
          code: "RULE_SCAN_LIMIT_EXCEEDED",
          message: "Rule discovery stopped after 25000 filesystem entries",
          relative_path: relativeDirectory || ".",
        });
        return;
      }
      const relativePath = relativeDirectory
        ? path.posix.join(relativeDirectory, entry.name)
        : entry.name;
      const absolutePath = path.join(directory, entry.name);
      if (!relativeDirectory) presentRootFiles.add(entry.name.toLowerCase());

      if (entry.isSymbolicLink()) {
        failures.push({
          stage: "rules",
          code: "RULE_PATH_REDIRECTED",
          message: "A symbolic link or junction was not followed during rule discovery",
          relative_path: relativePath,
        });
        continue;
      }
      if (entry.isDirectory()) {
        if (rulePurpose(relativePath)) {
          failures.push({
            stage: "rules",
            code: "RULE_DOCUMENT_NOT_FILE",
            message: "An authoritative document path is not a regular file",
            relative_path: relativePath,
          });
        }
        if (!ignoredDirectories.has(entry.name.toLowerCase())) {
          await visit(absolutePath, relativePath.replaceAll("\\", "/"), depth + 1);
        }
        continue;
      }
      if (!entry.isFile()) continue;
      const purpose = rulePurpose(relativePath);
      if (!purpose) continue;
      if (documents.length >= limits.maximumDocuments) {
        failures.push({
          stage: "rules",
          code: "RULE_DOCUMENT_LIMIT_EXCEEDED",
          message: "Rule discovery reached the retained document limit",
          relative_path: relativePath,
        });
        scanLimitReached = true;
        return;
      }
      const document = await hashRuleDocument(
        root,
        absolutePath,
        relativePath.replaceAll("\\", "/"),
        purpose,
        observedAt,
        failures,
        limits,
        budget,
        ruleFileOpener,
      );
      if (document) documents.push(document);
      if (budget.exhausted) {
        scanLimitReached = true;
        return;
      }
    }
  }

  await visit(root, "", 0);
  documents.sort((left, right) => left.relative_path.localeCompare(right.relative_path, "en"));
  return {
    status: failures.length === 0 ? "complete" : "partial",
    observed_at: observedAt,
    documents,
    missing_standard_documents: standardRuleDocuments.filter(
      (name) => !presentRootFiles.has(name.toLowerCase()),
    ),
    failures: boundedFailures(failures, "rules"),
  };
}

export async function inspectProject(rootPath, {
  gitExecutor = null,
  clock = () => new Date(),
  ruleLimits = {},
  monotonicClock = () => performance.now(),
  ruleFileOpener = open,
  maximumGitDurationMs = maximumGitObservationMs,
  gitMonotonicClock = () => performance.now(),
} = {}) {
  const observedAt = safeObservationTime(clock);
  const projectDirectory = await canonicalProjectDirectory(rootPath);
  const canonicalPath = projectDirectory.canonical_path;
  const wsl2ProjectPath = parseWsl2ProjectPath(canonicalPath);
  const effectiveGitExecutor = gitExecutor
    ?? (wsl2ProjectPath === null
      ? defaultGitExecutor
      : createWsl2ProjectGitExecutor(wsl2ProjectPath));
  const budget = gitObservationBudget({
    maximumDurationMs: maximumGitDurationMs,
    monotonicClock: gitMonotonicClock,
  });
  const boundedGitExecutor = gitObservationExecutor(effectiveGitExecutor, budget);
  const [gitResult, rules] = await Promise.all([
    inspectGit(canonicalPath, boundedGitExecutor, observedAt, budget),
    inspectRules(canonicalPath, observedAt, {
      ruleLimits,
      monotonicClock,
      ruleFileOpener,
    }),
  ]);
  await verifyProjectDirectoryIdentity(projectDirectory);
  return {
    canonical_path: canonicalPath,
    repo_root: gitResult.repo_root,
    git: gitResult.observation,
    rules,
  };
}
