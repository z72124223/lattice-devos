import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { resolveWindowsCodexRuntime } from "../src/codex-runtime-resolution.mjs";

async function fixture(t) {
  const root = await mkdtemp(path.join(os.tmpdir(), "lattice-codex-resolution-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const env = { LOCALAPPDATA: path.join(root, "local"), APPDATA: path.join(root, "roaming") };
  async function file(target) {
    await mkdir(path.dirname(target), { recursive: true });
    await writeFile(target, "fixture");
    return target;
  }
  return { env, desktop: (id) => file(path.join(env.LOCALAPPDATA, "OpenAI", "Codex", "bin", id, "codex.exe")),
    npm: () => file(path.join(env.APPDATA, "npm", "node_modules", "@openai", "codex", "bin", "codex.js")) };
}

test("default runtime uses the newest valid desktop CLI even when npm is installed", async (t) => {
  const f = await fixture(t);
  const older = await f.desktop("aaaaaaaaaaaaaaaa");
  const newer = await f.desktop("bbbbbbbbbbbbbbbb");
  await f.desktop("cccccccccccccccc");
  await f.desktop("unrelated-package");
  await f.npm();
  const probed = [];
  const result = await resolveWindowsCodexRuntime({ env: f.env, probeVersion: async (command) => {
    probed.push(command);
    if (command === newer) return "codex-cli 0.153.3";
    if (command === older) return "codex-cli 0.99.0";
    throw new Error("incomplete installation");
  } });
  assert.deepEqual(result, { command: newer, args: ["app-server", "--stdio"] });
  assert.equal(probed.length, 3);
});

test("npm remains a fallback when no valid desktop CLI exists", async (t) => {
  const f = await fixture(t);
  await f.desktop("aaaaaaaaaaaaaaaa");
  const script = await f.npm();
  const result = await resolveWindowsCodexRuntime({ env: f.env, nodeExecutable: "node-fixture",
    probeVersion: async () => "unexpected version output" });
  assert.deepEqual(result, { command: "node-fixture", args: [script, "app-server", "--stdio"] });
});

test("missing or relative installation roots cannot resolve against the working directory", async () => {
  await assert.rejects(resolveWindowsCodexRuntime({ env: { LOCALAPPDATA: ".", APPDATA: "" } }), /runtime was not found/u);
});
