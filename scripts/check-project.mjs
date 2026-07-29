import { readFile, readdir } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";
import process from "node:process";

const root = process.cwd();
const ignoredDirectories = new Set([".git", "node_modules", "coverage"]);

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

for (const file of constitutionFiles) {
  const content = await readFile(file, "utf8");
  if (!content.startsWith("---\n")) {
    errors.push(`${relative(file)}: frontmatter must start at byte 0.`);
  }
  for (const heading of requiredConstitutionHeadings) {
    if (!content.includes(`\n## ${heading}\n`)) {
      errors.push(`${relative(file)}: missing heading '${heading}'.`);
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
    `check=ok files=${files.length} constitutions=${constitutionFiles.length}\n`,
  );
}

