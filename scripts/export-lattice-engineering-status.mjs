import { execFile as execFileCallback, spawn } from "node:child_process";
import {
  access,
  mkdir,
  readFile,
  readdir,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { promisify } from "node:util";
import { fileURLToPath, pathToFileURL } from "node:url";

const execFile = promisify(execFileCallback);
const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const templatePath = path.resolve(
  scriptDirectory,
  "..",
  "tools",
  "engineering-status-dashboard",
  "index.template.html",
);
const schema = "lattice.engineering-status/1.0";
const terminalStates = new Set([
  "VERIFIED",
  "COMPLETE",
  "IN_PROGRESS",
  "FAIL",
  "BLOCKED",
  "WAITING_DEPENDENCY",
  "USER_ACTION",
  "UNKNOWN",
  "STALE",
  "PARTIAL",
  "PAUSED",
  "SUPERSEDED",
]);
const ticketStatusOutcomes = new Map([
  ["verified", "VERIFIED"],
  ["complete", "COMPLETE"],
  ["completed", "COMPLETE"],
  ["in_progress", "IN_PROGRESS"],
  ["active", "IN_PROGRESS"],
  ["ready", "IN_PROGRESS"],
  ["partial", "PARTIAL"],
  ["paused", "PAUSED"],
  ["superseded", "SUPERSEDED"],
  ["fail", "FAIL"],
  ["failed", "FAIL"],
  ["blocked", "BLOCKED"],
  ["waiting_dependency", "WAITING_DEPENDENCY"],
  ["user_action", "USER_ACTION"],
  ["stale", "STALE"],
]);

function cleanError(error) {
  const detail = String(error?.stderr || error?.message || error || "unknown error")
    .replaceAll(/\s+/gu, " ")
    .trim();
  return detail.slice(0, 240) || "unknown error";
}

async function runFile(command, args, options = {}) {
  const { stdout } = await execFile(command, args, {
    cwd: options.cwd,
    encoding: "utf8",
    maxBuffer: 8 * 1024 * 1024,
    timeout: options.timeout ?? 12_000,
    windowsHide: true,
  });
  return stdout.trim();
}

async function gitAt(repository, args, options = {}) {
  return runFile(
    "git",
    ["-c", `safe.directory=${repository}`, "-C", repository, ...args],
    options,
  );
}

function parseArguments(argv) {
  const options = {
    repository: process.cwd(),
    output: undefined,
    offline: false,
    open: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--repository") {
      options.repository = argv[++index];
    } else if (argument === "--output") {
      options.output = argv[++index];
    } else if (argument === "--offline") {
      options.offline = true;
    } else if (argument === "--open") {
      options.open = true;
    } else if (argument === "--help" || argument === "-h") {
      options.help = true;
    } else {
      throw new Error(`Unknown argument: ${argument}`);
    }
  }
  if (!options.repository) {
    throw new Error("--repository requires a path");
  }
  return options;
}

function defaultOutputDirectory() {
  const applicationData =
    process.env.LOCALAPPDATA || path.join(os.homedir(), ".local", "share");
  return path.join(applicationData, "LATTICE", "engineering-status");
}

function parseWorktreeList(text) {
  return text
    .split(/\r?\n\r?\n/gu)
    .map((block) => block.trim())
    .filter(Boolean)
    .map((block) => {
      const record = {};
      for (const line of block.split(/\r?\n/gu)) {
        const separator = line.indexOf(" ");
        if (separator < 0) {
          record[line] = true;
        } else {
          record[line.slice(0, separator)] = line.slice(separator + 1);
        }
      }
      return {
        path: record.worktree,
        head: record.HEAD || null,
        branch: record.branch?.replace(/^refs\/heads\//u, "") || "(detached)",
        detached: Boolean(record.detached),
        prunable: Boolean(record.prunable),
      };
    });
}

function identityFromBranch(branch) {
  const task = branch.match(/(?:^|[\/_-])task[-_\/]?([0-9]{3})(?=$|[\/_-])/iu);
  if (task) {
    return { kind: "TASK", id: `TASK-${task[1]}` };
  }
  const issue = branch.match(/(?:^|[\/_-])issue[-_\/]?([0-9]+)(?=$|[\/_-])/iu);
  if (issue) {
    return { kind: "ISSUE", id: `ISSUE-${issue[1]}` };
  }
  return { kind: "BRANCH", id: branch || "DETACHED" };
}

function parseFrontmatter(content) {
  if (!content.startsWith("---\n") && !content.startsWith("---\r\n")) {
    return { values: {}, duplicateKeys: [], valid: false };
  }
  const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/u);
  if (!match) {
    return { values: {}, duplicateKeys: [], valid: false };
  }
  const values = {};
  const duplicateKeys = new Set();
  for (const line of match[1].split(/\r?\n/gu)) {
    const entry = line.match(/^([a-zA-Z0-9_]+):\s*(.*?)\s*$/u);
    if (entry) {
      if (Object.hasOwn(values, entry[1])) {
        duplicateKeys.add(entry[1]);
      }
      values[entry[1]] = entry[2].replace(/^['"]|['"]$/gu, "");
    }
  }
  return { values, duplicateKeys: [...duplicateKeys], valid: true };
}

function redactLocalPaths(value) {
  return String(value || "")
    .replaceAll(/`(?:[a-z]:[\\/]|\\\\|\/)[^`\r\n]+`/giu, "[本機路徑]")
    .replaceAll(/(["'])(?:[a-z]:[\\/]|\\\\|\/)[^"'\r\n]+["']/giu, "[本機路徑]")
    .replaceAll(/\\\\[^<>\r\n,;，；。]+/gu, "[本機路徑]")
    .replaceAll(/[a-z]:[\\/][^<>\r\n,;，；。]+/giu, "[本機路徑]")
    .replaceAll(/(?<![\w:])\/(?:users|home|root|data|tmp|var|opt|mnt|private|srv|workspace)\/[^<>\r\n,;，；。]+/giu, "[本機路徑]");
}

function stripMarkdown(value, limit = 280) {
  const normalized = redactLocalPaths(value)
    .replaceAll(/<!--[^]*?-->/gu, " ")
    .replaceAll(/```[^]*?```/gu, " ")
    .replaceAll(/\[([^\]]+)\]\([^\)]+\)/gu, "$1")
    .replaceAll(/[`*_>#|]/gu, " ")
    .replaceAll(/^\s*[-+]\s+/gmu, "")
    .replaceAll(/\s+/gu, " ")
    .trim();
  if (normalized.length <= limit) {
    return normalized;
  }
  return `${normalized.slice(0, limit - 1).trimEnd()}…`;
}

function escapeRegularExpression(value) {
  return String(value).replaceAll(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

function extractSection(content, headingNames) {
  const alternatives = headingNames
    .map((heading) => heading.replaceAll(/[.*+?^${}()|[\]\\]/gu, "\\$&"))
    .join("|");
  const pattern = new RegExp(`^##\\s+(?:${alternatives})\\s*$`, "imu");
  const match = pattern.exec(content);
  if (!match) {
    return "";
  }
  const remainder = content.slice(match.index + match[0].length).replace(/^\r?\n/u, "");
  const nextHeading = remainder.search(/^##\s+/mu);
  const body = nextHeading < 0 ? remainder : remainder.slice(0, nextHeading);
  const firstParagraph = body
    .trim()
    .split(/\r?\n\s*\r?\n/gu)
    .find((paragraph) => paragraph.trim());
  return stripMarkdown(firstParagraph || "");
}

function explicitTerminalState(content) {
  const matches = [
    ...content.matchAll(
      /Current\s+terminal\s+state\s+is\s+`?([A-Z][A-Z0-9_]*)`?/giu,
    ),
  ];
  if (matches.length === 0) {
    return null;
  }
  const candidate = matches.at(-1)[1].toUpperCase();
  return terminalStates.has(candidate) ? candidate : "UNKNOWN";
}

export function classifyTicketStatus(status) {
  const normalized = String(status || "")
    .trim()
    .toLowerCase()
    .replaceAll(/[-\s]+/gu, "_");
  return ticketStatusOutcomes.get(normalized) || null;
}

function outcomeFromTicket(frontmatter, content) {
  const explicit = explicitTerminalState(content);
  if (explicit) {
    return explicit;
  }
  return classifyTicketStatus(frontmatter.status) || "UNKNOWN";
}

function defaultNextStep(outcome) {
  const mapping = {
    VERIFIED: "已驗收；等待下一個已授權工程決定。",
    COMPLETE: "已完成；等待整合或下一個已授權工程決定。",
    IN_PROGRESS: "依目前票券繼續實作與驗證。",
    FAIL: "先完成明確修正，再重新執行失敗的驗證。",
    BLOCKED: "先解除目前阻擋條件。",
    WAITING_DEPENDENCY: "等待依賴或新的必要授權後再繼續。",
    USER_ACTION: "等待使用者完成票券記錄的決定。",
    STALE: "重新整理並核對目前分支證據。",
    PARTIAL: "依票券補完剩餘工作與驗證。",
    PAUSED: "等待恢復優先順序或票券記錄的條件。",
    SUPERSEDED: "改從取代這張票券的新任務著手。",
    UNKNOWN: "補齊目前票券或可驗證證據。",
  };
  return mapping[outcome] || mapping.UNKNOWN;
}

function humanizeBranch(branch, identity) {
  const withoutPrefix = branch
    .replace(/^(feature|fix|chore|docs|issue)\//iu, "")
    .replace(new RegExp(escapeRegularExpression(identity.id).replace("-", "[-_]"), "iu"), "")
    .replaceAll(/[-_/]+/gu, " ")
    .trim();
  if (!withoutPrefix) {
    return identity.kind === "BRANCH" ? branch : `${identity.id} 工程分支`;
  }
  return withoutPrefix.replace(/\b\w/gu, (letter) => letter.toUpperCase());
}

function humanActionSummary(humanGate) {
  if (!humanGate) {
    return "目前票券沒有記錄需要使用者操作。";
  }
  if (/no user action|不需要(?:你|使用者).*操作|無需(?:你|使用者)/iu.test(humanGate)) {
    return "目前不需要你操作。";
  }
  if (/requires?.*(?:user|human).*(?:approval|authorization|decision)|需要.*(?:授權|決定|確認)/iu.test(humanGate)) {
    return "票券記錄仍有需要你授權或決定的關卡。";
  }
  return stripMarkdown(humanGate, 180);
}

async function readTicket(worktreePath, identity, branch) {
  if (identity.kind !== "TASK") {
    return { ticket: null, error: null };
  }
  const directory = path.join(worktreePath, "docs", "tickets");
  try {
    const entries = await readdir(directory, { withFileTypes: true });
    const prefix = `${identity.id}-`;
    const candidates = entries.filter(
      (entry) =>
        entry.isFile() &&
        entry.name.toUpperCase().startsWith(prefix) &&
        entry.name.toLowerCase().endsWith(".md"),
    );
    if (candidates.length === 0) {
      return { ticket: null, error: "目前分支找不到對應 TASK 票券" };
    }
    if (candidates.length > 1) {
      return { ticket: null, error: "目前分支有重複的 TASK 票券" };
    }
    const candidate = candidates[0];
    const content = await readFile(path.join(directory, candidate.name), "utf8");
    const parsedFrontmatter = parseFrontmatter(content);
    if (!parsedFrontmatter.valid) {
      return { ticket: null, error: "TASK 票券的 frontmatter 無法辨識" };
    }
    if (parsedFrontmatter.duplicateKeys.length > 0) {
      return { ticket: null, error: "TASK 票券有重複的 frontmatter 欄位" };
    }
    const frontmatter = parsedFrontmatter.values;
    if (frontmatter.ticket_id !== identity.id) {
      return { ticket: null, error: "TASK 票券的 ticket_id 不符合分支" };
    }
    if (!frontmatter.status) {
      return { ticket: null, error: "TASK 票券缺少 status" };
    }
    if (!classifyTicketStatus(frontmatter.status)) {
      return { ticket: null, error: "TASK 票券的 status 無法辨識" };
    }
    if (frontmatter.branch !== branch) {
      return { ticket: null, error: "TASK 票券的 branch 不符合目前分支" };
    }
    return { ticket: {
      fileName: candidate.name,
      content,
      frontmatter,
    }, error: null };
  } catch (error) {
    if (error?.code === "ENOENT") {
      return { ticket: null, error: "目前分支找不到對應 TASK 票券" };
    }
    return { ticket: null, error: "TASK 票券無法讀取" };
  }
}

async function optionalGit(repository, args) {
  try {
    return { ok: true, value: await gitAt(repository, args) };
  } catch (error) {
    return { ok: false, error: cleanError(error), value: "" };
  }
}

async function collectRemoteHeads(repository, offline) {
  const checkedAt = new Date().toISOString();
  if (offline) {
    return { state: "offline", checkedAt, remotes: new Map(), error: null };
  }
  const namesResult = await optionalGit(repository, ["remote"]);
  if (!namesResult.ok) {
    return {
      state: "unavailable",
      checkedAt,
      remotes: new Map(),
      error: "Git 遠端清單無法讀取",
    };
  }
  const names = namesResult.value.split(/\r?\n/gu).filter(Boolean);
  if (names.length === 0) {
    return { state: "no-remotes", checkedAt, remotes: new Map(), error: null };
  }
  const results = await mapLimit(names, 3, async (name) => {
    const headsResult = await optionalGit(repository, ["ls-remote", "--heads", name]);
    if (!headsResult.ok) {
      return [name, { state: "unavailable", heads: new Map() }];
    }
    const heads = new Map();
    for (const line of headsResult.value.split(/\r?\n/gu).filter(Boolean)) {
      const match = line.match(/^([0-9a-f]{40})\s+refs\/heads\/(.+)$/iu);
      if (match) heads.set(match[2], match[1]);
    }
    return [name, { state: "available", heads }];
  });
  const remotes = new Map(results);
  const unavailable = results.filter(([, result]) => result.state !== "available").length;
  return {
    state: unavailable === 0 ? "available" : unavailable === results.length ? "unavailable" : "partial",
    checkedAt,
    remotes,
    error: unavailable ? "至少一個 Git 遠端無法即時核對" : null,
  };
}

async function collectWorktree(record, remoteEvidence) {
  const identity = identityFromBranch(record.branch);
  const errors = [];
  const statusResult = await optionalGit(record.path, ["status", "--porcelain=v1"]);
  const commitResult = await optionalGit(record.path, [
    "show",
    "-s",
    "--format=%H%x00%h%x00%s%x00%cI",
    "HEAD",
  ]);
  if (!statusResult.ok) {
    errors.push("Git 狀態無法讀取");
  }
  if (!commitResult.ok) {
    errors.push("最後提交無法讀取");
  }

  const upstreamResult = await optionalGit(record.path, [
    "rev-parse",
    "--abbrev-ref",
    "--symbolic-full-name",
    "@{upstream}",
  ]);
  let sync = {
    state: "no-upstream",
    ahead: 0,
    behind: 0,
    upstream: null,
    remoteVerified: false,
    remoteCheckedAt: remoteEvidence.checkedAt,
  };
  if (upstreamResult.ok && upstreamResult.value) {
    const slash = upstreamResult.value.indexOf("/");
    const remoteName = slash < 0 ? null : upstreamResult.value.slice(0, slash);
    const remoteBranch = slash < 0 ? null : upstreamResult.value.slice(slash + 1);
    const liveRemote = remoteName ? remoteEvidence.remotes.get(remoteName) : null;
    const remoteHead = liveRemote?.heads.get(remoteBranch);
    const upstreamHead = await optionalGit(record.path, ["rev-parse", "@{upstream}"]);
    if (liveRemote?.state === "available" && !remoteHead) {
      sync = {
        state: "remote-missing",
        ahead: 0,
        behind: 0,
        upstream: upstreamResult.value,
        remoteVerified: true,
        remoteCheckedAt: remoteEvidence.checkedAt,
      };
    } else if (remoteHead === record.head) {
      sync = {
        state: "synced",
        ahead: 0,
        behind: 0,
        upstream: upstreamResult.value,
        remoteVerified: true,
        remoteCheckedAt: remoteEvidence.checkedAt,
      };
    } else if (remoteHead && (!upstreamHead.ok || upstreamHead.value !== remoteHead)) {
      sync = {
        state: "remote-changed",
        ahead: 0,
        behind: 0,
        upstream: upstreamResult.value,
        remoteVerified: true,
        remoteCheckedAt: remoteEvidence.checkedAt,
      };
    } else if (remoteHead) {
      const divergence = await optionalGit(record.path, [
        "rev-list",
        "--left-right",
        "--count",
        "HEAD...@{upstream}",
      ]);
      if (divergence.ok) {
        const [ahead = 0, behind = 0] = divergence.value
          .split(/\s+/gu)
          .map((value) => Number.parseInt(value, 10));
        let state = "synced";
        if (ahead > 0 && behind > 0) state = "diverged";
        else if (ahead > 0) state = "ahead";
        else if (behind > 0) state = "behind";
        sync = {
          state,
          ahead: Number.isFinite(ahead) ? ahead : 0,
          behind: Number.isFinite(behind) ? behind : 0,
          upstream: upstreamResult.value,
          remoteVerified: true,
          remoteCheckedAt: remoteEvidence.checkedAt,
        };
      } else {
        sync = {
          state: "unknown",
          ahead: 0,
          behind: 0,
          upstream: upstreamResult.value,
          remoteVerified: true,
          remoteCheckedAt: remoteEvidence.checkedAt,
        };
        errors.push("遠端差異無法讀取");
      }
    } else {
      sync = {
        state: "unverified",
        ahead: 0,
        behind: 0,
        upstream: upstreamResult.value,
        remoteVerified: false,
        remoteCheckedAt: remoteEvidence.checkedAt,
      };
    }
  }

  const ticketResult = await readTicket(record.path, identity, record.branch);
  const ticket = ticketResult.ticket;
  if (ticketResult.error) {
    errors.push(ticketResult.error);
  }
  const ticketContent = ticket?.content || "";
  const ticketMetadata = ticket?.frontmatter || {};
  const outcome = outcomeFromTicket(ticketMetadata, ticketContent);
  const commitParts = commitResult.ok
    ? commitResult.value.split("\0")
    : [record.head || "", String(record.head || "").slice(0, 7), "", ""];
  const objective = extractSection(ticketContent, ["Objective", "目標"]);
  const nextStep = extractSection(ticketContent, [
    "Next action",
    "Next Action",
    "Next step",
    "Next Step",
    "下一步",
  ]);
  const humanGate = extractSection(ticketContent, ["Human gate", "Human Gate", "人類關卡"]);
  const clean = statusResult.ok ? statusResult.value.length === 0 : null;
  const changeCount = statusResult.ok
    ? statusResult.value.split(/\r?\n/gu).filter(Boolean).length
    : null;

  return {
    id: identity.id,
    kind: identity.kind,
    branch: record.branch,
    title: stripMarkdown(ticketMetadata.title || humanizeBranch(record.branch, identity), 120),
    summary: objective || "這個分支尚未提供白話目標摘要。",
    outcome,
    ticket: ticket
      ? {
          status: ticketMetadata.status || "unknown",
          file: ticket.fileName,
        }
      : null,
    nextStep: nextStep || defaultNextStep(outcome),
    userAction: humanActionSummary(humanGate),
    evidenceState: errors.length === 0 ? "complete" : "partial",
    errors,
    worktree: {
      name: path.basename(record.path),
      detached: record.detached,
      prunable: record.prunable,
    },
    git: {
      clean,
      changeCount,
      head: commitParts[0] || record.head || null,
      shortHead: commitParts[1] || String(record.head || "").slice(0, 7),
      lastCommit: commitParts[2] || "無法讀取最後提交",
      lastCommitAt: commitParts[3] || null,
      sync,
    },
    github: {
      state: "unknown",
      pr: null,
      ci: "unknown",
    },
  };
}

async function mapLimit(values, limit, mapper) {
  const results = new Array(values.length);
  let nextIndex = 0;
  async function worker() {
    while (nextIndex < values.length) {
      const currentIndex = nextIndex;
      nextIndex += 1;
      results[currentIndex] = await mapper(values[currentIndex], currentIndex);
    }
  }
  await Promise.all(
    Array.from({ length: Math.min(limit, values.length) }, () => worker()),
  );
  return results;
}

function githubSlug(remoteUrl) {
  const match = String(remoteUrl || "").match(
    /github\.com[/:]([^/]+)\/([^/]+?)(?:\.git)?$/iu,
  );
  return match ? `${match[1]}/${match[2]}` : null;
}

function ciState(rollup) {
  if (!Array.isArray(rollup) || rollup.length === 0) {
    return "unknown";
  }
  const values = rollup.map((check) =>
    String(check.conclusion || check.state || check.status || "").toUpperCase(),
  );
  if (values.some((value) => ["FAILURE", "ERROR", "CANCELLED", "TIMED_OUT"].includes(value))) {
    return "failing";
  }
  if (values.some((value) => ["PENDING", "QUEUED", "IN_PROGRESS", "EXPECTED"].includes(value))) {
    return "pending";
  }
  if (values.every((value) => ["SUCCESS", "NEUTRAL", "SKIPPED", "COMPLETED"].includes(value))) {
    return "passing";
  }
  return "unknown";
}

async function enrichFromGitHub(repository, items, offline) {
  if (offline) {
    for (const item of items) item.github.state = "offline";
    return { state: "offline", repository: null, error: null };
  }
  const remoteResult = await optionalGit(repository, ["remote", "get-url", "origin"]);
  const slug = remoteResult.ok ? githubSlug(remoteResult.value) : null;
  if (!slug) {
    for (const item of items) item.github.state = "unavailable";
    return { state: "unavailable", repository: null, error: "GitHub origin 未識別" };
  }
  try {
    const output = await runFile(
      "gh",
      [
        "pr",
        "list",
        "--repo",
        slug,
        "--state",
        "all",
        "--limit",
        "100",
        "--json",
        "number,title,state,isDraft,headRefName,url,statusCheckRollup,updatedAt",
      ],
      { cwd: repository, timeout: 15_000 },
    );
    const pullRequests = JSON.parse(output || "[]");
    const byBranch = new Map();
    for (const pullRequest of pullRequests) {
      if (!byBranch.has(pullRequest.headRefName)) {
        byBranch.set(pullRequest.headRefName, pullRequest);
      }
    }
    for (const item of items) {
      const pullRequest = byBranch.get(item.branch);
      item.github.state = "available";
      if (pullRequest) {
        item.github.pr = {
          number: pullRequest.number,
          title: stripMarkdown(pullRequest.title, 120),
          state: pullRequest.isDraft ? "DRAFT" : pullRequest.state,
          url: pullRequest.url,
          updatedAt: pullRequest.updatedAt,
        };
        item.github.ci = ciState(pullRequest.statusCheckRollup);
      }
    }
    return { state: "available", repository: slug, error: null };
  } catch (error) {
    for (const item of items) item.github.state = "unavailable";
    return { state: "unavailable", repository: slug, error: "GitHub PR/CI 查詢失敗" };
  }
}

async function repositoryDisplayName(repository) {
  try {
    const packageJson = JSON.parse(await readFile(path.join(repository, "package.json"), "utf8"));
    if (packageJson.name) {
      return packageJson.name;
    }
  } catch {
    // A package manifest is optional for fixture and non-Node repositories.
  }
  return path.basename(repository);
}

export async function buildSnapshot({ repository, offline = false } = {}) {
  const requestedRepository = path.resolve(repository || process.cwd());
  const repositoryRoot = await gitAt(requestedRepository, ["rev-parse", "--show-toplevel"]);
  const branch = await gitAt(repositoryRoot, ["branch", "--show-current"]);
  const head = await gitAt(repositoryRoot, ["rev-parse", "HEAD"]);
  const worktreeText = await gitAt(repositoryRoot, ["worktree", "list", "--porcelain"]);
  const records = parseWorktreeList(worktreeText);
  if (records.length === 0) {
    throw new Error("Git did not report any registered worktrees");
  }
  const remoteEvidence = await collectRemoteHeads(repositoryRoot, offline);
  const items = await mapLimit(records, 4, (record) => collectWorktree(record, remoteEvidence));
  const github = await enrichFromGitHub(repositoryRoot, items, offline);
  const incompleteCount = items.filter((item) => item.evidenceState !== "complete").length;
  const currentItem = items.find((item) => item.branch === branch) || null;
  return {
    schema,
    generatedAt: new Date().toISOString(),
    completeness: incompleteCount === 0 ? "complete" : "partial",
    repository: {
      displayName: await repositoryDisplayName(repositoryRoot),
      currentBranch: branch || "(detached)",
      head,
      shortHead: head.slice(0, 7),
      sourceWorktree: path.basename(repositoryRoot),
      github: github.repository,
    },
    currentItemId: currentItem?.id || null,
    sources: {
      git: "available",
      gitRemote: {
        state: remoteEvidence.state,
        checkedAt: remoteEvidence.checkedAt,
        error: remoteEvidence.error,
      },
      tickets: incompleteCount === 0 ? "available" : "partial",
      github,
    },
    items,
  };
}

function escapeEmbeddedJson(json) {
  return json
    .replaceAll("&", "\\u0026")
    .replaceAll("<", "\\u003c")
    .replaceAll(">", "\\u003e")
    .replaceAll("\u2028", "\\u2028")
    .replaceAll("\u2029", "\\u2029");
}

async function exists(file) {
  try {
    await access(file);
    return true;
  } catch {
    return false;
  }
}

async function replaceOutputPair(output, json, html) {
  const token = `${process.pid}-${Date.now()}`;
  const entries = [
    {
      target: path.join(output, "status.json"),
      temporary: path.join(output, `.status.json.tmp-${token}`),
      backup: path.join(output, `.status.json.bak-${token}`),
      content: json,
    },
    {
      target: path.join(output, "index.html"),
      temporary: path.join(output, `.index.html.tmp-${token}`),
      backup: path.join(output, `.index.html.bak-${token}`),
      content: html,
    },
  ];
  await Promise.all(entries.map((entry) => writeFile(entry.temporary, entry.content, "utf8")));
  const backedUp = [];
  const installed = [];
  let committed = false;
  try {
    for (const entry of entries) {
      if (await exists(entry.target)) {
        await rename(entry.target, entry.backup);
        backedUp.push(entry);
      }
    }
    for (const entry of entries) {
      await rename(entry.temporary, entry.target);
      installed.push(entry);
    }
    committed = true;
  } catch (error) {
    for (const entry of installed) {
      await rm(entry.target, { force: true });
    }
    for (const entry of [...backedUp].reverse()) {
      await rename(entry.backup, entry.target);
    }
    throw error;
  } finally {
    const cleanup = entries.map((entry) => rm(entry.temporary, { force: true }));
    if (committed) {
      cleanup.push(...entries.map((entry) => rm(entry.backup, { force: true })));
    }
    await Promise.all(cleanup);
  }
}

export async function writeDashboard(snapshot, outputDirectory) {
  if (snapshot?.schema !== schema || !Array.isArray(snapshot.items)) {
    throw new Error("Refusing to write an invalid engineering-status snapshot");
  }
  const template = await readFile(templatePath, "utf8");
  const placeholder = "__LATTICE_STATUS_JSON__";
  if (!template.includes(placeholder)) {
    throw new Error("Dashboard template is missing its data placeholder");
  }
  const output = path.resolve(outputDirectory || defaultOutputDirectory());
  await mkdir(output, { recursive: true });
  const prettyJson = `${JSON.stringify(snapshot, null, 2)}\n`;
  const html = template.replace(
    placeholder,
    escapeEmbeddedJson(JSON.stringify(snapshot)),
  );
  await replaceOutputPair(output, prettyJson, html);
  return {
    output,
    htmlPath: path.join(output, "index.html"),
    jsonPath: path.join(output, "status.json"),
  };
}

export async function openLocalFile(
  file,
  { platform = process.platform, spawnProcess = spawn } = {},
) {
  const command = platform === "win32" ? "explorer.exe" : platform === "darwin" ? "open" : "xdg-open";
  const child = spawnProcess(command, [file], {
    detached: true,
    stdio: "ignore",
    windowsHide: true,
  });
  await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("spawn", resolve);
  });
  child.unref();
}

function printHelp() {
  process.stdout.write(`LATTICE engineering status dashboard\n\nUsage:\n  node scripts/export-lattice-engineering-status.mjs [options]\n\nOptions:\n  --repository PATH  Git worktree used to discover the repository\n  --output PATH      Output directory (defaults to local application data)\n  --offline          Skip optional GitHub PR/CI enrichment\n  --open             Open index.html after a successful refresh\n  --help             Show this help\n`);
}

export async function main(argv = process.argv.slice(2)) {
  const options = parseArguments(argv);
  if (options.help) {
    printHelp();
    return;
  }
  const snapshot = await buildSnapshot(options);
  const output = await writeDashboard(snapshot, options.output);
  process.stdout.write(`LATTICE_STATUS_UPDATED=${output.htmlPath}\n`);
  process.stdout.write(`LATTICE_STATUS_ITEMS=${snapshot.items.length}\n`);
  process.stdout.write(`LATTICE_STATUS_COMPLETENESS=${snapshot.completeness}\n`);
  if (options.open) {
    await openLocalFile(output.htmlPath);
    process.stdout.write("LATTICE_STATUS_OPENED=1\n");
  }
}

const invokedDirectly =
  process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url;
if (invokedDirectly) {
  main().catch((error) => {
    process.stderr.write(`LATTICE_STATUS_FAILED=${cleanError(error)}\n`);
    process.exitCode = 1;
  });
}
