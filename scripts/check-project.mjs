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

const files = await walk(root);
const errors = [];
let runtimeContractChecked = false;

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
  const normalizedProtocol = protocol.replaceAll(/\s+/gu, " ");
  const requiredProtocolContent = [
    "protocol_id: LATTICE_ENGINEERING_PROTOCOL",
    "version: 2.0.0",
    "canonical_path: docs/contracts/ENGINEERING_PROTOCOL_V1.md",
    "## Entry",
    "## Product priority",
    "## Complexity circuit breaker",
    "Do not create another task only to repair governance",
    "Do not require all optional modules in one acceptance",
    "After two failed attempts at the same acceptance",
    "## Verification",
    "Tests prove only what they execute",
    "## Delivery and authority",
    "Ordinary local completion does not require a ticket",
    "default-branch mutation",
  ];
  for (const required of requiredProtocolContent) {
    if (!normalizedProtocol.includes(required)) {
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
  const normalizedAgents = agents.replaceAll(/\s+/gu, " ");
  if (!agents.includes(`\`${engineeringProtocolPath}\``)) {
    errors.push(`AGENTS.md: must point to ${engineeringProtocolPath}.`);
  }
  if (
    !normalizedAgents.includes("Do not create a TASK") ||
    !normalizedAgents.includes("only to satisfy workflow") ||
    !normalizedAgents.includes("Do not require every module or external service to pass in one run")
  ) {
    errors.push(
      "AGENTS.md: must prohibit governance-only task creation and all-module acceptance.",
    );
  }
}

const runtimeContract = spawnSync(
  "cargo",
  ["test", "-p", "lattice-core", "--test", "platform_manifest"],
  {
    cwd: root,
    encoding: "utf8",
  },
);
if (runtimeContract.status !== 0) {
  const detail = [runtimeContract.stdout, runtimeContract.stderr]
    .filter(Boolean)
    .join("\n")
    .trim();
  errors.push(
    `Runtime contract test failed${detail ? `: ${detail}` : "."}`,
  );
} else {
  runtimeContractChecked = true;
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
  const prior = ticketOwners.get(ticketId);
  if (prior) {
    errors.push(
      `${relative(file)}: duplicate ticket_id '${ticketId}' also owned by ${prior.file}.`,
    );
  } else {
    ticketOwners.set(ticketId, { file: relative(file) });
  }
}

const plansFile = files.find((candidate) => relative(candidate) === "PLANS.md");
let currentTaskCount = 0;
if (!plansFile) {
  errors.push("PLANS.md: missing project plan.");
} else {
  const plans = await readFile(plansFile, "utf8");
  const currentTaskMarkers = [
    ...plans.matchAll(/CURRENT (TASK-[0-9]{3})\b/gu),
  ];
  currentTaskCount = currentTaskMarkers.length;
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
    `check=ok files=${files.length} constitutions=${constitutionFiles.length} tickets=${ticketOwners.size} current_tasks=${currentTaskCount} runtime_contract=${runtimeContractChecked ? "ok" : "not-run"}\n`,
  );
}
