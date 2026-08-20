import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

const checkScript = path.resolve("scripts/check-project.mjs");
const engineeringProtocol = `---
protocol_id: LATTICE_ENGINEERING_PROTOCOL
version: 1.1.0
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
After the clean logical commit, run npm.cmd run delivery:finish instead of an ordinary manual push. Only LATTICE_DELIVERY_READY_TO_ARCHIVE=1 permits Codex to call the native archive-task action; every failure keeps the task open.

## Knowledge Routing
Personal preferences, historical cases, and detailed decision logic belong in LATTICE, Hermes, and the knowledge graph.

## Authority Boundary
Preserve authority and safety boundaries.
`;
const agents = `# Fixture Instructions

Before editing, read \`docs/contracts/ENGINEERING_PROTOCOL_V1.md\`.
Before claiming completion, reread it and run the focused checks.
After the clean logical commit, run \`npm.cmd run delivery:finish\`; archive the current Codex task only after \`LATTICE_DELIVERY_READY_TO_ARCHIVE=1\`.
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
  deliveryPush = "local_only",
  deliveryArchive = "keep_open",
  deliveryRemote = "origin",
  deliveryRepository = "github.com/example/fixture",
  gitBranch,
  detachedGitHead = false,
  defaultGitBranch,
  nestedGitWorktree = false,
}) {
  const fixtureContainer = await mkdtemp(path.join(tmpdir(), "lattice-project-check-"));
  const root = nestedGitWorktree ? path.join(fixtureContainer, "child-worktree") : fixtureContainer;
  try {
    if (nestedGitWorktree) {
      await mkdir(root, { recursive: true });
      await writeFile(path.join(fixtureContainer, ".git"), "gitdir: fixture-parent\n", "utf8");
    }
    const currentTask = plans.match(/CURRENT (TASK-[0-9]{3})\b/u)?.[1] || tickets[0]?.[1];
    const fixtureGitBranch = gitBranch || `feature/${String(currentTask).toLowerCase()}-fixture`;
    const gitInit = spawnSync("git", ["init", "-b", fixtureGitBranch], {
      cwd: root,
      encoding: "utf8",
    });
    assert.equal(gitInit.status, 0, gitInit.stderr);
    if (defaultGitBranch) {
      const defaultRef = spawnSync(
        "git",
        ["symbolic-ref", "refs/remotes/origin/HEAD", `refs/remotes/origin/${defaultGitBranch}`],
        { cwd: root, encoding: "utf8" },
      );
      assert.equal(defaultRef.status, 0, defaultRef.stderr);
    }
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
    for (const [name, ticketId, moduleId = "fixture", overrides = {}] of tickets) {
      const branch = overrides.branch || `feature/${ticketId.toLowerCase()}-fixture`;
      const ticketStatus = overrides.status || "complete";
      await writeFile(
        path.join(root, "docs", "tickets", name),
        `---\nticket_id: ${ticketId}\nmodule_id: ${moduleId}\nstatus: ${ticketStatus}\nallowed_paths:\n${includeGuideAllowedPath ? "  - tools/engineering-status-dashboard/branch-guide.zh-TW.json\n" : ""}${decoyGuidePath ? "other_paths:\n  - tools/engineering-status-dashboard/branch-guide.zh-TW.json\n" : ""}branch: ${branch}\n${deliveryRemote === null ? "" : `delivery_remote: ${deliveryRemote}\n`}${deliveryRepository === null ? "" : `delivery_repository: ${deliveryRepository}\n`}${deliveryPush === null ? "" : `delivery_push: ${deliveryPush}\n`}${deliveryArchive === null ? "" : `delivery_archive: ${deliveryArchive}\n`}---\n`,
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
    if (detachedGitHead) {
      const commit = spawnSync(
        "git",
        [
          "-c",
          "user.name=Fixture",
          "-c",
          "user.email=fixture@example.invalid",
          "commit",
          "--allow-empty",
          "-m",
          "fixture detached check",
        ],
        { cwd: root, encoding: "utf8" },
      );
      assert.equal(commit.status, 0, commit.stderr);
      const detach = spawnSync("git", ["switch", "--detach"], {
        cwd: root,
        encoding: "utf8",
      });
      assert.equal(detach.status, 0, detach.stderr);
    }
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
    await rm(fixtureContainer, { recursive: true, force: true });
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

test("project check requires the one-command delivery and archive-success rule", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    protocol: engineeringProtocol.replace(
      "After the clean logical commit, run npm.cmd run delivery:finish instead of an ordinary manual push. Only LATTICE_DELIVERY_READY_TO_ARCHIVE=1 permits Codex to call the native archive-task action; every failure keeps the task open.",
      "Finish manually.",
    ),
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /npm\.cmd run delivery:finish/u);
  assert.match(result.stderr, /LATTICE_DELIVERY_READY_TO_ARCHIVE=1/u);
});

test("project check requires current ticket push and archive policies", async () => {
  for (const fixtureOptions of [
    { deliveryPush: null },
    { deliveryPush: "push_everything" },
    { deliveryArchive: null },
    { deliveryArchive: "always_archive" },
    { deliveryRemote: null },
    { deliveryRemote: "--force" },
    { deliveryRepository: null },
    { deliveryRepository: "https://user:secret@example.invalid/repo" },
  ]) {
    const result = await runFixture({
      tickets: [["one.md", "TASK-017"]],
      plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
      ...fixtureOptions,
    });
    assert.equal(result.status, 1, JSON.stringify(fixtureOptions));
    assert.match(result.stderr, /delivery_(?:push|archive|remote|repository)/u);
  }
});

test("project check requires AGENTS to route successful delivery to native archival", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    agentsContent: agents.replace(
      "After the clean logical commit, run `npm.cmd run delivery:finish`; archive the current Codex task only after `LATTICE_DELIVERY_READY_TO_ARCHIVE=1`.",
      "Finish manually.",
    ),
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /AGENTS\.md.*delivery:finish/u);
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

test("project check permits a detached read-only integration verification", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    detachedGitHead: true,
  });

  assert.equal(result.status, 0, result.stderr);
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

test("project check accepts TASK-081, TASK-082, and TASK-083 parallel branches without changing PLANS", async () => {
  const parallelBranches = [
    ["TASK-081", "feature/task-081-dashboard-identity-reconciliation"],
    ["TASK-082", "feature/task-082-task-050-terminal-evidence"],
    ["TASK-083", "feature/task-083-task-075-terminal-evidence"],
  ];
  for (const [taskId, branch] of parallelBranches) {
    const result = await runFixture({
      tickets: [
        ["current.md", "TASK-078"],
        [`${taskId.toLowerCase()}.md`, taskId, "fixture", { branch }],
      ],
      plans: "**CURRENT TASK-078 IMPLEMENTATION:** shared planning index\n",
      gitBranch: branch,
    });

    assert.equal(result.status, 0, `${taskId}: ${result.stderr}`);
    assert.match(result.stdout, /current_tasks=1/u);
  }
});

test("project check rejects a parallel TASK branch without a matching ticket", async () => {
  const result = await runFixture({
    tickets: [["current.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** shared planning index\n",
    gitBranch: "feature/task-018-fixture",
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /parallel branch 'feature\/task-018-fixture' has no matching unique ticket 'TASK-018'/u);
});

test("project check rejects a parallel TASK ticket with a branch mismatch", async () => {
  const result = await runFixture({
    tickets: [
      ["current.md", "TASK-017"],
      ["parallel.md", "TASK-018", "fixture", { branch: "feature/task-018-other" }],
    ],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** shared planning index\n",
    gitBranch: "feature/task-018-fixture",
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /parallel ticket branch 'feature\/task-018-other' does not match current Git branch 'feature\/task-018-fixture'/u);
});

test("project check rejects a non-terminal parallel TASK ticket", async () => {
  const result = await runFixture({
    tickets: [
      ["current.md", "TASK-017"],
      ["parallel.md", "TASK-018", "fixture", { status: "in_progress" }],
    ],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** shared planning index\n",
    gitBranch: "feature/task-018-fixture",
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /parallel ticket must be terminal/u);
});

test("project check keeps a cancelled TASK-038 branch denied", async () => {
  const result = await runFixture({
    tickets: [
      ["current.md", "TASK-017"],
      ["task038.md", "TASK-038", "fixture", { status: "cancelled" }],
    ],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** shared planning index\n",
    gitBranch: "feature/task-038-fixture",
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /parallel ticket must be terminal/u);
});

test("project check rejects an unauthorized parallel delivery policy and the default branch", async () => {
  const unauthorized = await runFixture({
    tickets: [
      ["current.md", "TASK-017"],
      ["parallel.md", "TASK-018"],
    ],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** shared planning index\n",
    deliveryPush: "unauthorized_push",
    gitBranch: "feature/task-018-fixture",
  });
  const defaultBranch = await runFixture({
    tickets: [["current.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** shared planning index\n",
    gitBranch: "main",
    defaultGitBranch: "main",
  });

  assert.equal(unauthorized.status, 1);
  assert.match(unauthorized.stderr, /delivery_push must be 'authorized_non_force_feature_branch' or 'local_only'/u);
  assert.equal(defaultBranch.status, 1);
  assert.match(defaultBranch.stderr, /current Git branch 'main' must not be the default branch/u);
});

test("project check rejects a worktree nested inside another Git worktree", async () => {
  const result = await runFixture({
    tickets: [["current.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    nestedGitWorktree: true,
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /worktree root must not be nested inside another Git worktree/u);
});
