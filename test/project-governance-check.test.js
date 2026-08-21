import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

const checkScript = path.resolve("scripts/check-project.mjs");
const engineeringProtocol = `---
protocol_id: LATTICE_ENGINEERING_PROTOCOL
version: 2.0.0
status: active
canonical_path: docs/contracts/ENGINEERING_PROTOCOL_V1.md
---

# Fixture Engineering Protocol

## Safety Boundaries
Never force-push, mutate the default branch, deploy, release, expose secrets, or
perform destructive cleanup without explicit authority.

## Risk Tiers
Use focused checks for ordinary work. Reserve full integration, live services,
and independent review for changes whose actual risk requires them.

## Complexity Circuit Breaker
Dashboard, ticket, validator, finisher, and handoff defects do not by themselves
make product behavior fail. Do not create another task only to repair governance
for the current task. After one governance repair, record a known limitation or
simplify the gate.

## Delivery
Use npm.cmd run delivery:finish only for an explicitly authorized remote feature
delivery. LATTICE_DELIVERY_READY_TO_ARCHIVE=1 permits archival after that optional
delivery path; ordinary local completion does not require the finisher.
`;
const agents = `# Fixture Instructions

Before editing, read \`docs/contracts/ENGINEERING_PROTOCOL_V1.md\` and classify
the actual risk. Prefer the smallest verification that proves the change. Do not
create a new task for dashboard, ticket, validator, finisher, or handoff repair.
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
      const displayMetadata = overrides.displayName === undefined && overrides.displayPurpose === undefined
        ? ""
        : `display_name_zh_tw: ${overrides.displayName ?? ""}\ndisplay_purpose_zh_tw: ${overrides.displayPurpose ?? ""}\n`;
      await writeFile(
        path.join(root, "docs", "tickets", name),
        `---\nticket_id: ${ticketId}\nmodule_id: ${moduleId}\nstatus: ${ticketStatus}\n${displayMetadata}allowed_paths:\n${includeGuideAllowedPath ? "  - tools/engineering-status-dashboard/branch-guide.zh-TW.json\n" : ""}${decoyGuidePath ? "other_paths:\n  - tools/engineering-status-dashboard/branch-guide.zh-TW.json\n" : ""}branch: ${branch}\n${deliveryRemote === null ? "" : `delivery_remote: ${deliveryRemote}\n`}${deliveryRepository === null ? "" : `delivery_repository: ${deliveryRepository}\n`}${deliveryPush === null ? "" : `delivery_push: ${deliveryPush}\n`}${deliveryArchive === null ? "" : `delivery_archive: ${deliveryArchive}\n`}${overrides.extraFrontmatter || ""}---\n`,
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

test("project check requires the complexity circuit breaker", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    protocol: engineeringProtocol.replace(
      "Do not create another task only to repair governance",
      "Create another task whenever governance fails",
    ),
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /missing required contract content/u);
});

test("project check does not require a dashboard refresh for ordinary work", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    protocol: engineeringProtocol,
  });

  assert.equal(result.status, 0, result.stderr);
});

test("project check keeps the finisher optional for ordinary local completion", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    protocol: engineeringProtocol,
  });

  assert.equal(result.status, 0, result.stderr);
});

test("project check leaves delivery policy validation to the explicit finisher", async () => {
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
    assert.equal(result.status, 0, `${JSON.stringify(fixtureOptions)}: ${result.stderr}`);
  }
});

test("project check requires AGENTS to prohibit governance-only tasks", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    agentsContent: agents.replace(
      "Do not\ncreate a new task for dashboard, ticket, validator, finisher, or handoff repair.",
      "Create another task for every governance repair.",
    ),
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /governance-only task creation/u);
});

test("project check does not require dashboard presentation policy in the engineering protocol", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    protocol: engineeringProtocol,
  });

  assert.equal(result.status, 0, result.stderr);
});

test("project check does not require provenance policy for ordinary verification", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    protocol: engineeringProtocol,
  });

  assert.equal(result.status, 0, result.stderr);
});

test("project check treats a missing dashboard guide entry as non-blocking", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    includeGuideEntry: false,
  });

  assert.equal(result.status, 0, result.stderr);
});

test("project check does not expand allowed_paths only for dashboard presentation", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    includeGuideAllowedPath: false,
  });

  assert.equal(result.status, 0, result.stderr);
});

test("project check ignores dashboard guide placement during product verification", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    includeGuideAllowedPath: false,
    decoyGuidePath: true,
  });

  assert.equal(result.status, 0, result.stderr);
});

test("project check does not let a shared plan lock an unrelated maintenance branch", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    gitBranch: "feature/different-fixture",
  });

  assert.equal(result.status, 0, result.stderr);
});

test("project check permits a detached read-only integration verification", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    detachedGitHead: true,
  });

  assert.equal(result.status, 0, result.stderr);
});

test("project check does not block product verification on dashboard translation", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** fixture\n",
    guideEntryUsesChinese: false,
  });

  assert.equal(result.status, 0, result.stderr);
});

test("a missing complexity circuit breaker can be repaired and the same check rerun", async () => {
  const broken = engineeringProtocol.replace(
    "Do not create another task only to repair governance",
    "Create another task whenever governance fails",
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

test("project check permits zero or one current-task marker but rejects two", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans:
      "**CURRENT TASK-017 IMPLEMENTATION:** one\n**CURRENT TASK-018 GOVERNANCE:** two\n",
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /PLANS\.md: expected at most one CURRENT TASK marker/u);
});

test("project check requires a ticket matching the current-task marker", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017"]],
    plans: "**CURRENT TASK-018 GOVERNANCE:** missing ticket\n",
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /current task 'TASK-018' has no matching unique ticket/u);
});

test("project check does not require a constitution only to run ordinary verification", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-018", "postgres-store"]],
    plans: "**CURRENT TASK-018 GOVERNANCE:** missing constitution\n",
    gitBranch: "maintenance/ordinary-verification",
  });

  assert.equal(result.status, 0, result.stderr);
});

test("project check requires a module contract for the task branch being changed", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-018", "missing-module"]],
    plans: "# Shared plan without a global task lock\n",
    gitBranch: "feature/task-018-fixture",
  });

  assert.equal(result.status, 1);
  assert.match(result.stderr, /parallel module 'missing-module' has no MODULE_CONSTITUTION\.md/u);
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

test("project check accepts ticket-local display metadata for parallel tasks without the shared guide", async () => {
  const parallelBranches = [
    ["TASK-081", "feature/task-081-dashboard-identity-reconciliation"],
    ["TASK-082", "feature/task-082-task-050-terminal-evidence"],
    ["TASK-083", "feature/task-083-task-075-terminal-evidence"],
  ];
  for (const [taskId, branch] of parallelBranches) {
    const result = await runFixture({
      tickets: [
        ["current.md", "TASK-078", "fixture", {
          displayName: "目前規劃工作",
          displayPurpose: "保留唯一規劃焦點的繁體中文說明。",
        }],
        [`${taskId.toLowerCase()}.md`, taskId, "fixture", {
          branch,
          displayName: `${taskId} 平行工作`,
          displayPurpose: "從票券本身提供繁體中文交付用途。",
        }],
      ],
      plans: "**CURRENT TASK-078 IMPLEMENTATION:** shared planning index\n",
      gitBranch: branch,
      includeGuideAllowedPath: false,
      includeGuideEntry: false,
    });

    assert.equal(result.status, 0, `${taskId}: ${result.stderr}`);
  }
});

test("project check accepts TASK-081, TASK-082, and TASK-083 ticket-local metadata with a legacy shared protocol", async () => {
  const legacyProtocol = engineeringProtocol.replace(
    "Every new branch must provide a plain Traditional-Chinese name and purpose through its own exactly-once non-empty `display_name_zh_tw` and `display_purpose_zh_tw` ticket fields. The shared tools/engineering-status-dashboard/branch-guide.zh-TW.json is a legacy branch-guide fallback only: a ticket without both local fields must include that path in its `allowed_paths`.",
    "Every new branch must add a plain Traditional-Chinese name and purpose to tools/engineering-status-dashboard/branch-guide.zh-TW.json and include that path in the active ticket `allowed_paths`.",
  );
  const parallelBranches = [
    ["TASK-081", "feature/task-081-dashboard-identity-reconciliation"],
    ["TASK-082", "feature/task-082-task-050-terminal-evidence"],
    ["TASK-083", "feature/task-083-task-075-terminal-evidence"],
  ];
  for (const [taskId, branch] of parallelBranches) {
    const result = await runFixture({
      tickets: [
        ["current.md", "TASK-078", "fixture", {
          displayName: "目前規劃工作",
          displayPurpose: "保留唯一規劃焦點的繁體中文說明。",
        }],
        [`${taskId.toLowerCase()}.md`, taskId, "fixture", {
          branch,
          displayName: `${taskId} 平行工作`,
          displayPurpose: "從票券本身提供繁體中文交付用途。",
        }],
      ],
      plans: "**CURRENT TASK-078 IMPLEMENTATION:** shared planning index\n",
      gitBranch: branch,
      includeGuideAllowedPath: false,
      includeGuideEntry: false,
      protocol: legacyProtocol,
    });

    assert.equal(result.status, 0, `${taskId}: ${result.stderr}`);
  }
});

test("project check keeps dashboard display metadata outside product verification", async () => {
  const result = await runFixture({
    tickets: [
      ["current.md", "TASK-017", "fixture", {
        displayName: "目前工作",
        displayPurpose: "目前工作用途。",
      }],
      ["parallel.md", "TASK-018"],
    ],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** shared planning index\n",
    gitBranch: "feature/task-018-fixture",
    includeGuideAllowedPath: false,
    includeGuideEntry: false,
  });

  assert.equal(result.status, 0, result.stderr);
});

test("project check leaves incomplete dashboard display metadata visible but non-blocking", async () => {
  const invalidMetadata = [
    { displayName: "只有名稱" },
    { displayName: "名稱", displayPurpose: "用途", extraFrontmatter: "display_name_zh_tw: 重複名稱\n" },
    { displayName: "   ", displayPurpose: "用途" },
    { displayName: "English only", displayPurpose: "English purpose" },
  ];
  for (const overrides of invalidMetadata) {
    const result = await runFixture({
      tickets: [
        ["current.md", "TASK-017", "fixture", {
          displayName: "目前工作",
          displayPurpose: "目前工作用途。",
        }],
        ["parallel.md", "TASK-018", "fixture", overrides],
      ],
      plans: "**CURRENT TASK-017 IMPLEMENTATION:** shared planning index\n",
      gitBranch: "feature/task-018-fixture",
      includeGuideAllowedPath: false,
      includeGuideEntry: false,
    });

    assert.equal(result.status, 0, result.stderr);
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

test("project check accepts a non-terminal parallel TASK ticket during development", async () => {
  const result = await runFixture({
    tickets: [
      ["current.md", "TASK-017"],
      ["parallel.md", "TASK-018", "fixture", { status: "in_progress" }],
    ],
    plans: "**CURRENT TASK-017 IMPLEMENTATION:** shared planning index\n",
    gitBranch: "feature/task-018-fixture",
  });

  assert.equal(result.status, 0, result.stderr);
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
  assert.match(result.stderr, /cancelled or malformed task branches cannot resume/u);
});

test("project check leaves delivery policy and default-branch mutation protection to the finisher", async () => {
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

  assert.equal(unauthorized.status, 0, unauthorized.stderr);
  assert.equal(defaultBranch.status, 0, defaultBranch.stderr);
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

test("project check accepts a maintenance branch without a ticket or current-task marker", async () => {
  const result = await runFixture({
    tickets: [],
    plans: "# Plan\n\nStabilize the product without opening another task.\n",
    protocol: engineeringProtocol,
    agentsContent: agents,
    gitBranch: "maintenance/workflow-simplification",
    includeGuideAllowedPath: false,
    includeGuideEntry: false,
    deliveryPush: null,
    deliveryArchive: null,
    deliveryRemote: null,
    deliveryRepository: null,
  });

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /current_tasks=0/u);
});

test("project check accepts an in-progress task branch before delivery metadata exists", async () => {
  const result = await runFixture({
    tickets: [["one.md", "TASK-017", "fixture", { status: "in_progress" }]],
    plans: "# Plan\n\nNo global current-task lock.\n",
    protocol: engineeringProtocol,
    agentsContent: agents,
    gitBranch: "feature/task-017-fixture",
    includeGuideAllowedPath: false,
    includeGuideEntry: false,
    deliveryPush: null,
    deliveryArchive: null,
    deliveryRemote: null,
    deliveryRepository: null,
  });

  assert.equal(result.status, 0, result.stderr);
});

test("project check permits the default branch and leaves protected delivery to the finisher", async () => {
  const result = await runFixture({
    tickets: [],
    plans: "# Plan\n\nStable integration branch.\n",
    protocol: engineeringProtocol,
    agentsContent: agents,
    gitBranch: "main",
    defaultGitBranch: "main",
    includeGuideAllowedPath: false,
    includeGuideEntry: false,
    deliveryPush: null,
    deliveryArchive: null,
    deliveryRemote: null,
    deliveryRepository: null,
  });

  assert.equal(result.status, 0, result.stderr);
});
