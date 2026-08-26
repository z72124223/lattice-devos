import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { existsSync } from "node:fs";
import path from "node:path";
import readline from "node:readline";

export class CodexAppServer extends EventEmitter {
  constructor({
    codexBin = null,
    spawnProcess = spawn,
    requestTimeoutMs = 15_000,
    lifecycleTimeoutMs = 30_000,
  } = {}) {
    super();
    this.codexBin = codexBin;
    this.spawnProcess = spawnProcess;
    this.requestTimeoutMs = requestTimeoutMs;
    this.lifecycleTimeoutMs = lifecycleTimeoutMs;
    this.process = null;
    this.connectionGeneration = 0;
    this.ready = false;
    this.connectPromise = null;
    this.nextId = 1;
    this.pending = new Map();
    this.serverRequests = new Map();
    this.notificationSequence = 0;
    this.notificationHistory = [];
    this.notificationWaiters = new Set();
    this.activeTurns = new Map();
  }

  get connected() {
    return Boolean(this.ready && this.process && this.process.exitCode === null);
  }

  get pendingRequestCount() {
    return this.pending.size;
  }

  get pendingNotificationCount() {
    return this.notificationWaiters.size;
  }

  get pendingServerRequestCount() {
    return this.serverRequests.size;
  }

  notificationSnapshot({ method = null, threadId = null, turnId = null } = {}) {
    return this.notificationHistory.filter(({ message }) => {
      const observedThreadId = message.params?.threadId ?? message.params?.thread?.id ?? null;
      const observedTurnId = message.params?.turnId ?? message.params?.turn?.id ?? null;
      return (method === null || message.method === method)
        && (threadId === null || observedThreadId === threadId)
        && (turnId === null || observedTurnId === turnId);
    }).map((entry) => structuredClone(entry));
  }

  async connect() {
    if (this.connected) return;
    if (this.connectPromise) return this.connectPromise;
    const attempt = this.#connectOnce();
    this.connectPromise = attempt;
    try {
      await attempt;
    } finally {
      if (this.connectPromise === attempt) this.connectPromise = null;
    }
  }

  async #connectOnce() {
    let command = this.codexBin || "codex";
    let args = ["app-server", "--stdio"];
    if (process.platform === "win32" && !this.codexBin) {
      const codexScript = path.join(
        process.env.APPDATA || "",
        "npm",
        "node_modules",
        "@openai",
        "codex",
        "bin",
        "codex.js",
      );
      if (!existsSync(codexScript)) {
        throw new Error("Codex npm runtime was not found; set codexBin to an exact trusted path");
      }
      command = process.execPath;
      args = [codexScript, "app-server", "--stdio"];
    }
    const child = this.spawnProcess(command, args, {
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });
    const generation = this.connectionGeneration += 1;
    this.process = child;
    this.ready = false;
    this.notificationSequence = 0;
    this.notificationHistory = [];
    this.activeTurns.clear();
    this.#clearServerRequests();
    const lines = readline.createInterface({ input: child.stdout });
    this.lines = lines;
    lines.on("line", (line) => this.#receive(line, child, generation));
    child.once("exit", (code, signal) => {
      lines.close();
      if (this.process !== child) return;
      const error = new Error(`Codex App Server exited (${code ?? signal ?? "unknown"})`);
      this.ready = false;
      this.process = null;
      if (this.lines === lines) this.lines = null;
      this.#rejectOutstanding(error);
      this.emit("disconnect", { code, signal });
    });
    child.once("error", (cause) => {
      lines.close();
      if (this.process !== child) return;
      const error = new Error(`Unable to start Codex App Server: ${cause.message}`);
      this.ready = false;
      this.process = null;
      if (this.lines === lines) this.lines = null;
      this.#rejectOutstanding(error);
      this.emit("disconnect", { code: cause.code ?? null, signal: null });
    });
    child.stderr.on("data", (chunk) => this.emit("diagnostic", String(chunk)));

    try {
      await this.#request("initialize", {
        clientInfo: {
          name: "lattice_control",
          title: "LATTICE Control",
          version: "0.1.0",
        },
      }, { allowUnready: true });
      this.#send({ method: "initialized" }, { allowUnready: true });
      this.ready = true;
    } catch (error) {
      if (this.process === child) await this.close();
      throw error;
    }
  }

  async close() {
    const child = this.process;
    const lines = this.lines;
    this.ready = false;
    this.connectPromise = null;
    const error = new Error("Codex App Server connection closed");
    this.#rejectOutstanding(error);
    this.activeTurns.clear();
    if (!child) return;
    this.process = null;
    if (this.lines === lines) this.lines = null;
    lines?.close();
    child.stdin.end();
    if (child.exitCode === null) child.kill();
    this.emit("disconnect", { code: null, signal: "client-close" });
  }

  #send(message, { allowUnready = false } = {}) {
    const transportOpen = Boolean(this.process && this.process.exitCode === null);
    if (!transportOpen || (!allowUnready && !this.ready)) {
      throw new Error("Codex App Server is not connected");
    }
    this.process.stdin.write(`${JSON.stringify(message)}\n`);
  }

  send(message) {
    this.#send(message);
  }

  notify(method, params) {
    this.#send(params === undefined ? { method } : { method, params });
  }

  request(method, params = {}, options = {}) {
    return this.#request(method, params, options);
  }

  #request(
    method,
    params = {},
    { timeoutMs = this.requestTimeoutMs, allowUnready = false, signal = null } = {},
  ) {
    if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
      throw new TypeError("request timeout must be a positive number");
    }
    const id = this.nextId;
    this.nextId += 1;
    return new Promise((resolve, reject) => {
      const pending = {
        resolve,
        reject,
        timer: null,
        method,
        signal,
        abortListener: null,
        cleanup: null,
      };
      pending.cleanup = () => {
        clearTimeout(pending.timer);
        if (pending.signal && pending.abortListener) {
          pending.signal.removeEventListener("abort", pending.abortListener);
        }
      };
      pending.timer = setTimeout(() => {
        if (this.pending.get(id) !== pending) return;
        this.pending.delete(id);
        pending.cleanup();
        const error = new Error(`Codex App Server ${method} request timed out after ${timeoutMs}ms`);
        error.code = "CODEX_APP_SERVER_TIMEOUT";
        error.method = method;
        error.requestId = id;
        reject(error);
      }, timeoutMs);
      pending.timer.unref?.();
      this.pending.set(id, pending);
      if (signal) {
        pending.abortListener = () => {
          if (this.pending.get(id) !== pending) return;
          this.pending.delete(id);
          pending.cleanup();
          const error = new Error(`Codex App Server ${method} request was cancelled`);
          error.code = "CODEX_APP_SERVER_REQUEST_CANCELLED";
          error.method = method;
          error.requestId = id;
          reject(error);
        };
        if (signal.aborted) {
          pending.abortListener();
          return;
        }
        signal.addEventListener("abort", pending.abortListener, { once: true });
      }
      try {
        this.#send({ method, id, params }, { allowUnready });
      } catch (error) {
        if (this.pending.get(id) === pending) {
          this.pending.delete(id);
          pending.cleanup();
          reject(error);
        }
      }
    });
  }

  respond(id, result) {
    this.#settleServerRequest(id, { id, result });
  }

  rejectServerRequest(
    id,
    { code = -32601, message = "Unsupported Codex App Server request", data } = {},
  ) {
    const error = { code, message };
    if (data !== undefined) error.data = data;
    this.#settleServerRequest(id, { id, error });
  }

  deferServerRequest(id, { timeoutMs = this.requestTimeoutMs } = {}) {
    if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
      throw new TypeError("server request timeout must be a positive number");
    }
    const request = this.serverRequests.get(id);
    if (!request || request.state !== "received") {
      throw new Error(`Codex App Server request ${id} cannot be deferred`);
    }
    request.state = "deferred";
    request.timer = setTimeout(() => {
      if (!this.serverRequests.has(id)) return;
      try {
        this.rejectServerRequest(id, {
          code: -32001,
          message: `Codex App Server ${request.method} request timed out after ${timeoutMs}ms`,
        });
      } catch (error) {
        this.#clearServerRequest(id);
        this.emit("diagnostic", `Unable to reject timed out App Server request: ${error.message}`);
      }
    }, timeoutMs);
    request.timer.unref?.();
  }

  async listModels() {
    await this.connect();
    return this.request("model/list", { limit: 100 });
  }

  async startThread({ cwd, model = "gpt-5.6-terra", ...options }) {
    await this.connect();
    const result = await this.request("thread/start", { cwd, model, ...options });
    return result.thread;
  }

  async readThread(threadId, { includeTurns = true } = {}) {
    await this.connect();
    const result = await this.request("thread/read", { threadId, includeTurns });
    const thread = result?.thread;
    if (!thread || thread.id !== threadId) {
      const error = new Error(`Codex thread ${threadId} reconciliation failed`);
      error.code = "CODEX_THREAD_NOT_RECOVERABLE";
      throw error;
    }
    if (includeTurns && (!Array.isArray(thread.turns) || thread.turns.length === 0)) {
      const error = new Error(`Codex thread ${threadId} has an empty rollout and is not recoverable`);
      error.code = "CODEX_THREAD_NOT_RECOVERABLE";
      throw error;
    }
    return thread;
  }

  async resumeThread(threadId) {
    await this.connect();
    const result = await this.request("thread/resume", { threadId });
    const resumed = result?.thread;
    if (
      !resumed
      || resumed.id !== threadId
      || !Array.isArray(resumed.turns)
      || resumed.turns.length === 0
    ) {
      const error = new Error(`Codex thread ${threadId} resume reconciliation failed`);
      error.code = "CODEX_THREAD_NOT_RECOVERABLE";
      throw error;
    }
    const latestTurn = resumed.turns.at(-1);
    if (!latestTurn || !["completed", "interrupted", "failed"].includes(latestTurn.status)) {
      const error = new Error(`Codex thread ${threadId} does not have a terminal turn for reconciliation`);
      error.code = "CODEX_THREAD_NOT_RECOVERABLE";
      throw error;
    }
    const reconciled = await this.readThread(threadId, { includeTurns: true });
    const reconciledLatest = reconciled.turns.at(-1);
    if (
      reconciledLatest?.id !== latestTurn.id
      || reconciledLatest?.status !== latestTurn.status
    ) {
      const error = new Error(`Codex thread ${threadId} changed during resume reconciliation`);
      error.code = "CODEX_THREAD_NOT_RECOVERABLE";
      throw error;
    }
    return reconciled;
  }

  async startTurn(threadId, text) {
    await this.connect();
    const result = await this.request("turn/start", {
      threadId,
      input: [{ type: "text", text }],
    });
    return result.turn;
  }

  waitForThreadStarted(threadId, { timeoutMs = this.lifecycleTimeoutMs } = {}) {
    return this.#waitForNotification({
      method: "thread/started",
      threadId,
      timeoutMs,
    }).then((message) => message.params.thread);
  }

  waitForTurnStarted(threadId, turnId, { timeoutMs = this.lifecycleTimeoutMs } = {}) {
    return this.#waitForNotification({
      method: "turn/started",
      threadId,
      turnId,
      timeoutMs,
    }).then((message) => message.params.turn);
  }

  waitForTurnCompleted(
    threadId,
    turnId,
    { timeoutMs = this.lifecycleTimeoutMs, statuses = ["completed", "interrupted", "failed"], signal } = {},
  ) {
    return this.#waitForNotification({
      method: "turn/completed",
      threadId,
      turnId,
      turnStatuses: statuses,
      timeoutMs,
      signal,
    }).then((message) => message.params.turn);
  }

  isTurnActive(threadId, turnId) {
    return this.activeTurns.get(threadId) === turnId;
  }

  async interruptTurn(threadId, turnId, { timeoutMs = this.lifecycleTimeoutMs } = {}) {
    if (!this.isTurnActive(threadId, turnId)) {
      throw new Error(`Codex turn ${threadId}/${turnId} is not active`);
    }
    const terminalController = new AbortController();
    const requestController = new AbortController();
    const terminal = this.waitForTurnCompleted(threadId, turnId, {
      timeoutMs,
      statuses: ["interrupted", "failed"],
      signal: terminalController.signal,
    });
    terminal.catch(() => {});
    const request = this.request(
      "turn/interrupt",
      { threadId, turnId },
      { timeoutMs, signal: requestController.signal },
    );
    request.catch(() => {});
    try {
      const first = await Promise.race([
        terminal.then((turn) => ({ kind: "terminal", turn })),
        request.then(() => ({ kind: "request" })),
      ]);
      if (first.kind === "terminal") {
        requestController.abort();
        await Promise.allSettled([request]);
        return first.turn;
      }
      return await terminal;
    } catch (error) {
      terminalController.abort();
      requestController.abort();
      if (error.code === "CODEX_APP_SERVER_TIMEOUT") {
        this.activeTurns.delete(threadId);
        await this.close();
      }
      throw error;
    }
  }

  async archiveThread(threadId) {
    await this.connect();
    return this.request("thread/archive", { threadId });
  }

  #waitForNotification({
    method,
    threadId = null,
    turnId = null,
    turnStatuses = null,
    timeoutMs,
    signal = null,
  }) {
    if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
      throw new TypeError("notification timeout must be a positive number");
    }
    const match = (entry) => {
      const message = entry.message;
      if (message.method !== method) return false;
      const observedThreadId = message.params?.threadId ?? message.params?.thread?.id ?? null;
      const observedTurnId = message.params?.turnId ?? message.params?.turn?.id ?? null;
      const observedTurnStatus = message.params?.turn?.status ?? null;
      return (threadId === null || observedThreadId === threadId)
        && (turnId === null || observedTurnId === turnId)
        && (turnStatuses === null || turnStatuses.includes(observedTurnStatus));
    };
    const observed = this.notificationHistory.findLast(match);
    if (observed) return Promise.resolve(observed.message);
    return new Promise((resolve, reject) => {
      const waiter = { match, resolve, reject, timer: null, signal, abortListener: null };
      const cleanup = () => {
        clearTimeout(waiter.timer);
        if (waiter.signal && waiter.abortListener) {
          waiter.signal.removeEventListener("abort", waiter.abortListener);
        }
      };
      waiter.cleanup = cleanup;
      waiter.timer = setTimeout(() => {
        if (!this.notificationWaiters.delete(waiter)) return;
        cleanup();
        const correlation = [threadId, turnId].filter(Boolean).join("/");
        const error = new Error(
          `Codex App Server ${method} notification${correlation ? ` for ${correlation}` : ""} timed out after ${timeoutMs}ms`,
        );
        error.code = "CODEX_APP_SERVER_TIMEOUT";
        error.method = method;
        error.threadId = threadId;
        error.turnId = turnId;
        reject(error);
      }, timeoutMs);
      waiter.timer.unref?.();
      this.notificationWaiters.add(waiter);
      if (signal) {
        waiter.abortListener = () => {
          if (!this.notificationWaiters.delete(waiter)) return;
          cleanup();
          const error = new Error(`Codex App Server ${method} notification wait was cancelled`);
          error.code = "CODEX_APP_SERVER_WAIT_CANCELLED";
          reject(error);
        };
        if (signal.aborted) waiter.abortListener();
        else signal.addEventListener("abort", waiter.abortListener, { once: true });
      }
    });
  }

  #rejectOutstanding(error) {
    for (const pending of this.pending.values()) {
      pending.cleanup?.();
      pending.reject(error);
    }
    this.pending.clear();
    for (const waiter of this.notificationWaiters) {
      waiter.cleanup?.();
      waiter.reject(error);
    }
    this.notificationWaiters.clear();
    this.#clearServerRequests();
  }

  #clearServerRequest(id) {
    const request = this.serverRequests.get(id);
    if (!request) return null;
    clearTimeout(request.timer);
    this.serverRequests.delete(id);
    return request;
  }

  #clearServerRequests() {
    for (const request of this.serverRequests.values()) clearTimeout(request.timer);
    this.serverRequests.clear();
  }

  #settleServerRequest(id, message) {
    const request = this.#clearServerRequest(id);
    if (!request) {
      throw new Error(`Codex App Server request ${id} is not pending`);
    }
    this.#send(message, { allowUnready: true });
    this.emit("serverRequestSettled", {
      id,
      method: request.method,
      response: message,
    });
  }

  #receive(line, child, generation) {
    if (this.process !== child || this.connectionGeneration !== generation) return;
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      this.emit("diagnostic", `Invalid App Server JSON: ${line.slice(0, 200)}`);
      return;
    }

    if (Object.hasOwn(message, "id") && !message.method) {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      pending.cleanup?.();
      if (message.error) {
        const error = new Error(message.error.message || "Codex App Server request failed");
        error.code = message.error.code ?? null;
        error.data = message.error.data ?? null;
        error.method = pending.method;
        pending.reject(error);
      } else {
        pending.resolve(message.result);
      }
      return;
    }

    if (Object.hasOwn(message, "id") && message.method) {
      const request = { id: message.id, method: message.method, state: "received", timer: null };
      this.serverRequests.set(message.id, request);
      try {
        this.emit("serverRequest", message);
      } catch (error) {
        this.emit("diagnostic", `App Server request handler failed: ${error.message}`);
        if (this.serverRequests.has(message.id)) {
          this.rejectServerRequest(message.id, {
            code: -32603,
            message: "Codex App Server request handler failed",
          });
        }
        return;
      }
      if (request.state === "received" && this.serverRequests.has(message.id)) {
        this.rejectServerRequest(message.id, {
          code: -32601,
          message: `Unsupported Codex App Server request: ${message.method}`,
        });
      }
      return;
    }

    if (message.method) {
      const threadId = message.params?.threadId ?? null;
      const turnId = message.params?.turn?.id ?? null;
      const turnStatus = message.params?.turn?.status ?? null;
      if (message.method === "turn/started" && threadId && turnId && turnStatus === "inProgress") {
        this.activeTurns.set(threadId, turnId);
      } else if (
        message.method === "turn/completed"
        && threadId
        && turnId
        && ["completed", "interrupted", "failed"].includes(turnStatus)
        && this.activeTurns.get(threadId) === turnId
      ) {
        this.activeTurns.delete(threadId);
      }
      const entry = {
        sequence: this.notificationSequence += 1,
        observedAt: new Date().toISOString(),
        message,
      };
      this.notificationHistory.push(entry);
      if (this.notificationHistory.length > 512) this.notificationHistory.shift();
      for (const waiter of [...this.notificationWaiters]) {
        if (!waiter.match(entry)) continue;
        this.notificationWaiters.delete(waiter);
        waiter.cleanup?.();
        waiter.resolve(message);
      }
      this.emit("notification", message, entry);
    }
  }
}
