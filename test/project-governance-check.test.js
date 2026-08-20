import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

const checkScript = path.resolve("scripts/check-project.mjs");
const engineeringProtocol = `---
protocol_id: LATTICE_ENGINEERING_PROTOCOL
version: 1.0.3
status: active
canonical_path: docs/contracts/ENGINEERING_PROTOCOL_V1.md
---

# Fixture Engineering Protocol

## Mandatory Entry
Read before work.

## Mandatory Delivery
If an ordinary reproducible check fails, repair it within the authorized scope and rerun the same failed check.
After the durable handoff is current, run npm.cmd run status:refresh; the projection never replaces ticket, Git, test, CI, review, or LATTICE acceptance evidence.
Every new branch must add a plain Traditional-Chinese name and purpose to tools/engineering-status-dashboard/branch-guide.zh-TW.json and include that path in the active ticket \`allowed_paths\`.

## Knowledge Routing
Personal preferences, historical cases, and detailed decision logic belong in LATTICE, Hermes, and the knowledge graph.

## Authority Boundary
Preserve authority and safety boundaries.
`;
const agents = `# Fixture Instructions

Before editing, read \`docs/contracts/ENGINEERING_PROTOCOL_V1.md\`.
Before claiming completion, reread it and run the focused checks.
`;
const constitution = `---
module_id: fixture
name: Fixture
version: 1.0
status: active
---

# Fixture

## Mission
Fixture.

## Non-Goals
None.

## Owned Data
None.

## Public Contracts
None.

## Invariants
None.

## Allowed Dependencies
None.

## Forbidden Dependencies
None.

## Failure, Compatibility, And Migration
None.

## Acceptance Gates
None.

## Change Policy
None.

## Amendment History
None.
`;

async function runFixture({
  tickets,
  plans,
  protocol = engineeringProtocol,
  repairProtocol,
  agentsContent = agents,
  constitutionPath = path.join(
    "docs",
    "modules",
    "fixture",
    "MODULE_CONSTITUTION.md",
  ),
  includeGuideAllowedPath = true,
  includeGuideEntry = true,
  guideEntryUsesChinese = true,
  decoyGuidePath = false,
  gitBranch,
}) {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-project-check-"));
  try {
    const currentTask = plans.match(/CURRENT (TASK-[0-9]{3})\b/u)?.[1] || tickets[0]?.[1];
    const fixtureGitBranch = gitBranch || `feature/${String(currentTask).toLowerCase()}-fixture`;
    const gitInit = spawnSync("git", ["init", "-b", fixtureGitBranch], {
      cwd: root,
      encoding: "utf8",
    });
    assert.equal(gitInit.status, 0, gitInit.stderr);
    const constitutionFile = path.join(root, constitutionPath);
    await mkdir(path.dirname(constitutionFile), { recursive: true });
    await mkdir(path.join(root, "docs", "contracts"), { recursive: true });
    await mkdir(path.join(root, "docs", "tickets"), { recursive: true });
    await mkdir(
      path.join(root, "tools", "engineering-status-dashboard"),
      { recursive: true },
    );
    await writeFile(constitutionFile, constitution, "utf8");
    if (protocol !== null) {
      await writeFile(
        path.join(root, "docs", "contracts", "ENGINEERING_PROTOCOL_V1.md"),
        protocol,
        "utf8",
      );
    }
    if (agentsContent !== null) {
      await writeFile(path.join(root, "AGENTS.md"), agentsContent, "utf8");
    }
    await writeFile(path.join(root, "PLANS.md"), plans, "utf8");
    const guideBranches = {};
    for (const [name, ticketId, moduleId = "fixture"] of tickets) {
      const branch = `feature/${ticketId.toLowerCase()}-fixture`;
      await writeFile(
        path.join(root, "docs", "tickets", name),
        `---\nticket_id: ${ticketId}\nmodule_id: ${moduleId}\nallowed_paths:\n${includeGuideAllowedPath ? "  - tools/engineering-status-dashboard/branch-guide.zh-TW.json\n" : ""}${decoyGuidePath ? "other_paths:\n  - tools/engineering-status-dashboard/branch-guide.zh-TW.json\n" : ""}branch: ${branch}\n---\n`,
        "utf8",
      );
      if (includeGuideEntry) {
        guideBranches[branch] = {
          name: guideEntryUsesChinese ? `${ticketId} 中文名稱` : `${ticketId} English Name`,
          purpose: guideEntryUsesChinese
            ? "用繁體中文說明這條測試分支的用途。"
            : "English purpose only.",
        };
      }
    }
    await writeFile(
      path.join(root, "tools", "engineering-status-dashboard", "branch-guide.zh-TW.json"),
      `${JSON.stringify({
        schema: "lattice.branch-guide.zh-TW/1.0",
        branches: guideBranches,
      }, null, 2)}\n`,
      "utf8",
    );
    const initial = spawnSync(process.execPath, [checkScript], {
      cwd: root,
      encoding: "utf8",
    });
    if (repairProtocol === undefined) {
      return initial;
    }
    await writeFile(
      path.join(root, "docs", "contracts", "ENGINEERING_PROTOCOL_V1.md"),
      repairProtocol,
      "utf8",
    );
    const rerun = spawnSync(process.execPath, [checkScript], {
      cwd: root,
      encoding: "utf8",
    });
    return { initial, rerun };
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("project check rejects duplicate ticket IDs", async () => {
  const result = await runFixture({
    tickets: [
      ["one.md", "TASK-017"],
      ["two.md", "TASK-017"],
    ],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /duplicate ticket_id 'TASK-017'/u);
});

test("project check requires the readable versioned engineering protocol", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    protocol: null,
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /missing engineering protocol/u);
});

test("project check requires AGENTS to point to the engineering protocol", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    agentsContent: "# Fixture Instructions\n",
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /AGENTS\.md: must point to docs\/contracts\/ENGINEERING_PROTOCOL_V1\.md/u);
});

test("project check requires detailed knowledge to route outside the protocol", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    protocol: engineeringProtocol.replace(
      "Personal preferences, historical cases, and detailed decision logic belong in LATTICE, Hermes, and the knowledge graph.",
      "Keep every rule here.",
    ),
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /missing required contract content/u);
});

test("project check enforces the post-handoff dashboard refresh rule", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    protocol: engineeringProtocol.replace(
      "After the durable handoff is current, run npm.cmd run status:refresh; the projection never replaces ticket, Git, test, CI, review, or LATTICE acceptance evidence.",
      "No local projection is required.",
    ),
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /npm\.cmd run status:refresh/u);
});

test("project check requires the protocol to describe the Chinese purpose guide", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    protocol: engineeringProtocol.replace(
      "Every new branch must add a plain Traditional-Chinese name and purpose to tools/engineering-status-dashboard/branch-guide.zh-TW.json and include that path in the active ticket `allowed_paths`.",
      "Branch explanations are optional.",
    ),
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /branch-guide\.zh-TW\.json/u);
});

test("project check requires the current branch in the Chinese purpose guide", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    includeGuideEntry: false,
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /missing plain Traditional-Chinese name and purpose/u);
});

test("project check requires the guide in current ticket allowed_paths", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    includeGuideAllowedPath: false,
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /allowed_paths must include/u);
});

test("project check does not accept the guide path under another frontmatter list", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    includeGuideAllowedPath: false,
    decoyGuidePath: true,
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /allowed_paths must include/u);
});

test("project check requires current ticket branch to match Git", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    gitBranch: "feature/different-fixture",
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /does not match current Git branch/u);
});

test("project check rejects English-only guide text", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    guideEntryUsesChinese: false,
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /missing plain Traditional-Chinese name and purpose/u);
});

test("an ordinary protocol error can be repaired and the same check rerun", async () => {
  const broken = engineeringProtocol.replace(
    "repair it within the authorized scope and rerun the same failed check.",
    "close the task immediately.",
  );
  const { initial, rerun } = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    protocol: broken,
    repairProtocol: engineeringProtocol,
  });
  assert.equal(initial.status, 1);
  assert.match(initial.stderr, /missing required contract content/u);
  assert.equal(rerun.status, 0, rerun.stderr);
  assert.match(rerun.stdout, /check=ok/u);
});

test("project check requires exactly one current-task marker", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans:
      "**CURRENT TASK-017 IMPLEMENTATION:** one\n**CURRENT TASK-018 GOVERNANCE:** two\n",
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /PLANS\.md: expected exactly one CURRENT TASK marker/u);
});

test("project check requires a ticket matching the current-task marker", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-018 GOVERNANCE:** missing ticket\n",
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /current task 'TASK-018' has no matching unique ticket/u);
});

test("project check requires a constitution for the current ticket module", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-018", "postgres-store"]],
    plans: "**CURRENT TASK-018 GOVERNANCE:** missing constitution\n",
  });

  assert.equal(result.status, 1);
  assert.match(
    result.stderr,
    /current module 'postgres-store' has no MODULE_CONSTITUTION\.md/u,
  );
});

test("project check rejects a current-module constitution outside its canonical path", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-018"]],
    plans: "**CURRENT TASK-018 GOVERNANCE:** wrong constitution path\n",
    constitutionPath: path.join("scratch", "MODULE_CONSTITUTION.md"),
  });

  assert.equal(result.status, 1);
  assert.match(
    result.stderr,
    /constitution path must be 'docs\/modules\/fixture\/MODULE_CONSTITUTION\.md'/u,
  );
});

test("project check accepts unique tickets and one current-task marker", async () => {
  const result = await runFixture({
    tickets: [
      ["one.md", "TASK-017"],
      ["two.md", "TASK-018", "inactive-future-module"],
    ],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
  });

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /check=ok/u);
});
