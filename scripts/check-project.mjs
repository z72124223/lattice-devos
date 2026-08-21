import { lstat, readFile, readdir, realpath } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";
import process from "node:process";

const root = process.cwd();
const ignoredDirectories = new Set([
  ".git",
  "node_modules",
  "coverage",
  "target",
]);

async function walk(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    if (entry.isDirectory() && ignoredDirectories.has(entry.name)) {
      continue;
    }
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await walk(absolute)));
    } else if (entry.isFile()) {
      files.push(absolute);
    }
  }
  return files;
}

function relative(file) {
  return path.relative(root, file).replaceAll("\\", "/");
}

function frontmatterScalar(frontmatter, key) {
  const pattern = new RegExp(`^${key}:\\s*(\\S+)\\s*$`, "gmu");
  const matches = [...frontmatter.matchAll(pattern)];
  return {
    valid: matches.length === 1,
    value: matches.length === 1 ? matches[0][1] : null,
  };
}

const files = await walk(root);
const errors = [];

async function isGitWorktreeRoot(candidate) {
  try {
    const dotGit = await lstat(path.join(candidate, ".git"));
    return dotGit.isFile();
  } catch {
    return false;
  }
}

const resolvedRoot = await realpath(root);
if (path.resolve(root).toLowerCase() !== resolvedRoot.toLowerCase()) {
  errors.push("worktree root must not resolve through a reparse point.");
}
for (let ancestor = path.dirname(resolvedRoot); ancestor !== path.dirname(ancestor); ancestor = path.dirname(ancestor)) {
  if (await isGitWorktreeRoot(ancestor)) {
    errors.push("worktree root must not be nested inside another Git worktree.");
    break;
  }
}

const engineeringProtocolPath = "docs/contracts/ENGINEERING_PROTOCOL_V1.md";
const engineeringProtocolFile = files.find(
  (candidate) => relative(candidate) === engineeringProtocolPath,
);
if (!engineeringProtocolFile) {
  errors.push(`${engineeringProtocolPath}: missing engineering protocol.`);
} else {
  const protocol = await readFile(engineeringProtocolFile, "utf8");
  const normalizedProtocol = protocol.replaceAll(/\s+/gu, " ").toLowerCase();
  const requiredProtocolContent = [
    "protocol_id: LATTICE_ENGINEERING_PROTOCOL",
    "version: 2.0.0",
    "canonical_path: docs/contracts/ENGINEERING_PROTOCOL_V1.md",
    "## Safety Boundaries",
    "## Risk Tiers",
    "## Complexity Circuit Breaker",
    "## Delivery",
    "Do not create another task only to repair governance",
    "ordinary local completion does not require the finisher",
  ];
  for (const required of requiredProtocolContent) {
    if (!normalizedProtocol.includes(required.toLowerCase())) {
      errors.push(
        `${engineeringProtocolPath}: missing required contract content '${required.replaceAll("\n", " ")}'.`,
      );
    }
  }
}

const agentsFile = files.find((candidate) => relative(candidate) === "AGENTS.md");
if (!agentsFile) {
  errors.push("AGENTS.md: missing repository instructions.");
} else {
  const agents = await readFile(agentsFile, "utf8");
  const normalizedAgents = agents.replaceAll(/\s+/gu, " ").toLowerCase();
  if (!agents.includes(`\`${engineeringProtocolPath}\``)) {
    errors.push(`AGENTS.md: must point to ${engineeringProtocolPath}.`);
  }
  if (
    !normalizedAgents.includes("classify") ||
    !normalizedAgents.includes("risk") ||
    !normalizedAgents.includes("do not create a new task")
  ) {
    errors.push(
      "AGENTS.md: must require risk classification and prohibit governance-only task creation.",
    );
  }
}

for (const file of files.filter((candidate) => candidate.endsWith(".js") || candidate.endsWith(".mjs"))) {
  const result = spawnSync(process.execPath, ["--check", file], {
    cwd: root,
    encoding: "utf8",
  });
  if (result.status !== 0) {
    errors.push(`${relative(file)}: ${result.stderr.trim()}`);
  }
}

for (const file of files.filter((candidate) => candidate.endsWith(".json"))) {
  try {
    JSON.parse(await readFile(file, "utf8"));
  } catch (error) {
    errors.push(`${relative(file)}: invalid JSON: ${error.message}`);
  }
}

const requiredConstitutionHeadings = [
  "Mission",
  "Non-Goals",
  "Owned Data",
  "Public Contracts",
  "Invariants",
  "Allowed Dependencies",
  "Forbidden Dependencies",
  "Failure, Compatibility, And Migration",
  "Acceptance Gates",
  "Change Policy",
  "Amendment History",
];

const constitutionFiles = files.filter((candidate) =>
  candidate.endsWith("MODULE_CONSTITUTION.md"),
);
if (constitutionFiles.length === 0) {
  errors.push("No MODULE_CONSTITUTION.md files found.");
}

const constitutionOwners = new Map();
for (const file of constitutionFiles) {
  const content = await readFile(file, "utf8");
  if (!content.startsWith("---\n")) {
    errors.push(`${relative(file)}: frontmatter must start at byte 0.`);
  }
  const frontmatterEnd = content.indexOf("\n---\n", 4);
  const frontmatter =
    content.startsWith("---\n") && frontmatterEnd >= 0
      ? content.slice(4, frontmatterEnd)
      : "";
  const moduleMatches = [
    ...frontmatter.matchAll(/^module_id:\s*([a-z0-9]+(?:-[a-z0-9]+)*)\s*$/gmu),
  ];
  if (moduleMatches.length !== 1) {
    errors.push(`${relative(file)}: expected exactly one canonical module_id.`);
  } else {
    const moduleId = moduleMatches[0][1];
    const canonicalPath = `docs/modules/${moduleId}/MODULE_CONSTITUTION.md`;
    if (relative(file) !== canonicalPath) {
      errors.push(
        `${relative(file)}: constitution path must be '${canonicalPath}'.`,
      );
    }
    const prior = constitutionOwners.get(moduleId);
    if (prior) {
      errors.push(
        `${relative(file)}: duplicate module_id '${moduleId}' also owned by ${prior}.`,
      );
    } else {
      constitutionOwners.set(moduleId, relative(file));
    }
  }
  for (const heading of requiredConstitutionHeadings) {
    if (!content.includes(`\n## ${heading}\n`)) {
      errors.push(`${relative(file)}: missing heading '${heading}'.`);
    }
  }
}

const ticketFiles = files.filter((candidate) => {
  const name = relative(candidate);
  return name.startsWith("docs/tickets/") && name.endsWith(".md");
});
const ticketOwners = new Map();
for (const file of ticketFiles) {
  const content = await readFile(file, "utf8");
  const frontmatterEnd = content.indexOf("\n---\n", 4);
  const frontmatter =
    content.startsWith("---\n") && frontmatterEnd >= 0
      ? content.slice(4, frontmatterEnd)
      : "";
  const matches = [...frontmatter.matchAll(/^ticket_id:\s*(TASK-[0-9]{3})\s*$/gmu)];
  if (matches.length !== 1) {
    errors.push(`${relative(file)}: expected exactly one TASK-nnn ticket_id.`);
    continue;
  }
  const ticketId = matches[0][1];
  const moduleMatches = [
    ...frontmatter.matchAll(/^module_id:\s*([a-z0-9]+(?:-[a-z0-9]+)*)\s*$/gmu),
  ];
  if (moduleMatches.length !== 1) {
    errors.push(`${relative(file)}: expected exactly one canonical module_id.`);
    continue;
  }
  const moduleId = moduleMatches[0][1];
  const branchMatches = [...frontmatter.matchAll(/^branch:\s*(\S+)\s*$/gmu)];
  const branch = branchMatches.length === 1 ? branchMatches[0][1] : null;
  const status = frontmatterScalar(frontmatter, "status");
  const prior = ticketOwners.get(ticketId);
  if (prior) {
    errors.push(
      `${relative(file)}: duplicate ticket_id '${ticketId}' also owned by ${prior.file}.`,
    );
  } else {
    ticketOwners.set(ticketId, {
      file: relative(file),
      moduleId,
      branch,
      status,
    });
  }
}

const plansFile = files.find((candidate) => relative(candidate) === "PLANS.md");
const currentGitBranchResult = spawnSync("git", ["branch", "--show-current"], {
  cwd: root,
  encoding: "utf8",
  windowsHide: true,
});
const currentGitBranch =
  currentGitBranchResult.status === 0 ? currentGitBranchResult.stdout.trim() : "";
const defaultGitBranchResult = spawnSync(
  "git",
  ["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"],
  { cwd: root, encoding: "utf8", windowsHide: true },
);
const defaultGitBranch = defaultGitBranchResult.status === 0
  ? defaultGitBranchResult.stdout.trim().replace(/^origin\//u, "")
  : "";
const parallelTaskMatch = currentGitBranch.match(
  /^feature\/(task-[0-9]{3})-[a-z0-9]+(?:-[a-z0-9]+)*$/u,
);
if (parallelTaskMatch) {
  const parallelTaskId = parallelTaskMatch[1].toUpperCase();
  const parallelTicket = ticketOwners.get(parallelTaskId);
  if (!parallelTicket) {
    errors.push(
      `parallel branch '${currentGitBranch}' has no matching unique ticket '${parallelTaskId}'.`,
    );
  } else {
    if (parallelTicket.branch !== currentGitBranch) {
      errors.push(
        `${parallelTicket.file}: parallel ticket branch '${parallelTicket.branch || ""}' does not match current Git branch '${currentGitBranch}'.`,
      );
    }
    if (!constitutionOwners.has(parallelTicket.moduleId)) {
      errors.push(
        `${parallelTicket.file}: parallel module '${parallelTicket.moduleId}' has no MODULE_CONSTITUTION.md.`,
      );
    }
    const parallelStatus = (parallelTicket.status.value || "").toLowerCase();
    if (!parallelTicket.status.valid || parallelStatus === "cancelled") {
      errors.push(`${parallelTicket.file}: cancelled or malformed task branches cannot resume.`);
    }
  }
}
let currentTaskMarkers = [];
if (!plansFile) {
  errors.push("PLANS.md: missing project plan.");
} else {
  const plans = await readFile(plansFile, "utf8");
  currentTaskMarkers = [
    ...plans.matchAll(/CURRENT (TASK-[0-9]{3})\b/gu),
  ];
  if (currentTaskMarkers.length > 1) {
    errors.push(
      `PLANS.md: expected at most one CURRENT TASK marker; found ${currentTaskMarkers.length}.`,
    );
  } else if (currentTaskMarkers.length === 1) {
    const currentTaskId = currentTaskMarkers[0][1];
    const currentTicket = ticketOwners.get(currentTaskId);
    if (!currentTicket) {
      errors.push(
        `PLANS.md: current task '${currentTaskId}' has no matching unique ticket.`,
      );
    } else {
      if (!currentTicket.branch) {
        errors.push(`${currentTicket.file}: current ticket requires exactly one branch.`);
      } else if (currentGitBranchResult.status !== 0) {
        errors.push(`${currentTicket.file}: current Git branch cannot be identified.`);
      }
    }
  }
}

if (errors.length > 0) {
  for (const error of errors) {
    process.stderr.write(`${error}\n`);
  }
  process.exitCode = 1;
} else {
  process.stdout.write(
    `check=ok files=${files.length} constitutions=${constitutionFiles.length} tickets=${ticketOwners.size} current_tasks=${currentTaskMarkers.length}\n`,
  );
}
