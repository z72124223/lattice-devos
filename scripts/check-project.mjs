import { readFile, readdir } from "node:fs/promises";
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

function frontmatterListIncludes(frontmatter, key, expectedValue) {
  const lines = frontmatter.split(/\r?\n/gu);
  const start = lines.findIndex((line) => line === `${key}:`);
  if (start < 0) return false;
  for (let index = start + 1; index < lines.length; index += 1) {
    const line = lines[index];
    if (/^[a-zA-Z0-9_]+:/u.test(line)) break;
    const item = line.match(/^\s+-\s*(.*?)\s*$/u);
    if (item?.[1] === expectedValue) return true;
  }
  return false;
}

const files = await walk(root);
const errors = [];

const engineeringProtocolPath = "docs/contracts/ENGINEERING_PROTOCOL_V1.md";
const engineeringProtocolFile = files.find(
  (candidate) => relative(candidate) === engineeringProtocolPath,
);
if (!engineeringProtocolFile) {
  errors.push(`${engineeringProtocolPath}: missing engineering protocol.`);
} else {
  const protocol = await readFile(engineeringProtocolFile, "utf8");
  const requiredProtocolContent = [
    "protocol_id: LATTICE_ENGINEERING_PROTOCOL",
    "version: 1.0.3",
    "canonical_path: docs/contracts/ENGINEERING_PROTOCOL_V1.md",
    "## Mandatory Entry",
    "## Mandatory Delivery",
    "repair it within the authorized scope and rerun the same failed check",
    "npm.cmd run status:refresh",
    "the projection never replaces ticket, Git, test, CI",
    "LATTICE acceptance evidence",
    "plain Traditional-Chinese name and purpose",
    "tools/engineering-status-dashboard/branch-guide.zh-TW.json",
    "active ticket `allowed_paths`",
    "## Knowledge Routing",
    "Personal preferences, historical cases, and detailed decision logic belong in LATTICE, Hermes, and the knowledge graph",
    "## Authority Boundary",
  ];
  for (const required of requiredProtocolContent) {
    if (!protocol.includes(required)) {
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
  if (!agents.includes(`\`${engineeringProtocolPath}\``)) {
    errors.push(`AGENTS.md: must point to ${engineeringProtocolPath}.`);
  }
  if (!agents.includes("Before editing") || !agents.includes("Before claiming completion")) {
    errors.push("AGENTS.md: must require engineering protocol checks before editing and completion.");
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
  const includesBranchGuide = frontmatterListIncludes(
    frontmatter,
    "allowed_paths",
    "tools/engineering-status-dashboard/branch-guide.zh-TW.json",
  );
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
      includesBranchGuide,
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
if (!plansFile) {
  errors.push("PLANS.md: missing project plan.");
} else {
  const plans = await readFile(plansFile, "utf8");
  const currentTaskMarkers = [
    ...plans.matchAll(/CURRENT (TASK-[0-9]{3})\b/gu),
  ];
  if (currentTaskMarkers.length !== 1) {
    errors.push(
      `PLANS.md: expected exactly one CURRENT TASK marker; found ${currentTaskMarkers.length}.`,
    );
  } else {
    const currentTaskId = currentTaskMarkers[0][1];
    const currentTicket = ticketOwners.get(currentTaskId);
    if (!currentTicket) {
      errors.push(
        `PLANS.md: current task '${currentTaskId}' has no matching unique ticket.`,
      );
    } else if (!constitutionOwners.has(currentTicket.moduleId)) {
      errors.push(
        `${currentTicket.file}: current module '${currentTicket.moduleId}' has no MODULE_CONSTITUTION.md.`,
      );
    } else {
      const branchGuidePath = "tools/engineering-status-dashboard/branch-guide.zh-TW.json";
      if (!currentTicket.branch) {
        errors.push(`${currentTicket.file}: current ticket requires exactly one branch.`);
      } else if (currentGitBranchResult.status !== 0) {
        errors.push(`${currentTicket.file}: current Git branch cannot be identified.`);
      } else if (
        currentGitBranch &&
        currentGitBranch !== defaultGitBranch &&
        currentTicket.branch !== currentGitBranch
      ) {
        errors.push(
          `${currentTicket.file}: ticket branch '${currentTicket.branch}' does not match current Git branch '${currentGitBranch}'.`,
        );
      }
      if (!currentTicket.includesBranchGuide) {
        errors.push(
          `${currentTicket.file}: current ticket allowed_paths must include '${branchGuidePath}'.`,
        );
      }
      const branchGuideFile = files.find((candidate) => relative(candidate) === branchGuidePath);
      if (!branchGuideFile) {
        errors.push(`${branchGuidePath}: missing Traditional-Chinese branch guide.`);
      } else if (currentTicket.branch) {
        try {
          const guide = JSON.parse(await readFile(branchGuideFile, "utf8"));
          const entry = guide?.branches?.[currentTicket.branch];
          if (
            guide?.schema !== "lattice.branch-guide.zh-TW/1.0" ||
            typeof entry?.name !== "string" ||
            !entry.name.trim() ||
            !/\p{Script=Han}/u.test(entry.name) ||
            typeof entry?.purpose !== "string" ||
            !entry.purpose.trim() ||
            !/\p{Script=Han}/u.test(entry.purpose)
          ) {
            errors.push(
              `${branchGuidePath}: missing plain Traditional-Chinese name and purpose for '${currentTicket.branch}'.`,
            );
          }
        } catch {
          // The generic JSON validator already reports the parse failure.
        }
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
    `check=ok files=${files.length} constitutions=${constitutionFiles.length} tickets=${ticketOwners.size} current_tasks=1\n`,
  );
}
