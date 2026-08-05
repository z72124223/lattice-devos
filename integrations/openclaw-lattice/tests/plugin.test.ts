import assert from "node:assert/strict";
import { test } from "node:test";

import type { OpenClawPluginApi } from "openclaw/plugin-sdk/plugin-entry";

import {
  createLatticeCommandDefinition,
  registerLatticeCommand,
} from "../src/index.js";

type CommandDefinition = Parameters<OpenClawPluginApi["registerCommand"]>[0];

test("registers only the authenticated lattice command name without slash", () => {
  const observed: CommandDefinition[] = [];
  registerLatticeCommand({
    registerCommand(command) {
      observed.push(command);
    },
  });
  assert.equal(observed.length, 1);
  assert.equal(observed[0]?.name, "lattice");
  assert.equal(observed[0]?.requireAuth, true);
  assert.equal(observed[0]?.acceptsArgs, true);
});

test("command definition is closed and does not register a generic tool", () => {
  const command = createLatticeCommandDefinition();
  assert.deepEqual(Object.keys(command).sort(), [
    "acceptsArgs",
    "description",
    "handler",
    "name",
    "requireAuth",
  ]);
});
