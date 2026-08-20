import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { existsSync } from "node:fs";
import path from "node:path";
import readline from "node:readline";

export class CodexAppServer extends EventEmitter {
  constructor({ codexBin = null, spawnProcess = spawn } = {}) {
    super();
    this.codexBin = codexBin;
    this.spawnProcess = spawnProcess;
    this.process = null;
    this.nextId = 1;
    this.pending = new Map();
  }

  get connected() {
    return Boolean(this.process && this.process.exitCode === null);
  }

  async connect() {
    if (this.connected) return;
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
    this.process = this.spawnProcess(command, args, {
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });
    this.process.once("exit", (code, signal) => {
      const error = new Error(`Codex App Server exited (${code ?? signal ?? "unknown"})`);
      for (const pending of this.pending.values()) pending.reject(error);
      this.pending.clear();
      this.emit("disconnect", { code, signal });
    });
    this.process.once("error", (cause) => {
      const error = new Error(`Unable to start Codex App Server: ${cause.message}`);
      for (const pending of this.pending.values()) pending.reject(error);
      this.pending.clear();
      this.emit("disconnect", { code: cause.code ?? null, signal: null });
    });
    this.process.stderr.on("data", (chunk) => this.emit("diagnostic", String(chunk)));
    this.lines = readline.createInterface({ input: this.process.stdout });
    this.lines.on("line", (line) => this.#receive(line));

    await this.request("initialize", {
      clientInfo: {
        name: "lattice_control",
        title: "LATTICE Control",
        version: "0.1.0",
      },
    });
    this.notify("initialized", {});
  }

  async close() {
    if (!this.process) return;
    this.lines?.close();
    this.process.stdin.end();
    if (this.process.exitCode === null) this.process.kill();
    this.process = null;
  }

  send(message) {
    if (!this.connected) throw new Error("Codex App Server is not connected");
    this.process.stdin.write(`${JSON.stringify(message)}\n`);
  }

  notify(method, params = {}) {
    this.send({ method, params });
  }

  request(method, params = {}) {
    const id = this.nextId;
    this.nextId += 1;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ method, id, params });
    });
  }

  respond(id, result) {
    this.send({ id, result });
  }

  async listModels() {
    await this.connect();
    return this.request("model/list", { limit: 100 });
  }

  async startThread({ cwd, model = "gpt-5.6-terra" }) {
    await this.connect();
    const result = await this.request("thread/start", { cwd, model });
    return result.thread;
  }

  async resumeThread(threadId) {
    await this.connect();
    const result = await this.request("thread/resume", { threadId });
    return result.thread;
  }

  async startTurn(threadId, text) {
    await this.connect();
    const result = await this.request("turn/start", {
      threadId,
      input: [{ type: "text", text }],
    });
    return result.turn;
  }

  async archiveThread(threadId) {
    await this.connect();
    return this.request("thread/archive", { threadId });
  }

  #receive(line) {
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
      if (message.error) {
        pending.reject(new Error(message.error.message || "Codex App Server request failed"));
      } else {
        pending.resolve(message.result);
      }
      return;
    }

    if (Object.hasOwn(message, "id") && message.method) {
      this.emit("serverRequest", message);
      return;
    }

    if (message.method) this.emit("notification", message);
  }
}
