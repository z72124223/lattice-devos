import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

const checkScript = path.resolve("scripts/check-project.mjs");
const engineeringProtocol = await readFile(
  path.resolve("docs/contracts/ENGINEERING_PROTOCOL_V1.md"),
  "utf8",
);
const agents = await readFile(path.resolve("AGENTS.md"), "utf8");

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
  plans = "# Plan\n\nNo global current-task lock.\n",
  protocol = engineeringProtocol,
  tickets = [],
  gitBranch = "product/fixture",
  nestedGitWorktree = false,
} = {}) {
  const fixtureContainer = await mkdtemp(path.join(tmpdir(), "lattice-project-check-"));
  const root = nestedGitWorktree
    ? path.join(fixtureContainer, "child-worktree")
    : fixtureContainer;

  try {
    if (nestedGitWorktree) {
      await mkdir(root, { recursive: true });
      await writeFile(path.join(fixtureContainer, ".git"), "gitdir: fixture-parent\n", "utf8");
    }

    const gitInit = spawnSync("git", ["init", "-b", gitBranch], {
      cwd: root,
      encoding: "utf8",
    });
    assert.equal(gitInit.status, 0, gitInit.stderr);

    const moduleDirectory = path.join(root, "docs", "modules", "fixture");
    const contractDirectory = path.join(root, "docs", "contracts");
    const ticketDirectory = path.join(root, "docs", "tickets");
    await mkdir(moduleDirectory, { recursive: true });
    await mkdir(contractDirectory, { recursive: true });
    await mkdir(ticketDirectory, { recursive: true });

    await writeFile(path.join(root, "AGENTS.md"), agents, "utf8");
    await writeFile(path.join(root, "PLANS.md"), plans, "utf8");
    await writeFile(
      path.join(moduleDirectory, "MODULE_CONSTITUTION.md"),
      constitution,
      "utf8",
    );
    if (protocol !== null) {
      await writeFile(
        path.join(contractDirectory, "ENGINEERING_PROTOCOL_V1.md"),
        protocol,
        "utf8",
      );
    }

    for (const [name, ticketId, status = "in_progress"] of tickets) {
      await writeFile(
        path.join(ticketDirectory, name),
        `---\nticket_id: ${ticketId}\nmodule_id: fixture\nstatus: ${status}\n---\n`,
        "utf8",
      );
    }

    return spawnSync(process.execPath, [checkScript], {
      cwd: root,
      encoding: "utf8",
    });
  } finally {
    await rm(fixtureContainer, { recursive: true, force: true });
  }
}

test("maintenance and product branches do not need a TASK", async () => {
  const result = await runFixture();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /current_tasks=0/u);
});

test("an in-progress task may be verified without delivery metadata", async () => {
  const result = await runFixture({
    gitBranch: "feature/task-017-fixture",
    tickets: [["task.md", "TASK-017", "in_progress"]],
  });
  assert.equal(result.status, 0, result.stderr);
});

test("duplicate ticket identities still fail closed", async () => {
  const result = await runFixture({
    tickets: [
      ["one.md", "TASK-017"],
      ["two.md", "TASK-017"],
    ],
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /duplicate ticket_id/u);
});

test("the shared plan may have zero or one current focus, never two", async () => {
  const result = await runFixture({
    plans: "CURRENT TASK-017 work\nCURRENT TASK-018 work\n",
    tickets: [
      ["one.md", "TASK-017"],
      ["two.md", "TASK-018"],
    ],
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /expected at most one CURRENT TASK marker/u);
});

test("a current focus must still resolve to a unique ticket", async () => {
  const result = await runFixture({ plans: "CURRENT TASK-017 work\n" });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /has no matching unique ticket/u);
});

test("the complexity circuit breaker is mandatory", async () => {
  const result = await runFixture({
    protocol: engineeringProtocol.replace(
      /Do not create\s+another task only to repair governance/u,
      "Create a new task whenever governance fails",
    ),
  });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /missing required contract content/u);
});

test("a worktree nested inside another worktree still fails closed", async () => {
  const result = await runFixture({ nestedGitWorktree: true });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /must not be nested inside another Git worktree/u);
});
