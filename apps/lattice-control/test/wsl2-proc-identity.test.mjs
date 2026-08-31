import assert from "node:assert/strict";
import test from "node:test";

import { parseProcStat } from "../src/wsl2-proc-identity.mjs";

test("proc stat parsing preserves pgrp and starttime when comm contains spaces and parentheses", () => {
  const tail = ["S", "11", "42", "11", "0", "0", "0", "0", "0", "0", "0", "0", "0", "0", "0", "0", "0", "0", "0", "987654", "0"];
  const parsed = parseProcStat(`123 (codex worker ) name) ${tail.join(" ")}\n`);
  assert.deepEqual(parsed, { pid: 123, processGroupId: 42, startTime: "987654" });
});

test("proc stat parsing rejects truncated input", () => {
  assert.throws(() => parseProcStat("123 (bad) S 1"), /WSL2_PROC_STAT_REJECTED/u);
});
