import assert from "node:assert/strict";
import { mkdtemp, mkdir, realpath, writeFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { resolveWindowsCodexRuntime } from "../src/codex-runtime-resolution.mjs";

async function fixture(t) {
  const temporaryRoot = await realpath(os.tmpdir());
  const root = await mkdtemp(path.join(temporaryRoot, "lattice-codex-resolution-"));
  const createdRoot = await realpath(root);
  t.after(async () => {
    const resolved = await realpath(root), relative = path.relative(temporaryRoot, resolved);
    assert.equal(resolved, createdRoot);
    assert.ok(relative && !path.isAbsolute(relative) && relative !== ".." && !relative.startsWith(`..${path.sep}`));
    await rm(resolved, { recursive: true, force: true });
  });
  const env = { LOCALAPPDATA: path.join(root, "local"), APPDATA: path.join(root, "roaming") };
  async function file(target) {
    await mkdir(path.dirname(target), { recursive: true });
    await writeFile(target, "fixture");
    return target;
  }
  return { env, desktop: (id) => file(path.join(env.LOCALAPPDATA, "OpenAI", "Codex", "bin", id, "codex.exe")),
    npm: () => file(path.join(env.APPDATA, "npm", "node_modules", "@openai", "codex", "bin", "codex.js")) };
}

test("Desktop 0.155.0-alpha.16 wins over 0.144.6 instead of falling back to installed npm", async (t) => {
  const f = await fixture(t);
  const older = await f.desktop("aaaaaaaaaaaaaaaa");
  const newer = await f.desktop("bbbbbbbbbbbbbbbb");
  await f.desktop("cccccccccccccccc");
  await f.desktop("unrelated-package");
  await f.npm();
  const probed = [];
  const result = await resolveWindowsCodexRuntime({ env: f.env, probeVersion: async (command) => {
    probed.push(command);
    if (command === newer) return "codex-cli 0.155.0-alpha.16";
    if (command === older) return "codex-cli 0.144.6";
    throw new Error("incomplete installation");
  } });
  assert.deepEqual(result, { command: newer, args: ["app-server", "--stdio"] });
  assert.equal(probed.length, 3);
});

test("stable, prerelease, and build metadata are valid SemVer desktop versions", async (t) => {
  const f = await fixture(t), command = await f.desktop("aaaaaaaaaaaaaaaa");
  await f.npm();
  for (const version of ["0.0.0", "1.2.3-alpha.16", "1.2.3+build.007", "1.2.3-0.A-z.9+build.007-x", "1.2.3-01a.-.A-1+000"]) {
    await t.test(version, async () => {
      assert.deepEqual(await resolveWindowsCodexRuntime({ env: f.env, probeVersion: async () => `codex-cli ${version}` }),
        { command, args: ["app-server", "--stdio"] });
    });
  }
});

test("malformed version output cannot become a desktop candidate", async (t) => {
  const f = await fixture(t);
  await f.desktop("aaaaaaaaaaaaaaaa");
  const script = await f.npm();
  const invalid = ["", "codex 1.2.3", "codex-cli v1.2.3", "codex-cli 1.2.3 extra", "codex-cli 1.2.3\nother output",
    ...["01.2.3", "1.02.3", "1.2.03", "1.2.3.4", "1.2.3-", "1.2.3+", "1.2.3-alpha..1", "1.2.3+a..b",
      "1.2.3-01", "1.2.3-alpha.01", "1.2.3-alpha_beta", "1.2.3+build_meta", "1.2.3-α"].map(version => `codex-cli ${version}`)];
  for (const output of invalid) await t.test(JSON.stringify(output), async () => {
    assert.deepEqual(await resolveWindowsCodexRuntime({ env: f.env, nodeExecutable: "node-fixture", probeVersion: async () => output }),
      { command: "node-fixture", args: [script, "app-server", "--stdio"] });
  });
});

test("desktop choice uses SemVer precedence rather than lexical or floating point ordering", async (t) => {
  const f = await fixture(t), lower = await f.desktop("aaaaaaaaaaaaaaaa"), higher = await f.desktop("bbbbbbbbbbbbbbbb");
  const pairs = [
    ["major before minor", "1.99.99", "2.0.0-alpha"], ["minor before patch", "1.2.99", "1.3.0-alpha"],
    ["patch before prerelease", "1.2.3", "1.2.4-alpha"], ["stable above prerelease", "1.2.3-zzzz", "1.2.3"],
    ["numeric identifier value", "1.2.3-alpha.9", "1.2.3-alpha.10"], ["numeric below nonnumeric", "1.2.3-99", "1.2.3-0A"],
    ["ASCII text ordering", "1.2.3-Z", "1.2.3-a"], ["longer shared prefix", "1.2.3-alpha", "1.2.3-alpha.0"],
    ["large numeric identifier", "1.2.3-9007199254740992", "1.2.3-9007199254740993"],
    ["large core integer", "9007199254740992.0.0", "9007199254740993.0.0"],
  ];
  for (const [name, before, after] of pairs) await t.test(name, async () => {
    assert.deepEqual(await resolveWindowsCodexRuntime({ env: f.env,
      probeVersion: async command => `codex-cli ${command === lower ? before : after}` }),
    { command: higher, args: ["app-server", "--stdio"] });
  });
  for (const [before, after] of [["1.2.3+z", "1.2.3+a"], ["1.2.3-alpha+z", "1.2.3-alpha+a"], ["1.2.3", "1.2.3+build.1"]]) {
    await t.test(`build metadata tie: ${before} / ${after}`, async () => {
      assert.deepEqual(await resolveWindowsCodexRuntime({ env: f.env,
        probeVersion: async command => `codex-cli ${command === lower ? before : after}` }),
      { command: [lower, higher].sort((a, b) => a.localeCompare(b))[0], args: ["app-server", "--stdio"] });
    });
  }
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
