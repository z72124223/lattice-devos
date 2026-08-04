import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

const checkScript = path.resolve("scripts/check-project.mjs");
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
  constitutionPath = path.join(
    "docs",
    "modules",
    "fixture",
    "MODULE_CONSTITUTION.md",
  ),
}) {
  const root = await mkdtemp(path.join(tmpdir(), "lattice-project-check-"));
  try {
    const constitutionFile = path.join(root, constitutionPath);
    await mkdir(path.dirname(constitutionFile), { recursive: true });
    await mkdir(path.join(root, "docs", "tickets"), { recursive: true });
    await writeFile(constitutionFile, constitution, "utf8");
    await writeFile(path.join(root, "PLANS.md"), plans, "utf8");
    for (const [name, ticketId, moduleId = "fixture"] of tickets) {
      await writeFile(
        path.join(root, "docs", "tickets", name),
        `---\nticket_id: ${ticketId}\nmodule_id: ${moduleId}\n---\n`,
        "utf8",
      );
    }
    return spawnSync(process.execPath, [checkScript], {
      cwd: root,
      encoding: "utf8",
    });
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
