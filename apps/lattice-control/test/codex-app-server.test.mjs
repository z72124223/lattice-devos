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
  killCount = 0;

  kill() {
    this.killCount += 1;
    this.exitCode = 0;
    this.emit("exit", 0, null);
  }
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function observeClientMessages(child, onMessage) {
  let buffered = "";
  child.stdin.on("data", (chunk) => {
    buffered += chunk.toString("utf8");
    for (;;) {
      const newline = buffered.indexOf("\n");
      if (newline < 0) break;
      const line = buffered.slice(0, newline);
      buffered = buffered.slice(newline + 1);
      if (line) onMessage(JSON.parse(line));
    }
  });
}

function sendServerMessage(child, message) {
  child.stdout.write(`${JSON.stringify(message)}\n`);
}

function nextMacrotask() {
  return new Promise((resolve) => setImmediate(resolve));
}

function createInitializedConnector(onMessage = () => {}, options = {}) {
  const child = new FakeProcess();
  const messages = [];
  observeClientMessages(child, (message) => {
    messages.push(message);
    if (message.method === "initialize") {
      sendServerMessage(child, {
        id: message.id,
        result: { platformFamily: "windows" },
      });
      return;
    }
    onMessage(message, child);
  });
  const codex = new CodexAppServer({
    codexBin: "codex-test",
    spawnProcess: () => child,
    ...options,
  });
  return { child, codex, messages };
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

test("concurrent starts share one connection and cannot outrun initialized", async () => {
  const child = new FakeProcess();
  const initializeObserved = deferred();
  const messages = [];
  let initializeRequest;
  let initializeReplied = false;
  let threadNumber = 0;

  observeClientMessages(child, (message) => {
    messages.push(message);
    if (message.method === "initialize") {
      initializeRequest = message;
      initializeObserved.resolve();
    } else if (message.method === "thread/start") {
      threadNumber += 1;
      sendServerMessage(child, {
        id: message.id,
        result: { thread: { id: `thread-${threadNumber}` } },
      });
    }
  });

  const launches = [];
  const codex = new CodexAppServer({
    codexBin: "codex-test",
    spawnProcess(command, args, options) {
      launches.push({ command, args, options });
      return child;
    },
  });
  const starts = [
    codex.startThread({ cwd: "C:\\workspace-a" }),
    codex.startThread({ cwd: "C:\\workspace-b" }),
  ];

  try {
    await initializeObserved.promise;
    await nextMacrotask();

    assert.equal(launches.length, 1);
    assert.equal(messages.filter(({ method }) => method === "initialize").length, 1);
    assert.equal(
      messages.filter(({ method }) => method === "thread/start").length,
      0,
      "thread/start must wait until initialize has completed and initialized was sent",
    );

    initializeReplied = true;
    sendServerMessage(child, {
      id: initializeRequest.id,
      result: { platformFamily: "windows" },
    });
    const threads = await Promise.all(starts);
    assert.deepEqual(threads.map(({ id }) => id), ["thread-1", "thread-2"]);

    const initializedIndex = messages.findIndex(({ method }) => method === "initialized");
    const threadStartIndexes = messages
      .map(({ method }, index) => method === "thread/start" ? index : -1)
      .filter((index) => index >= 0);
    assert.ok(initializedIndex >= 0);
    assert.ok(threadStartIndexes.every((index) => index > initializedIndex));
  } finally {
    if (initializeRequest && !initializeReplied) {
      sendServerMessage(child, {
        id: initializeRequest.id,
        result: { platformFamily: "windows" },
      });
    }
    await Promise.allSettled(starts);
    await codex.close();
  }
});

test("a timed out RPC is rejected and removed from the public pending count", async () => {
  const child = new FakeProcess();
  let modelRequest;
  observeClientMessages(child, (message) => {
    if (message.method === "initialize") {
      sendServerMessage(child, {
        id: message.id,
        result: { platformFamily: "windows" },
      });
    } else if (message.method === "model/list") {
      modelRequest = message;
    }
  });

  const codex = new CodexAppServer({
    codexBin: "codex-test",
    requestTimeoutMs: 20,
    spawnProcess: () => child,
  });
  const models = codex.listModels();
  let guardTimer;
  const guard = new Promise((resolve, reject) => {
    guardTimer = setTimeout(
      () => reject(new Error("test guard expired before the connector rejected the RPC")),
      250,
    );
  });

  try {
    await assert.rejects(
      Promise.race([models, guard]),
      /(?:model\/list.*timed out|timed out.*model\/list)/iu,
    );
    clearTimeout(guardTimer);
    assert.equal(codex.pendingRequestCount, 0);

    sendServerMessage(child, {
      id: modelRequest.id,
      result: { data: [], nextCursor: null },
    });
    await nextMacrotask();
    assert.equal(codex.pendingRequestCount, 0, "a late reply must not restore timed-out state");
  } finally {
    clearTimeout(guardTimer);
    await codex.close();
    await Promise.allSettled([models]);
  }
});

test("accepted starts correlate exact started notifications before or after the RPC reply", async () => {
  const child = new FakeProcess();
  let turnRequest;
  observeClientMessages(child, (message) => {
    if (message.method === "initialize") {
      sendServerMessage(child, {
        id: message.id,
        result: { platformFamily: "windows" },
      });
    } else if (message.method === "thread/start") {
      sendServerMessage(child, {
        method: "thread/started",
        params: { thread: { id: "thread-ready" } },
      });
      sendServerMessage(child, {
        id: message.id,
        result: { thread: { id: "thread-ready" } },
      });
    } else if (message.method === "turn/start") {
      turnRequest = message;
      sendServerMessage(child, {
        id: message.id,
        result: { turn: { id: "turn-ready", status: "inProgress" } },
      });
    }
  });

  const codex = new CodexAppServer({
    codexBin: "codex-test",
    spawnProcess: () => child,
  });

  try {
    const thread = await codex.startThread({ cwd: "C:\\workspace" });
    await codex.waitForThreadStarted(thread.id, { timeoutMs: 200 });

    const turn = await codex.startTurn(thread.id, "Run the focused check.");
    assert.equal(turnRequest.params.threadId, thread.id);
    let turnStartedSettled = false;
    const turnStarted = codex.waitForTurnStarted(thread.id, turn.id, { timeoutMs: 200 });
    turnStarted.then(
      () => { turnStartedSettled = true; },
      () => { turnStartedSettled = true; },
    );

    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "other-thread", turn: { id: turn.id, status: "inProgress" } },
    });
    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: thread.id, turn: { id: "other-turn", status: "inProgress" } },
    });
    await nextMacrotask();
    assert.equal(turnStartedSettled, false, "unrelated started notifications cannot release the waiter");

    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: thread.id, turn: { id: turn.id, status: "inProgress" } },
    });
    await turnStarted;
  } finally {
    await codex.close();
  }
});

test("interrupt is fail-closed until the exact turn reports inProgress", async () => {
  const { child, codex, messages } = createInitializedConnector();
  try {
    await codex.connect();
    assert.equal(codex.isTurnActive("thread-active", "turn-active"), false);

    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "thread-active", turn: { id: "turn-active", status: "completed" } },
    });
    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "other-thread", turn: { id: "turn-active", status: "inProgress" } },
    });
    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "thread-active", turn: { id: "other-turn", status: "inProgress" } },
    });
    await nextMacrotask();
    assert.equal(codex.isTurnActive("thread-active", "turn-active"), false);
    await assert.rejects(
      codex.interruptTurn("thread-active", "turn-active", { timeoutMs: 100 }),
      /turn.*not active|no active turn/iu,
    );
    assert.equal(messages.some(({ method }) => method === "turn/interrupt"), false);

    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "thread-active", turn: { id: "turn-active", status: "inProgress" } },
    });
    await nextMacrotask();
    assert.equal(codex.isTurnActive("thread-active", "turn-active"), true);
  } finally {
    await codex.close();
  }
});

test("interrupt waits past RPC acceptance for the exact interrupted or failed terminal", async () => {
  const interruptRequests = [];
  const { child, codex } = createInitializedConnector((message, server) => {
    if (message.method !== "turn/interrupt") return;
    interruptRequests.push(message);
    sendServerMessage(server, { id: message.id, result: {} });
  });

  try {
    await codex.connect();
    for (const [index, terminalStatus] of ["interrupted", "failed"].entries()) {
      const threadId = `thread-${index}`;
      const turnId = `turn-${index}`;
      sendServerMessage(child, {
        method: "turn/started",
        params: { threadId, turn: { id: turnId, status: "inProgress" } },
      });
      await nextMacrotask();
      assert.equal(codex.isTurnActive(threadId, turnId), true);

      let settled = false;
      const interrupted = codex.interruptTurn(threadId, turnId, { timeoutMs: 200 });
      interrupted.then(
        () => { settled = true; },
        () => { settled = true; },
      );
      await nextMacrotask();
      assert.equal(settled, false, "the turn/interrupt RPC result is not a terminal event");

      if (index === 0) {
        sendServerMessage(child, {
          method: "turn/completed",
          params: { threadId: "other-thread", turn: { id: turnId, status: terminalStatus } },
        });
        sendServerMessage(child, {
          method: "turn/completed",
          params: { threadId, turn: { id: "other-turn", status: terminalStatus } },
        });
        sendServerMessage(child, {
          method: "turn/completed",
          params: { threadId, turn: { id: turnId, status: "completed" } },
        });
        await nextMacrotask();
        assert.equal(settled, false, "wrong IDs or a non-interrupt terminal cannot release the waiter");
      }

      sendServerMessage(child, {
        method: "turn/completed",
        params: { threadId, turn: { id: turnId, status: terminalStatus } },
      });
      await interrupted;
      assert.equal(codex.isTurnActive(threadId, turnId), false);
      assert.equal(codex.pendingNotificationCount, 0);
      assert.equal(child.killCount, 0, "a correlated terminal must not kill the App Server");
    }

    assert.deepEqual(
      interruptRequests.map(({ params }) => params),
      [
        { threadId: "thread-0", turnId: "turn-0" },
        { threadId: "thread-1", turnId: "turn-1" },
      ],
    );
  } finally {
    await codex.close();
  }
});

test("interrupt timeout clears its waiter and only then kills the owned process", async () => {
  const interruptObserved = deferred();
  const { child, codex } = createInitializedConnector((message, server) => {
    if (message.method !== "turn/interrupt") return;
    sendServerMessage(server, { id: message.id, result: {} });
    interruptObserved.resolve();
  });
  let interrupted;
  let guardTimer;

  try {
    await codex.connect();
    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "thread-timeout", turn: { id: "turn-timeout", status: "inProgress" } },
    });
    await nextMacrotask();

    interrupted = codex.interruptTurn("thread-timeout", "turn-timeout", { timeoutMs: 20 });
    await interruptObserved.promise;
    await nextMacrotask();
    assert.equal(child.killCount, 0, "RPC acceptance alone must not kill the App Server");

    const guard = new Promise((resolve, reject) => {
      guardTimer = setTimeout(
        () => reject(new Error("test guard expired before interrupt timeout cleanup")),
        250,
      );
    });
    await assert.rejects(
      Promise.race([interrupted, guard]),
      /(?:interrupt|turn\/completed).*timed out/iu,
    );
    clearTimeout(guardTimer);
    assert.equal(codex.pendingNotificationCount, 0);
    assert.equal(codex.isTurnActive("thread-timeout", "turn-timeout"), false);
    assert.equal(child.killCount, 1);
  } finally {
    clearTimeout(guardTimer);
    await codex.close();
    if (interrupted) await Promise.allSettled([interrupted]);
  }
});

test("an exact interrupt terminal wins over a lost RPC acknowledgement without killing peers", async () => {
  const interruptObserved = deferred();
  const { child, codex } = createInitializedConnector((message) => {
    if (message.method === "turn/interrupt") interruptObserved.resolve();
  });
  try {
    await codex.connect();
    sendServerMessage(child, {
      method: "turn/started",
      params: { threadId: "thread-terminal-first", turn: { id: "turn-terminal-first", status: "inProgress" } },
    });
    await nextMacrotask();

    const interrupting = codex.interruptTurn(
      "thread-terminal-first",
      "turn-terminal-first",
      { timeoutMs: 30 },
    );
    await interruptObserved.promise;
    sendServerMessage(child, {
      method: "turn/completed",
      params: {
        threadId: "thread-terminal-first",
        turn: { id: "turn-terminal-first", status: "interrupted" },
      },
    });

    const terminal = await interrupting;
    assert.equal(terminal.status, "interrupted");
    assert.equal(codex.pendingRequestCount, 0);
    assert.equal(codex.pendingNotificationCount, 0);
    assert.equal(child.killCount, 0);
  } finally {
    await codex.close();
  }
});

test("readThread returns only an exact thread with non-empty turns", async () => {
  const { codex } = createInitializedConnector((message, server) => {
    if (message.method !== "thread/read") return;
    const { threadId, includeTurns } = message.params;
    assert.equal(includeTurns, true);
    const responses = {
      "thread-valid": { id: "thread-valid", turns: [{ id: "turn-done", status: "completed" }] },
      "thread-mismatch": { id: "other-thread", turns: [{ id: "turn-done", status: "completed" }] },
      "thread-missing-turns": { id: "thread-missing-turns" },
      "thread-empty": { id: "thread-empty", turns: [] },
    };
    sendServerMessage(server, { id: message.id, result: { thread: responses[threadId] } });
  });

  try {
    assert.deepEqual(
      await codex.readThread("thread-valid", { includeTurns: true }),
      { id: "thread-valid", turns: [{ id: "turn-done", status: "completed" }] },
    );
    for (const threadId of ["thread-mismatch", "thread-missing-turns", "thread-empty"]) {
      await assert.rejects(
        codex.readThread(threadId, { includeTurns: true }),
        /not recoverable|reconciliation|empty rollout/iu,
      );
    }
  } finally {
    await codex.close();
  }
});

test("resume rejects empty or non-terminal loaded rollouts before reconciliation read", async () => {
  const { codex, messages } = createInitializedConnector((message, server) => {
    if (message.method === "thread/resume") {
      const turns = message.params.threadId === "thread-empty"
        ? []
        : [{ id: "turn-active", status: "inProgress" }];
      sendServerMessage(server, {
        id: message.id,
        result: { thread: { id: message.params.threadId, turns } },
      });
    } else if (message.method === "thread/read") {
      sendServerMessage(server, {
        id: message.id,
        result: { thread: { id: message.params.threadId, turns: [] } },
      });
    }
  });

  try {
    for (const threadId of ["thread-empty", "thread-active"]) {
      await assert.rejects(
        codex.resumeThread(threadId),
        /not recoverable|reconciliation|empty rollout|terminal turn/iu,
      );
    }
    assert.equal(messages.filter(({ method }) => method === "thread/resume").length, 2);
    assert.equal(messages.some(({ method }) => method === "thread/read"), false);
  } finally {
    await codex.close();
  }
});

test("fresh-process resume loads terminal history before an exact reconciliation read", async () => {
  const persistedThread = {
    id: "thread-resumable",
    turns: [{ id: "turn-completed", status: "completed" }],
  };
  const { codex, messages } = createInitializedConnector((message, server) => {
    if (message.method === "thread/resume") {
      sendServerMessage(server, { id: message.id, result: { thread: persistedThread } });
    } else if (message.method === "thread/read") {
      sendServerMessage(server, { id: message.id, result: { thread: persistedThread } });
    }
  });

  try {
    assert.deepEqual(await codex.resumeThread(persistedThread.id), persistedThread);
    assert.deepEqual(
      messages.map(({ method }) => method),
      ["initialize", "initialized", "thread/resume", "thread/read"],
    );
    assert.deepEqual(messages[2].params, {
      threadId: persistedThread.id,
    });
    assert.deepEqual(messages[3].params, {
      threadId: persistedThread.id,
      includeTurns: true,
    });
  } finally {
    await codex.close();
  }
});

test("an unhandled server request receives an immediate method-not-found response", async () => {
  const { child, codex, messages } = createInitializedConnector();
  try {
    await codex.connect();
    sendServerMessage(child, {
      id: 701,
      method: "unknown/server/request",
      params: { value: "untrusted" },
    });
    await nextMacrotask();

    const response = messages.find((message) => message.id === 701 && !message.method);
    assert.equal(response?.error?.code, -32601);
    assert.match(response?.error?.message ?? "", /method not found|unsupported|unknown/iu);
    assert.equal(codex.pendingServerRequestCount, 0);
  } finally {
    await codex.close();
  }
});

test("a handler can explicitly defer and then resolve a server request", async () => {
  const { child, codex, messages } = createInitializedConnector();
  codex.on("serverRequest", (request) => {
    codex.deferServerRequest(request.id, { timeoutMs: 200 });
  });

  try {
    await codex.connect();
    sendServerMessage(child, {
      id: 702,
      method: "item/commandExecution/requestApproval",
      params: { reason: "Run focused tests" },
    });
    await nextMacrotask();
    assert.equal(codex.pendingServerRequestCount, 1);
    assert.equal(messages.some((message) => message.id === 702 && !message.method), false);

    codex.respond(702, { decision: "accept" });
    await nextMacrotask();
    assert.deepEqual(
      messages.find((message) => message.id === 702 && !message.method),
      { id: 702, result: { decision: "accept" } },
    );
    assert.equal(codex.pendingServerRequestCount, 0);
  } finally {
    await codex.close();
  }
});

test("a deferred server request times out with an explicit error and no leaked state", async () => {
  const timedOut = deferred();
  const { child, codex } = createInitializedConnector((message) => {
    if (message.id === 703 && message.error) timedOut.resolve(message);
  });
  codex.on("serverRequest", (request) => {
    codex.deferServerRequest(request.id, { timeoutMs: 20 });
  });
  let guardTimer;

  try {
    await codex.connect();
    sendServerMessage(child, {
      id: 703,
      method: "item/fileChange/requestApproval",
      params: {},
    });
    const guard = new Promise((resolve, reject) => {
      guardTimer = setTimeout(
        () => reject(new Error("test guard expired before deferred request timeout")),
        250,
      );
    });
    const response = await Promise.race([timedOut.promise, guard]);
    clearTimeout(guardTimer);

    assert.equal(response.id, 703);
    assert.ok(Number.isInteger(response.error.code));
    assert.match(response.error.message, /timed out|timeout/iu);
    assert.equal(codex.pendingServerRequestCount, 0);
    assert.equal(codex.listenerCount("serverRequest"), 1);
  } finally {
    clearTimeout(guardTimer);
    await codex.close();
  }
});

test("closing the connector clears deferred server requests", async () => {
  const { child, codex } = createInitializedConnector();
  codex.on("serverRequest", (request) => {
    codex.deferServerRequest(request.id, { timeoutMs: 5_000 });
  });

  await codex.connect();
  sendServerMessage(child, {
    id: 704,
    method: "item/tool/requestUserInput",
    params: {},
  });
  await nextMacrotask();
  assert.equal(codex.pendingServerRequestCount, 1);

  await codex.close();
  assert.equal(codex.pendingServerRequestCount, 0);
});

test("owned close emits one disconnect so shared work can fail closed", async () => {
  const { codex } = createInitializedConnector();
  const disconnects = [];
  codex.on("disconnect", (details) => disconnects.push(details));

  await codex.connect();
  await codex.close();
  await nextMacrotask();

  assert.deepEqual(disconnects, [{ code: null, signal: "client-close" }]);
});

test("late output from an exited generation cannot settle or activate a reconnect", async () => {
  const children = [new FakeProcess(), new FakeProcess()];
  for (const child of children) {
    observeClientMessages(child, (message) => {
      if (message.method === "initialize") {
        sendServerMessage(child, { id: message.id, result: { platformFamily: "windows" } });
      } else if (message.method === "model/list") {
        sendServerMessage(child, { id: message.id, result: { data: [], nextCursor: null } });
      }
    });
  }
  let launch = 0;
  const codex = new CodexAppServer({
    codexBin: "codex-test",
    spawnProcess: () => children[launch++],
  });

  try {
    await codex.connect();
    children[0].exitCode = 1;
    children[0].emit("exit", 1, null);
    await nextMacrotask();
    await codex.connect();

    sendServerMessage(children[0], {
      method: "turn/started",
      params: {
        threadId: "stale-thread",
        turn: { id: "stale-turn", status: "inProgress" },
      },
    });
    await codex.listModels();
    await nextMacrotask();

    assert.equal(launch, 2);
    assert.equal(codex.connected, true);
    assert.equal(codex.isTurnActive("stale-thread", "stale-turn"), false);
    assert.equal(codex.notificationSnapshot({ threadId: "stale-thread" }).length, 0);
  } finally {
    await codex.close();
  }
});
