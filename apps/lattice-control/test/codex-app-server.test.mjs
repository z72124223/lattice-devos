import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import process from "node:process";
import test from "node:test";
import { CodexAppServer } from "../src/codex-app-server.mjs";

class FakeProcess extends EventEmitter {
  stdin = new PassThrough();
  stdout = new PassThrough();
  stderr = new PassThrough();
  exitCode = null;

  kill() {
    this.exitCode = 0;
    this.emit("exit", 0, null);
  }
}

test("uses the official JSONL handshake and model listing without starting a turn", async () => {
  const child = new FakeProcess();
  const launches = [];
  let buffered = "";
  child.stdin.on("data", (chunk) => {
    buffered += chunk.toString("utf8");
    for (;;) {
      const newline = buffered.indexOf("\n");
      if (newline < 0) break;
      const message = JSON.parse(buffered.slice(0, newline));
      buffered = buffered.slice(newline + 1);
      if (message.method === "initialize") {
        child.stdout.write(`${JSON.stringify({ id: message.id, result: { platformFamily: "windows" } })}\n`);
      } else if (message.method === "model/list") {
        child.stdout.write(`${JSON.stringify({
          id: message.id,
          result: { data: [{ id: "gpt-5.6-terra" }], nextCursor: null },
        })}\n`);
      }
    }
  });

  const codex = new CodexAppServer({
    spawnProcess(command, args, options) {
      launches.push({ command, args, options });
      return child;
    },
  });
  const models = await codex.listModels();
  assert.equal(models.data[0].id, "gpt-5.6-terra");
  assert.equal(launches.length, 1);
  if (process.platform === "win32") {
    assert.equal(launches[0].command, process.execPath);
    assert.match(launches[0].args[0], /@openai[\\/]codex[\\/]bin[\\/]codex\.js$/iu);
    assert.deepEqual(launches[0].args.slice(1), ["app-server", "--stdio"]);
  } else {
    assert.equal(launches[0].command, "codex");
  }
  await codex.close();
});
