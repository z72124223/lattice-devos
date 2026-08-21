import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import test from "node:test";

test("project check enforces the four-part Runtime contract", () => {
  const result = spawnSync(process.execPath, ["scripts/check-project.mjs"], {
    cwd: process.cwd(),
    encoding: "utf8",
  });

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /runtime_contract=ok/u);
});
