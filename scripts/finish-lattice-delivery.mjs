import { spawnSync } from "node:child_process";
import {
  lstat,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  rmdir,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const exporterPath = path.join(scriptDirectory, "export-lattice-engineering-status.mjs");
const pushPolicies = new Set([
  "authorized_non_force_feature_branch",
  "local_only",
]);
const archivePolicies = new Set(["after_success", "keep_open"]);
const terminalTicketStatuses = new Set([
  "blocked",
  "complete",
  "completed",
  "failed",
  "fail",
  "partial",
  "paused",
  "verified",
  "waiting_dependency",
]);
const successfulTerminalTicketStatuses = new Set([
  "complete",
  "completed",
  "verified",
]);
const dashboardOwnershipMarker = ".lattice-engineering-status-owned";
const dashboardOwnershipContent = "lattice.engineering-status-output/1\n";
const disabledHooksPath = process.platform === "win32" ? "NUL" : "/dev/null";
const dashboardOwnedNames = new Set([
  dashboardOwnershipMarker,
  "index.html",
  "status.json",
]);

class DeliveryError extends Error {
  constructor(code, message, options = {}) {
    super(message, options);
    this.name = "DeliveryError";
    this.code = code;
  }
}

function commandFailure(code, message) {
  return new DeliveryError(code, message);
}

function run(command, args, cwd) {
  const environment = command === "git"
    ? {
        ...process.env,
        GCM_INTERACTIVE: "Never",
        GIT_TERMINAL_PROMPT: "0",
      }
    : process.env;
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    env: environment,
    timeout: 120_000,
    windowsHide: true,
  });
  if (result.error || result.status !== 0) {
    return {
      ok: false,
      status: result.status,
      error: result.error,
      stdout: result.stdout || "",
      stderr: result.stderr || "",
    };
  }
  return {
    ok: true,
    status: result.status,
    stdout: result.stdout || "",
    stderr: result.stderr || "",
  };
}

function git(repository, args, failureCode, failureMessage) {
  const result = run(
    "git",
    ["-c", `core.hooksPath=${disabledHooksPath}`, ...args],
    repository,
  );
  if (!result.ok) {
    throw commandFailure(failureCode, failureMessage);
  }
  return result.stdout.trim();
}

function parseFrontmatter(content) {
  const match = String(content).match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/u);
  if (!match) {
    throw commandFailure("TICKET_FRONTMATTER_INVALID", "ticket frontmatter is invalid");
  }
  const values = {};
  for (const line of match[1].split(/\r?\n/gu)) {
    const entry = line.match(/^([a-zA-Z0-9_]+):\s*(.*?)\s*$/u);
    if (!entry) {
      continue;
    }
    if (Object.hasOwn(values, entry[1])) {
      throw commandFailure(
        "TICKET_FRONTMATTER_DUPLICATE",
        `ticket frontmatter duplicates ${entry[1]}`,
      );
    }
    values[entry[1]] = entry[2].replace(/^['"]|['"]$/gu, "");
  }
  return values;
}

async function findTicket(repository, branch, expectedTaskId, sourceHead) {
  const entries = git(
    repository,
    ["ls-tree", "-r", "--name-only", sourceHead, "--", "docs/tickets"],
    "TICKET_DIRECTORY_MISSING",
    "committed TASK ticket directory is unavailable",
  ).split(/\r?\n/gu).filter(Boolean);
  const matches = [];
  for (const entry of entries) {
    const name = path.posix.basename(entry.replaceAll("\\", "/"));
    if (!/^TASK-[0-9]{3}-.+\.md$/iu.test(name)) {
      continue;
    }
    const content = git(
      repository,
      ["show", `${sourceHead}:${entry.replaceAll("\\", "/")}`],
      "TICKET_READ_FAILED",
      "committed TASK ticket cannot be read",
    );
    let metadata;
    try {
      metadata = parseFrontmatter(content);
    } catch (error) {
      if (new RegExp(`^branch:\\s*${escapeRegularExpression(branch)}\\s*$`, "mu").test(content)) {
        throw error;
      }
      continue;
    }
    if (metadata.branch === branch) {
      matches.push({ file: name, metadata });
    }
  }
  if (matches.length !== 1) {
    throw commandFailure(
      matches.length === 0 ? "TICKET_NOT_FOUND" : "TICKET_AMBIGUOUS",
      matches.length === 0
        ? "exactly one TASK ticket must match the current branch"
        : "multiple TASK tickets match the current branch",
    );
  }
  const ticket = matches[0];
  if (!/^TASK-[0-9]{3}$/u.test(ticket.metadata.ticket_id || "")) {
    throw commandFailure(
      "TICKET_ID_INVALID",
      "ticket_id must name one canonical TASK ticket",
    );
  }
  if (expectedTaskId && ticket.metadata.ticket_id !== expectedTaskId) {
    throw commandFailure(
      "TICKET_ID_INVALID",
      "ticket_id must match the current TASK feature branch",
    );
  }
  const normalizedStatus = String(ticket.metadata.status || "")
    .trim()
    .toLowerCase()
    .replaceAll(/[-\s]+/gu, "_");
  if (!terminalTicketStatuses.has(normalizedStatus)) {
    throw commandFailure(
      "TICKET_NOT_TERMINAL",
      "TASK ticket must be terminal before delivery",
    );
  }
  if (!pushPolicies.has(ticket.metadata.delivery_push)) {
    throw commandFailure(
      "PUSH_POLICY_INVALID",
      "delivery_push must explicitly be authorized_non_force_feature_branch or local_only",
    );
  }
  if (!archivePolicies.has(ticket.metadata.delivery_archive)) {
    throw commandFailure(
      "ARCHIVE_POLICY_INVALID",
      "delivery_archive must explicitly be after_success or keep_open",
    );
  }
  if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/u.test(ticket.metadata.delivery_remote || "")) {
    throw commandFailure(
      "REMOTE_POLICY_INVALID",
      "delivery_remote must be one safe named Git remote",
    );
  }
  return { ...ticket, normalizedStatus };
}

function parseTicketDependencies(metadata) {
  if (!Object.hasOwn(metadata, "depends_on")) {
    return [];
  }
  const match = String(metadata.depends_on).match(/^\[(.*?)\]$/u);
  if (!match) {
    throw commandFailure("DEPENDENCY_INVALID", "depends_on must be one canonical TASK list");
  }
  const dependencies = match[1].trim() === ""
    ? []
    : match[1].split(",").map((value) => value.trim());
  if (
    dependencies.some((dependency) => !/^TASK-[0-9]{3}$/u.test(dependency)) ||
    new Set(dependencies).size !== dependencies.length
  ) {
    throw commandFailure("DEPENDENCY_INVALID", "depends_on must be one unique canonical TASK list");
  }
  return dependencies;
}

async function verifyTicketDependencies(repository, ticket, sourceHead) {
  const dependencies = parseTicketDependencies(ticket.metadata);
  if (dependencies.length === 0) {
    return;
  }
  if (dependencies.includes(ticket.metadata.ticket_id)) {
    throw commandFailure("DEPENDENCY_INVALID", "TASK ticket cannot depend on itself");
  }
  const entries = git(
    repository,
    ["ls-tree", "-r", "--name-only", sourceHead, "--", "docs/tickets"],
    "TICKET_DIRECTORY_MISSING",
    "committed TASK ticket directory is unavailable",
  ).split(/\r?\n/gu).filter(Boolean);
  for (const dependency of dependencies) {
    const matches = [];
    for (const entry of entries) {
      const name = path.posix.basename(entry.replaceAll("\\", "/"));
      if (!/^TASK-[0-9]{3}-.+\.md$/iu.test(name)) {
        continue;
      }
      const content = git(
        repository,
        ["show", `${sourceHead}:${entry.replaceAll("\\", "/")}`],
        "TICKET_READ_FAILED",
        "committed TASK ticket cannot be read",
      );
      let metadata;
      try {
        metadata = parseFrontmatter(content);
      } catch {
        continue;
      }
      if (metadata.ticket_id === dependency) {
        matches.push(metadata);
      }
    }
    if (matches.length !== 1) {
      throw commandFailure(
        "DEPENDENCY_UNRESOLVED",
        "declared TASK dependency must resolve exactly once",
      );
    }
    const dependencyStatus = String(matches[0].status || "")
      .trim()
      .toLowerCase()
      .replaceAll(/[-\s]+/gu, "_");
    if (!successfulTerminalTicketStatuses.has(dependencyStatus)) {
      throw commandFailure(
        "DEPENDENCY_NOT_TERMINAL",
        "declared TASK dependency must be successfully terminal",
      );
    }
  }
}

async function findIssueEvidence(repository, branch, expectedIssueId, sourceHead) {
  const entries = git(
    repository,
    ["ls-tree", "-r", "--name-only", sourceHead, "--", "docs/issues"],
    "ISSUE_DIRECTORY_MISSING",
    "committed ISSUE evidence directory is unavailable",
  ).split(/\r?\n/gu).filter(Boolean);
  const byIdentity = [];
  const byBranch = [];
  for (const entry of entries) {
    const name = path.posix.basename(entry.replaceAll("\\", "/"));
    const filenameIdentity = name.match(/^ISSUE-([0-9]{3})-.+\.md$/iu);
    if (!filenameIdentity) {
      continue;
    }
    const content = git(
      repository,
      ["show", `${sourceHead}:${entry.replaceAll("\\", "/")}`],
      "ISSUE_READ_FAILED",
      "committed ISSUE evidence cannot be read",
    );
    let metadata;
    try {
      metadata = parseFrontmatter(content);
    } catch (error) {
      if (
        new RegExp(`^issue_id:\\s*${escapeRegularExpression(expectedIssueId)}\\s*$`, "mu").test(content) ||
        new RegExp(`^branch:\\s*${escapeRegularExpression(branch)}\\s*$`, "mu").test(content)
      ) {
        throw error;
      }
      continue;
    }
    const fileIssueId = `ISSUE-${filenameIdentity[1]}`;
    if (metadata.issue_id !== fileIssueId) {
      if (
        metadata.issue_id === expectedIssueId ||
        fileIssueId === expectedIssueId ||
        metadata.branch === branch
      ) {
        throw commandFailure(
          "ISSUE_ID_MISMATCH",
          "ISSUE evidence issue_id must match its filename and issue branch number",
        );
      }
      continue;
    }
    const evidence = { file: name, metadata };
    if (metadata.issue_id === expectedIssueId) {
      byIdentity.push(evidence);
    }
    if (metadata.branch === branch) {
      byBranch.push(evidence);
    }
  }
  if (byIdentity.length !== 1) {
    if (byIdentity.length === 0 && byBranch.length > 0) {
      throw commandFailure(
        "ISSUE_ID_MISMATCH",
        "ISSUE evidence issue_id must match the issue branch number",
      );
    }
    throw commandFailure(
      byIdentity.length === 0 ? "ISSUE_EVIDENCE_NOT_FOUND" : "ISSUE_EVIDENCE_AMBIGUOUS",
      byIdentity.length === 0
        ? "exactly one committed ISSUE evidence record must match the issue branch"
        : "multiple committed ISSUE evidence records match the issue identity",
    );
  }
  const issue = byIdentity[0];
  if (issue.metadata.branch !== branch) {
    throw commandFailure("ISSUE_BRANCH_MISMATCH", "ISSUE evidence branch must match the current branch");
  }
  if (byBranch.length !== 1 || byBranch[0].metadata.issue_id !== expectedIssueId) {
    throw commandFailure("ISSUE_ID_MISMATCH", "ISSUE evidence issue_id must match the issue branch number");
  }
  const normalizedStatus = String(issue.metadata.status || "")
    .trim()
    .toLowerCase()
    .replaceAll(/[-\s]+/gu, "_");
  if (!terminalTicketStatuses.has(normalizedStatus)) {
    throw commandFailure("ISSUE_NOT_TERMINAL", "ISSUE evidence must be terminal before delivery");
  }
  if (!pushPolicies.has(issue.metadata.delivery_push)) {
    throw commandFailure(
      "PUSH_POLICY_INVALID",
      "delivery_push must explicitly be authorized_non_force_feature_branch or local_only",
    );
  }
  if (!archivePolicies.has(issue.metadata.delivery_archive)) {
    throw commandFailure(
      "ARCHIVE_POLICY_INVALID",
      "delivery_archive must explicitly be after_success or keep_open",
    );
  }
  if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/u.test(issue.metadata.delivery_remote || "")) {
    throw commandFailure(
      "REMOTE_POLICY_INVALID",
      "delivery_remote must be one safe named Git remote",
    );
  }
  return { ...issue, normalizedStatus };
}

function parseDeliveryBranch(branch) {
  const task = branch.match(/^feature\/task-([0-9]{3})-[a-z0-9]+(?:-[a-z0-9]+)*$/u);
  if (task) {
    return { kind: "TASK", expectedTaskId: `TASK-${task[1]}` };
  }
  const issue = branch.match(/^feature\/issue-([0-9]{3})-[a-z0-9]+(?:-[a-z0-9]+)*$/u);
  if (issue) {
    return { kind: "ISSUE", expectedIssueId: `ISSUE-${issue[1]}` };
  }
  throw commandFailure(
    "TASK_BRANCH_INVALID",
    "delivery requires a feature/task-nnn-* or feature/issue-nnn-* branch",
  );
}

function escapeRegularExpression(value) {
  return String(value).replaceAll(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

function defaultOutputDirectory() {
  const applicationData =
    process.env.LOCALAPPDATA || path.join(os.homedir(), ".local", "share");
  return path.join(applicationData, "LATTICE", "engineering-status");
}

async function defaultRefresh({ repository, outputDirectory }) {
  const result = run(
    process.execPath,
    [
      exporterPath,
      "--repository",
      repository,
      "--output",
      outputDirectory,
    ],
    repository,
  );
  if (!result.ok) {
    throw commandFailure("DASHBOARD_REFRESH_FAILED", "engineering dashboard refresh failed");
  }
}

function normalizeError(error) {
  if (error instanceof DeliveryError) {
    return error;
  }
  return new DeliveryError("UNEXPECTED_FAILURE", error?.message || "unexpected delivery failure", {
    cause: error,
  });
}

function remoteDefaultBranch(repository, remote) {
  const live = run("git", ["ls-remote", "--symref", remote, "HEAD"], repository);
  if (live.ok) {
    const match = live.stdout.match(/^ref:\s+refs\/heads\/(\S+)\s+HEAD$/mu);
    if (match) {
      return match[1];
    }
  }
  throw commandFailure("DEFAULT_BRANCH_UNKNOWN", "Git default branch cannot be verified");
}

function oneRemoteUrl(repository, remote, mode) {
  const args = ["remote", "get-url"];
  if (mode === "push") {
    args.push("--push");
  }
  args.push("--all", remote);
  const urls = git(
    repository,
    args,
    "REMOTE_URL_READ_FAILED",
    "Git remote endpoint cannot be read",
  )
    .split(/\r?\n/gu)
    .filter(Boolean);
  if (urls.length !== 1 || /[\r\n]/u.test(urls[0])) {
    throw commandFailure(
      "REMOTE_URL_AMBIGUOUS",
      `delivery remote must have exactly one ${mode} endpoint`,
    );
  }
  return urls[0];
}

function canonicalRemoteEndpoint(repository, remoteUrl) {
  const raw = String(remoteUrl).trim();
  const isWindowsPath = /^[a-zA-Z]:[\\/]/u.test(raw) || raw.startsWith("\\\\");
  const isRelativePath = raw.startsWith("./") || raw.startsWith("../") ||
    raw.startsWith(".\\") || raw.startsWith("..\\");
  if (isWindowsPath || isRelativePath || path.isAbsolute(raw)) {
    const normalizedPath = path.resolve(repository, raw).replaceAll("\\", "/");
    return `file:${process.platform === "win32" ? normalizedPath.toLowerCase() : normalizedPath}`;
  }

  const scpStyle = raw.includes("://")
    ? null
    : raw.match(/^(?:[^@/\s]+@)?([^:/\s]+):(.+)$/u);
  if (scpStyle) {
    const host = scpStyle[1].toLowerCase();
    const repositoryPath = scpStyle[2].replace(/^\/+|\/+$/gu, "").replace(/\.git$/iu, "");
    return `${host}/${host === "github.com" ? repositoryPath.toLowerCase() : repositoryPath}`;
  }

  try {
    const parsed = new URL(raw);
    if (parsed.username || parsed.password || parsed.search || parsed.hash) {
      throw new Error("remote URL contains credentials or mutable URL components");
    }
    if (parsed.protocol === "file:") {
      const normalizedPath = path.resolve(fileURLToPath(parsed)).replaceAll("\\", "/");
      return `file:${process.platform === "win32" ? normalizedPath.toLowerCase() : normalizedPath}`;
    }
    const host = parsed.hostname.toLowerCase();
    const port = parsed.port ? `:${parsed.port}` : "";
    const repositoryPath = decodeURIComponent(parsed.pathname)
      .replace(/^\/+|\/+$/gu, "")
      .replace(/\.git$/iu, "");
    if (!host || !repositoryPath) {
      throw new Error("missing host or repository path");
    }
    return `${host}${port}/${host === "github.com" ? repositoryPath.toLowerCase() : repositoryPath}`;
  } catch {
    throw commandFailure("REMOTE_URL_INVALID", "Git remote endpoint is unsupported");
  }
}

function captureRemoteIdentity(repository, remote, declaredRepository) {
  const fetchUrl = oneRemoteUrl(repository, remote, "fetch");
  const pushUrl = oneRemoteUrl(repository, remote, "push");
  const fetchEndpoint = canonicalRemoteEndpoint(repository, fetchUrl);
  const pushEndpoint = canonicalRemoteEndpoint(repository, pushUrl);
  if (fetchEndpoint !== pushEndpoint) {
    throw commandFailure(
      "REMOTE_ENDPOINT_SPLIT",
      "delivery remote fetch and push endpoint must be identical",
    );
  }
  if (declaredRepository && fetchEndpoint !== declaredRepository) {
    throw commandFailure(
      "REMOTE_IDENTITY_MISMATCH",
      "delivery remote does not match the authorized repository identity",
    );
  }
  return { fetchEndpoint, fetchUrl, pushUrl };
}

function sameRemoteIdentity(left, right) {
  return left.fetchEndpoint === right.fetchEndpoint &&
    left.fetchUrl === right.fetchUrl &&
    left.pushUrl === right.pushUrl;
}

function remoteBranchHead(repository, endpoint, branch, phase = "remote") {
  const output = git(
    repository,
    ["ls-remote", "--heads", endpoint, `refs/heads/${branch}`],
    phase === "final" ? "FINAL_REMOTE_READ_FAILED" : "REMOTE_READ_FAILED",
    `${phase} branch head cannot be read`,
  );
  const lines = output.split(/\r?\n/gu).filter(Boolean);
  return lines.length === 1 ? lines[0].split(/\s+/u)[0] : null;
}

function verifyUpstream(repository, remote, branch, expectedHead, phase) {
  const upstreamName = git(
    repository,
    ["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
    "UPSTREAM_READ_FAILED",
    `Git upstream cannot be read ${phase}`,
  );
  const upstreamHead = git(
    repository,
    ["rev-parse", "@{upstream}"],
    "UPSTREAM_READ_FAILED",
    `Git upstream head cannot be read ${phase}`,
  );
  if (upstreamName !== `${remote}/${branch}` || upstreamHead !== expectedHead) {
    throw commandFailure("UPSTREAM_MISMATCH", `Git upstream does not match ${phase}`);
  }
}

async function canonicalizeCandidate(candidate) {
  const missing = [];
  let cursor = path.resolve(candidate);
  while (true) {
    try {
      const existing = await realpath(cursor);
      return path.join(existing, ...missing);
    } catch (error) {
      if (error?.code !== "ENOENT") {
        throw error;
      }
      const parent = path.dirname(cursor);
      if (parent === cursor) {
        throw error;
      }
      missing.unshift(path.basename(cursor));
      cursor = parent;
    }
  }
}

async function pathExists(candidate) {
  try {
    await lstat(candidate);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") {
      return false;
    }
    throw error;
  }
}

async function validateOwnedOutput(candidate, { allowEmpty = false, allowLegacy = false } = {}) {
  const details = await lstat(candidate);
  if (details.isSymbolicLink() || !details.isDirectory()) {
    throw commandFailure("OUTPUT_NOT_OWNED", "dashboard output is not an owned directory");
  }
  const entries = await readdir(candidate);
  if (entries.length === 0 && allowEmpty) {
    return;
  }
  if (entries.some((entry) => !dashboardOwnedNames.has(entry))) {
    throw commandFailure("OUTPUT_NOT_OWNED", "dashboard output contains unowned data");
  }
  for (const entry of entries) {
    const entryDetails = await lstat(path.join(candidate, entry));
    if (entryDetails.isSymbolicLink() || !entryDetails.isFile()) {
      throw commandFailure("OUTPUT_NOT_OWNED", "dashboard output contains an unsafe entry");
    }
  }
  if (entries.includes(dashboardOwnershipMarker)) {
    const marker = await readFile(path.join(candidate, dashboardOwnershipMarker), "utf8");
    if (marker !== dashboardOwnershipContent) {
      throw commandFailure("OUTPUT_NOT_OWNED", "dashboard ownership marker is invalid");
    }
    return;
  }
  if (
    allowLegacy &&
    entries.length === 2 &&
    entries.includes("index.html") &&
    entries.includes("status.json")
  ) {
    return;
  }
  throw commandFailure("OUTPUT_NOT_OWNED", "dashboard output ownership is not proven");
}

async function removeOwnedOutput(candidate) {
  await validateOwnedOutput(candidate, { allowEmpty: true, allowLegacy: true });
  const entries = await readdir(candidate);
  for (const entry of entries) {
    await rm(path.join(candidate, entry), { force: true });
  }
  await rmdir(candidate);
}

async function refreshSafely({
  canonicalOutput,
  canonicalRepository,
  refresh,
  repository,
  resolvedOutput,
}) {
  const outputParent = path.dirname(canonicalOutput);
  const canonicalParent = await realpath(outputParent);
  if (
    pathIsInside(canonicalRepository, canonicalParent) ||
    pathIsInside(canonicalOutput, canonicalRepository)
  ) {
    throw commandFailure(
      "OUTPUT_INSIDE_REPOSITORY",
      "dashboard output parent must remain outside the source repository",
    );
  }
  const staging = await mkdtemp(
    path.join(canonicalParent, `.${path.basename(canonicalOutput)}.staging-`),
  );
  const backup = `${staging}.previous`;
  let movedPrevious = false;
  let published = false;
  try {
    if (await pathExists(canonicalOutput)) {
      await validateOwnedOutput(canonicalOutput, { allowEmpty: true, allowLegacy: true });
    }
    await refresh({ repository, outputDirectory: staging });
    await validateOwnedOutput(staging, { allowLegacy: true });
    await writeFile(
      path.join(staging, dashboardOwnershipMarker),
      dashboardOwnershipContent,
      { encoding: "utf8", flag: "wx" },
    );
    const observedOutput = await canonicalizeCandidate(resolvedOutput);
    if (
      pathIsInside(canonicalRepository, observedOutput) ||
      path.relative(canonicalOutput, observedOutput) !== ""
    ) {
      throw commandFailure(
        "OUTPUT_PATH_CHANGED",
        "dashboard output changed during refresh",
      );
    }
    if (await pathExists(canonicalOutput)) {
      await rename(canonicalOutput, backup);
      movedPrevious = true;
      await validateOwnedOutput(backup, { allowEmpty: true, allowLegacy: true });
    }
    await rename(staging, canonicalOutput);
    published = true;
    const publishedOutput = await realpath(canonicalOutput);
    if (pathIsInside(canonicalRepository, publishedOutput)) {
      throw commandFailure(
        "OUTPUT_INSIDE_REPOSITORY",
        "published dashboard must remain outside the source repository",
      );
    }
    if (movedPrevious) {
      await removeOwnedOutput(backup);
      movedPrevious = false;
    }
  } catch (error) {
    if (published && await pathExists(canonicalOutput)) {
      await removeOwnedOutput(canonicalOutput);
      published = false;
    }
    if (movedPrevious && !await pathExists(canonicalOutput)) {
      await rename(backup, canonicalOutput);
      movedPrevious = false;
    }
    throw error;
  } finally {
    if (await pathExists(staging)) {
      await removeOwnedOutput(staging);
    }
    if (movedPrevious && await pathExists(backup)) {
      await removeOwnedOutput(backup);
    }
  }
}

function pathIsInside(parent, candidate) {
  const relativePath = path.relative(parent, candidate);
  return (
    relativePath === "" ||
    (!relativePath.startsWith(`..${path.sep}`) &&
      relativePath !== ".." &&
      !path.isAbsolute(relativePath))
  );
}

function verifyLocalSnapshot(repository, expectedBranch, expectedHead, phase) {
  const branch = git(
    repository,
    ["branch", "--show-current"],
    "SNAPSHOT_READ_FAILED",
    `Git branch cannot be read ${phase}`,
  );
  const head = git(
    repository,
    ["rev-parse", "HEAD"],
    "SNAPSHOT_READ_FAILED",
    `Git HEAD cannot be read ${phase}`,
  );
  const status = git(
    repository,
    ["status", "--porcelain=v1", "--untracked-files=normal"],
    "SNAPSHOT_READ_FAILED",
    `Git worktree status cannot be read ${phase}`,
  );
  if (branch !== expectedBranch || head !== expectedHead || status) {
    throw commandFailure(
      "LOCAL_SNAPSHOT_CHANGED",
      `worktree changed ${phase}`,
    );
  }
}

export async function finishDelivery({
  repository = process.cwd(),
  outputDirectory = defaultOutputDirectory(),
  refresh = defaultRefresh,
  beforePush = async () => {},
} = {}) {
  const resolvedRepository = path.resolve(repository);
  const resolvedOutput = path.resolve(outputDirectory);
  const repositoryRoot = path.resolve(
    git(
      resolvedRepository,
      ["rev-parse", "--show-toplevel"],
      "REPOSITORY_INVALID",
      "repository cannot be identified",
    ),
  );
  const canonicalRepository = await realpath(repositoryRoot);
  const canonicalOutput = await canonicalizeCandidate(resolvedOutput);
  if (
    pathIsInside(canonicalRepository, canonicalOutput) ||
    pathIsInside(canonicalOutput, canonicalRepository)
  ) {
    throw commandFailure(
      "OUTPUT_INSIDE_REPOSITORY",
      "dashboard output must be disjoint from the source repository",
    );
  }
  if (await pathExists(canonicalOutput)) {
    await validateOwnedOutput(canonicalOutput, { allowEmpty: true, allowLegacy: true });
  }
  const state = {
    taskId: null,
    issueId: null,
    branch: null,
    localHead: null,
    remote: null,
    remoteIdentity: null,
    pushPolicy: null,
    archivePolicy: null,
    ticketStatus: null,
    preflight: "FAIL",
    pushStep: "NOT_RUN",
    remoteStep: "NOT_RUN",
    refreshStep: "NOT_RUN",
    finalStep: "NOT_RUN",
  };
  let failure = null;

  try {
    const branch = git(
      resolvedRepository,
      ["branch", "--show-current"],
      "BRANCH_READ_FAILED",
      "current Git branch cannot be identified",
    );
    if (!branch) {
      throw commandFailure("DETACHED_HEAD", "delivery requires a named branch");
    }
    state.branch = branch;
    git(
      resolvedRepository,
      ["check-ref-format", "--branch", branch],
      "BRANCH_INVALID",
      "current Git branch name is invalid",
    );
    const initialStatus = git(
      resolvedRepository,
      ["status", "--porcelain=v1", "--untracked-files=normal"],
      "STATUS_READ_FAILED",
      "Git worktree status cannot be read",
    );
    if (initialStatus) {
      throw commandFailure("WORKTREE_DIRTY", "worktree must be clean before delivery");
    }
    state.localHead = git(
      resolvedRepository,
      ["rev-parse", "HEAD"],
      "HEAD_READ_FAILED",
      "local HEAD cannot be read",
    );
    verifyLocalSnapshot(resolvedRepository, branch, state.localHead, "during preflight");
    const deliveryBranch = parseDeliveryBranch(branch);
    const deliveryEvidence = deliveryBranch.kind === "TASK"
      ? await findTicket(
        resolvedRepository,
        branch,
        deliveryBranch.expectedTaskId,
        state.localHead,
      )
      : await findIssueEvidence(
        resolvedRepository,
        branch,
        deliveryBranch.expectedIssueId,
        state.localHead,
      );
    if (deliveryBranch.kind === "TASK") {
      await verifyTicketDependencies(resolvedRepository, deliveryEvidence, state.localHead);
      state.taskId = deliveryEvidence.metadata.ticket_id;
    } else {
      state.issueId = deliveryEvidence.metadata.issue_id;
    }
    state.remote = deliveryEvidence.metadata.delivery_remote;
    state.pushPolicy = deliveryEvidence.metadata.delivery_push;
    state.archivePolicy = deliveryEvidence.metadata.delivery_archive;
    state.ticketStatus = deliveryEvidence.normalizedStatus;
    if (!deliveryEvidence.metadata.delivery_repository) {
      throw commandFailure(
        "REMOTE_IDENTITY_MISSING",
        "delivery_repository must identify the authorized Git repository",
      );
    }
    const configuredRemotes = git(
      resolvedRepository,
      ["remote"],
      "REMOTE_READ_FAILED",
      "configured Git remotes cannot be read",
    )
      .split(/\r?\n/gu)
      .filter(Boolean);
    if (!configuredRemotes.includes(state.remote)) {
      throw commandFailure(
        "REMOTE_NOT_CONFIGURED",
        "delivery_remote must name a configured Git remote",
      );
    }
    state.remoteIdentity = captureRemoteIdentity(
      resolvedRepository,
      state.remote,
      deliveryEvidence.metadata.delivery_repository,
    );
    const defaultBranch = remoteDefaultBranch(
      resolvedRepository,
      state.remoteIdentity.fetchUrl,
    );
    if (branch === defaultBranch) {
      throw commandFailure("DEFAULT_BRANCH_FORBIDDEN", "delivery refuses the default branch");
    }
    await beforePush({
      branch: state.branch,
      localHead: state.localHead,
      repository: resolvedRepository,
    });
    verifyLocalSnapshot(resolvedRepository, branch, state.localHead, "before push");
    const beforePushRemoteIdentity = captureRemoteIdentity(
      resolvedRepository,
      state.remote,
    );
    if (!sameRemoteIdentity(state.remoteIdentity, beforePushRemoteIdentity)) {
      throw commandFailure("REMOTE_ENDPOINT_CHANGED", "remote endpoint changed before push");
    }
    if (
      remoteDefaultBranch(resolvedRepository, state.remoteIdentity.fetchUrl) === branch
    ) {
      throw commandFailure("DEFAULT_BRANCH_FORBIDDEN", "delivery refuses the default branch");
    }
    state.preflight = "PASS";

    if (state.pushPolicy === "authorized_non_force_feature_branch") {
      git(
        resolvedRepository,
        [
          "push",
          "--porcelain",
          "--no-follow-tags",
          "--no-verify",
          "--recurse-submodules=no",
          state.remoteIdentity.pushUrl,
          `${state.localHead}:refs/heads/${branch}`,
        ],
        "PUSH_FAILED",
        "git push failed",
      );
      state.pushStep = "PUSHED_NON_FORCE";
      const remoteHead = remoteBranchHead(
        resolvedRepository,
        state.remoteIdentity.pushUrl,
        branch,
      );
      if (!remoteHead || remoteHead !== state.localHead) {
        throw commandFailure(
          "REMOTE_HEAD_MISMATCH",
          "remote branch head does not equal local HEAD",
        );
      }
      git(
        resolvedRepository,
        ["update-ref", `refs/remotes/${state.remote}/${branch}`, state.localHead],
        "UPSTREAM_SET_FAILED",
        "remote-tracking branch cannot be updated",
      );
      git(
        resolvedRepository,
        ["branch", "--set-upstream-to", `${state.remote}/${branch}`, branch],
        "UPSTREAM_SET_FAILED",
        "Git upstream cannot be configured",
      );
      verifyUpstream(
        resolvedRepository,
        state.remote,
        branch,
        state.localHead,
        "after push",
      );
      state.remoteStep = "VERIFIED";
    } else {
      state.pushStep = "SKIPPED_LOCAL_ONLY";
      state.remoteStep = "SKIPPED_LOCAL_ONLY";
    }
  } catch (error) {
    failure = normalizeError(error);
  }

  try {
    await refreshSafely({
      canonicalOutput,
      canonicalRepository,
      refresh,
      repository: resolvedRepository,
      resolvedOutput,
    });
    state.refreshStep = "PASS";
  } catch (error) {
    state.refreshStep = "FAIL";
    if (!failure) {
      failure = normalizeError(error);
    }
  }


  if (!failure) {
    try {
      const finalCanonicalOutput = await canonicalizeCandidate(resolvedOutput);
      if (
        pathIsInside(canonicalRepository, finalCanonicalOutput) ||
        path.relative(canonicalOutput, finalCanonicalOutput) !== ""
      ) {
        throw commandFailure(
          "OUTPUT_PATH_CHANGED",
          "dashboard output changed during delivery",
        );
      }
      const finalBranch = git(
        resolvedRepository,
        ["branch", "--show-current"],
        "FINAL_STATE_READ_FAILED",
        "final Git branch cannot be read",
      );
      const finalHead = git(
        resolvedRepository,
        ["rev-parse", "HEAD"],
        "FINAL_STATE_READ_FAILED",
        "final Git HEAD cannot be read",
      );
      const finalStatus = git(
        resolvedRepository,
        ["status", "--porcelain=v1", "--untracked-files=normal"],
        "FINAL_STATE_READ_FAILED",
        "final Git worktree status cannot be read",
      );
      if (
        finalBranch !== state.branch ||
        finalHead !== state.localHead ||
        finalStatus
      ) {
        throw commandFailure(
          "FINAL_STATE_CHANGED",
          "worktree changed during delivery",
        );
      }
      if (state.pushPolicy === "authorized_non_force_feature_branch") {
        const finalRemoteIdentity = captureRemoteIdentity(
          resolvedRepository,
          state.remote,
        );
        if (!sameRemoteIdentity(state.remoteIdentity, finalRemoteIdentity)) {
          throw commandFailure("REMOTE_ENDPOINT_CHANGED", "remote endpoint changed during delivery");
        }
        if (
          remoteDefaultBranch(resolvedRepository, state.remoteIdentity.fetchUrl) === state.branch
        ) {
          throw commandFailure("DEFAULT_BRANCH_FORBIDDEN", "delivery refuses the default branch");
        }
        const finalRemoteHead = remoteBranchHead(
          resolvedRepository,
          state.remoteIdentity.pushUrl,
          state.branch,
          "final",
        );
        if (!finalRemoteHead || finalRemoteHead !== state.localHead) {
          throw commandFailure(
            "FINAL_REMOTE_CHANGED",
            "remote branch changed during delivery",
          );
        }
        verifyUpstream(
          resolvedRepository,
          state.remote,
          state.branch,
          state.localHead,
          "during final verification",
        );
      }
      state.finalStep = "PASS";
    } catch (error) {
      state.finalStep = "FAIL";
      failure = normalizeError(error);
    }
  }

  if (failure) {
    throw failure;
  }

  return {
    success: true,
    taskId: state.taskId,
    issueId: state.issueId,
    branch: state.branch,
    localHead: state.localHead,
    push: {
      policy: state.pushPolicy,
      performed: state.pushStep === "PUSHED_NON_FORCE",
    },
    remote: {
      name: state.remote,
      verified: state.remoteStep === "VERIFIED",
    },
    refresh: { completed: true },
    archiveReady:
      state.archivePolicy === "after_success" &&
      successfulTerminalTicketStatuses.has(state.ticketStatus),
  };
}

export function formatSuccessOutput(result) {
  const lines = [
    "LATTICE_DELIVERY_FINISHED=1",
    result.taskId ? `task=${result.taskId}` : `issue=${result.issueId}`,
    `branch=${result.branch}`,
    `head=${result.localHead}`,
    `push=${result.push.performed ? "PUSHED_NON_FORCE" : "SKIPPED_LOCAL_ONLY"}`,
    `remote=${result.remote.verified ? "VERIFIED" : "SKIPPED_LOCAL_ONLY"}`,
    "dashboard=REFRESHED",
  ];
  if (result.archiveReady) {
    lines.push("LATTICE_DELIVERY_READY_TO_ARCHIVE=1");
  }
  return `${lines.join("\n")}\n`;
}

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--repository") {
      const value = argv[index + 1];
      if (!value || value.startsWith("--")) {
        throw commandFailure("ARGUMENT_INVALID", "--repository requires a path");
      }
      options.repository = value;
      index += 1;
    } else if (argument === "--output") {
      const value = argv[index + 1];
      if (!value || value.startsWith("--")) {
        throw commandFailure("ARGUMENT_INVALID", "--output requires a path");
      }
      options.outputDirectory = value;
      index += 1;
    } else if (argument === "--help" || argument === "-h") {
      options.help = true;
    } else {
      throw commandFailure("ARGUMENT_INVALID", "unknown command argument");
    }
  }
  if (options.repository === undefined) {
    options.repository = process.cwd();
  }
  if (options.outputDirectory === undefined) {
    options.outputDirectory = defaultOutputDirectory();
  }
  if (!options.repository) {
    throw commandFailure("ARGUMENT_INVALID", "--repository requires a path");
  }
  if (!options.outputDirectory) {
    throw commandFailure("ARGUMENT_INVALID", "--output requires a path");
  }
  return options;
}

export async function main(argv = process.argv.slice(2)) {
  const options = parseArguments(argv);
  if (options.help) {
    process.stdout.write(
      "LATTICE engineering delivery finisher\n\n" +
      "Usage:\n" +
      "  node scripts/finish-lattice-delivery.mjs [options]\n\n" +
      "Options:\n" +
      "  --repository PATH  Clean committed TASK worktree (defaults to cwd)\n" +
      "  --output PATH      Dashboard output directory\n" +
      "  --help             Show this help\n",
    );
    return;
  }
  const result = await finishDelivery(options);
  process.stdout.write(formatSuccessOutput(result));
}

const isMain =
  process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url;

if (isMain) {
  main().catch((error) => {
    const normalized = normalizeError(error);
    process.stderr.write(
      `LATTICE_DELIVERY_FINISHED=0\nerror_code=${normalized.code}\nerror=delivery failed; task kept open\n`,
    );
    process.exitCode = 1;
  });
}
