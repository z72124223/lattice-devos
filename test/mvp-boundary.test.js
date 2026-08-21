import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("the default developer commands do not start the retired full chain", async () => {
  const packageJson = JSON.parse(await readFile("package.json", "utf8"));

  assert.equal(packageJson.scripts["operator:start"], undefined);
  assert.equal(packageJson.scripts["operator:smoke"], undefined);
  assert.equal(packageJson.scripts["test:operator-entrypoint"], undefined);
  assert.match(packageJson.scripts["legacy:full-chain:start"], /start-lattice-full-chain/u);
  assert.match(packageJson.scripts["legacy:full-chain:smoke"], /start-lattice-full-chain/u);
  assert.match(packageJson.scripts["legacy:full-chain:test"], /test-lattice-full-chain-entrypoint/u);
});

test("the default Rust workspace gate excludes retired and deferred runtime modules", async () => {
  const cargo = await readFile("Cargo.toml", "utf8");
  const defaultMembers = cargo.match(/default-members\s*=\s*\[(?<members>[\s\S]*?)\]/u);

  assert.ok(defaultMembers?.groups?.members, "Cargo.toml must define default-members");
  const members = defaultMembers.groups.members;
  assert.doesNotMatch(members, /lattice-(?:runtime|openclaw|hermes|orchestrator)/u);
  assert.match(members, /lattice-core/u);
  assert.match(members, /lattice-task-domain/u);
  assert.match(members, /lattice-project-registry/u);
  assert.match(members, /lattice-task-ledger/u);
  assert.match(members, /lattice-codebase-memory/u);
});
