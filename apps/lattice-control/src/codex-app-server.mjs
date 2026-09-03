import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { EventEmitter } from "node:events";
import { existsSync } from "node:fs";
import path from "node:path";
import readline from "node:readline";

import {
  WSL2_PROCESS_MARKER_SCHEMA,
  WSL2_SUBTREE_EXIT_SCHEMA,
} from "./wsl2-execution-domain.mjs";
import { probeWsl2ProviderPostExit } from "./wsl2-provider-subtree-reconcile.mjs";

const ATTEMPT_RECEIPT = /^attempt-receipt:sha256:[a-f0-9]{64}$/u;
const HEX_64 = /^[a-f0-9]{64}$/u;
const TOOL_INPUT_KEYS = Object.freeze([
  "executable", "verifier_tool", "sandbox_helper", "node_runtime", "rustc", "rustdoc",
  "keyring_daemon", "keyring_libraries",
]);
const SEAL_KEYS = Object.freeze([
  "path", "resolved_path", "sha256", "device", "inode", "owner_uid", "mode", "size",
]);
const KEYRING_LIBRARY_FILES = Object.freeze(["libgck-1.so.0.0.0", "libgcr-base-3.so.1.0.0"]);

function exactObjectKeys(value, keys) {
  return value !== null
    && typeof value === "object"
    && !Array.isArray(value)
    && Object.keys(value).sort().join("\0") === [...keys].sort().join("\0");
}

function launchOption(args, name) {
  const key = `--${name}`;
  const indexes = args.flatMap((value, index) => value === key ? [index] : []);
  if (indexes.length !== 1) return null;
  const value = args[indexes[0] + 1];
  return typeof value === "string" && value.length > 0 ? value : null;
}

function boundedInteger(value, minimum, maximum) {
  if (typeof value !== "string" || !/^\d+$/u.test(value)) return null;
  const number = Number(value);
  return Number.isSafeInteger(number) && number >= minimum && number <= maximum
    ? number
    : null;
}

function validSupervisorSeal(value, { library = false, manifestPath = null } = {}) {
  const keys = library ? ["manifest_path", ...SEAL_KEYS] : SEAL_KEYS;
  return exactObjectKeys(value, keys)
    && typeof value.path === "string" && value.path.startsWith("/")
    && typeof value.resolved_path === "string" && value.resolved_path.startsWith("/")
    && HEX_64.test(value.sha256)
    && typeof value.device === "string" && /^\d+$/u.test(value.device)
    && typeof value.inode === "string" && /^\d+$/u.test(value.inode)
    && Number.isSafeInteger(value.owner_uid) && value.owner_uid >= 0
    && Number.isSafeInteger(value.mode) && value.mode > 0 && (value.mode & 0o022) === 0
    && Number.isSafeInteger(value.size) && value.size > 0
    && (!library || value.manifest_path === manifestPath);
}

function validProviderToolInputs(value) {
  return exactObjectKeys(value, TOOL_INPUT_KEYS)
    && validSupervisorSeal(value.executable)
    && value.verifier_tool === null
    && validSupervisorSeal(value.sandbox_helper)
    && value.node_runtime === null && value.rustc === null && value.rustdoc === null
    && validSupervisorSeal(value.keyring_daemon)
    && Array.isArray(value.keyring_libraries)
    && value.keyring_libraries.length === KEYRING_LIBRARY_FILES.length
    && value.keyring_libraries.every((entry, index) => validSupervisorSeal(entry, {
      library: true,
      manifestPath: KEYRING_LIBRARY_FILES[index],
    }));
}

function wsl2ProcessDomainContract(launchSpec) {
  const fence = launchSpec.processFence;
  const unit = launchSpec.serviceUnit;
  const identity = launchSpec.codexIdentity;
  const args = launchSpec.args;
  const executionEnvironmentRef = launchOption(args, "execution-environment-ref");
  const credentialSealDigest = launchOption(args, "credential-seal-digest");
  const attempt = boundedInteger(launchOption(args, "attempt"), 1, 100);
  const timeoutMs = boundedInteger(launchOption(args, "timeout-ms"), 1_000, 300_000);
  const stdoutLimitBytes = boundedInteger(
    launchOption(args, "stdout-limit-bytes"), 1_024, 1_048_576,
  );
  const stderrLimitBytes = boundedInteger(
    launchOption(args, "stderr-limit-bytes"), 1_024, 1_048_576,
  );
  const retryRaw = launchOption(args, "retry-of");
  const reconnectRaw = launchOption(args, "reconnect-of");
  const retryOf = retryRaw === "NONE" ? null : retryRaw;
  const reconnectOf = reconnectRaw === "NONE" ? null : reconnectRaw;
  const valid = typeof fence === "string" && /^[a-f0-9]{64}$/u.test(fence)
    && typeof unit === "string"
    && new RegExp(`^lattice-wsl2-[a-f0-9]{16}-provider-${fence.slice(0, 12)}\\.service$`, "u")
      .test(unit)
    && launchSpec.gracefulClose === true
    && identity?.schema === "lattice.wsl2-codex-launch/1.1"
    && executionEnvironmentRef !== null
    && /^execution-environment:sha256:[a-f0-9]{64}$/u.test(executionEnvironmentRef)
    && credentialSealDigest !== null
    && /^credential-seal:sha256:[a-f0-9]{64}$/u.test(credentialSealDigest)
    && identity.execution_environment_ref === executionEnvironmentRef
    && identity.credential_seal_digest === credentialSealDigest
    && identity.process_fence === fence
    && launchOption(args, "role") === "PROVIDER"
    && launchOption(args, "fence") === fence
    && launchOption(args, "unit") === unit
    && attempt !== null && timeoutMs !== null
    && stdoutLimitBytes !== null && stderrLimitBytes !== null
    && retryRaw !== null && reconnectRaw !== null
    && (retryOf === null || ATTEMPT_RECEIPT.test(retryOf))
    && (reconnectOf === null || ATTEMPT_RECEIPT.test(reconnectOf))
    && (retryOf === null || reconnectOf === null)
    && ((attempt === 1 && retryOf === null)
      || (attempt > 1 && (retryOf !== null || reconnectOf !== null)));
  if (!valid) {
    const error = new Error("WSL2 App Server launch process-domain contract was not exact");
    error.code = "CODEX_APP_SERVER_LAUNCH_REJECTED";
    throw error;
  }
  return Object.freeze({
    fence,
    unit,
    executionEnvironmentRef,
    credentialSealDigest,
    attempt,
    retryOf,
    reconnectOf,
    timeoutMs,
    stdoutLimitBytes,
    stderrLimitBytes,
  });
}

export function resolveCodexAppServerLaunch(
  codexBin,
  { platform = process.platform, env = process.env } = {},
) {
  const args = ["app-server", "--stdio"];
  if (platform !== "win32" || typeof codexBin !== "string" || !/\.cmd$/iu.test(codexBin)) {
    return Object.freeze({ command: codexBin, args });
  }
  const systemRoot = env.SystemRoot ?? env.WINDIR ?? "";
  const comSpec = env.ComSpec ?? "";
  const expectedComSpec = path.win32.join(systemRoot, "System32", "cmd.exe");
  if (
    env.LATTICE_DELIVERY_CODEX_MODE !== "SCRIPTED_ACCEPTANCE"
    || !path.win32.isAbsolute(codexBin)
    || /["&|<>^%!\r\n]/u.test(codexBin)
    || !path.win32.isAbsolute(comSpec)
    || path.win32.normalize(comSpec).toLowerCase()
      !== path.win32.normalize(expectedComSpec).toLowerCase()
  ) {
    const error = new Error("Codex command scripts are restricted to scripted acceptance");
    error.code = "CODEX_APP_SERVER_SCRIPT_REJECTED";
    throw error;
  }
  return Object.freeze({
    command: comSpec,
    args: ["/d", "/s", "/c", "call", codexBin, ...args],
  });
}

export class CodexAppServer extends EventEmitter {
  constructor({
    codexBin = null,
    launchSpec = null,
    spawnProcess = spawn,
    requestTimeoutMs = 15_000,
    lifecycleTimeoutMs = 30_000,
    sessionIdentityFactory = () => `app-server-session:sha256:${randomBytes(32).toString("hex")}`,
    runPostExitProbe = probeWsl2ProviderPostExit,
  } = {}) {
    super();
    this.codexBin = codexBin;
    this.launchSpec = launchSpec;
    this.spawnProcess = spawnProcess;
    this.requestTimeoutMs = requestTimeoutMs;
    this.lifecycleTimeoutMs = lifecycleTimeoutMs;
    this.sessionIdentityFactory = sessionIdentityFactory;
    this.runPostExitProbe = runPostExitProbe;
    this.process = null;
    this.closingProcess = null;
    this.closePromise = null;
    this.connectionGeneration = 0;
    this.currentGenerationRecord = null;
    this.closingGenerationRecord = null;
    this.appServerSessionId = null;
    this.ready = false;
    this.connectPromise = null;
    this.nextId = 1;
    this.pending = new Map();
    this.serverRequests = new Map();
    this.notificationSequence = 0;
    this.notificationHistory = [];
    this.notificationWaiters = new Set();
    this.activeTurns = new Map();
    this.processDomainIdentity = null;
    this.subtreeExitReceipt = null;
    this.outerPostExitReceipt = null;
    this.providerEffectCount = 0;
    this.diagnosticDrain = null;
    this.processDomainContract = null;
    this.processDomainDeadline = null;
    this.processDomainWaiter = null;
    this.processDomainFailure = null;
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

  get providerEffects() {
    return this.providerEffectCount;
  }

  async readAuthReadiness() {
    await this.connect();
    const generation = this.connectionGeneration;
    const sessionId = this.appServerSessionId;
    let result;
    try {
      result = await this.request("account/read", { refreshToken: false }, {
        expectedGeneration: generation,
        expectedSessionId: sessionId,
      });
    } catch {
      const error = new Error("Codex account readiness could not be verified");
      error.code = "CODEX_APP_SERVER_AUTH_READINESS_REJECTED";
      error.method = "account/read";
      throw error;
    }
    if (
      !result
      || typeof result !== "object"
      || Array.isArray(result)
      || typeof result.requiresOpenaiAuth !== "boolean"
      || (result.account !== null && (
        !result.account
        || typeof result.account !== "object"
        || Array.isArray(result.account)
      ))
      || this.connectionGeneration !== generation
      || this.appServerSessionId !== sessionId
      || !this.connected
    ) {
      const error = new Error("Codex account readiness response was not exact");
      error.code = "CODEX_APP_SERVER_AUTH_READINESS_REJECTED";
      error.method = "account/read";
      throw error;
    }
    const accountType = result.account?.type;
    const authMode = ["chatgpt", "apiKey", "amazonBedrock"].includes(accountType)
      ? accountType
      : null;
    return Object.freeze({
      schema: "lattice.codex-auth-readiness/1.0",
      ready: authMode === "chatgpt" && result.requiresOpenaiAuth === true,
      authMode,
      appServerGeneration: generation,
      appServerSessionId: sessionId,
    });
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

  #emitDisconnectOnce(record, details) {
    if (!record || record.disconnectEmitted) return false;
    record.disconnectEmitted = true;
    for (const listener of this.rawListeners("disconnect")) {
      try {
        listener.call(this, details);
      } catch {
        record.disconnectListenerFailure = true;
      }
    }
    return true;
  }

  #finalizeGenerationExit(record, code, signal) {
    if (!record || record.exitObserved) return;
    record.exitObserved = true;
    record.lines?.close();
    const child = record.child;
    const isCurrent = this.currentGenerationRecord === record;
    const isClosing = this.closingGenerationRecord === record;
    const ownedClose = record.closing === true;
    const error = new Error(`Codex App Server exited (${code ?? signal ?? "unknown"})`);
    if (!ownedClose) error.code = "CODEX_APP_SERVER_PROCESS_EXITED";
    if (
      isCurrent
      && !ownedClose
      && this.processDomainContract
      && !this.subtreeExitReceipt
    ) {
      this.processDomainFailure ??= error;
    }
    if (isCurrent || isClosing) {
      this.#rejectProcessDomainWaiter(error, child, record.generation);
    }
    if (isCurrent) {
      this.ready = false;
      if (this.process === child) this.process = null;
      this.currentGenerationRecord = null;
      this.appServerSessionId = null;
      this.#rejectOutstanding(error);
    }
    if (isClosing) {
      if (this.closingProcess === child) this.closingProcess = null;
      this.closingGenerationRecord = null;
      this.appServerSessionId = null;
      this.diagnosticDrain = null;
    }
    if (this.lines === record.lines) this.lines = null;
    this.#emitDisconnectOnce(record, ownedClose
      ? { code: null, signal: "client-close" }
      : { code, signal });
  }

  async connect() {
    if (this.closePromise) await this.closePromise;
    const retained = this.currentGenerationRecord ?? this.closingGenerationRecord;
    if (retained && !retained.exitObserved && retained.child.exitCode !== null) {
      this.#finalizeGenerationExit(
        retained,
        retained.child.exitCode,
        retained.child.signalCode ?? null,
      );
    }
    if (this.connectPromise) return this.connectPromise;
    if (this.closingGenerationRecord && !this.closingGenerationRecord.exitObserved) {
      const error = new Error(
        "Codex App Server reconnect requires the prior owned process to exit first",
      );
      error.code = "CODEX_APP_SERVER_CLOSE_PENDING";
      throw error;
    }
    if (
      this.currentGenerationRecord
      && !this.currentGenerationRecord.exitObserved
      && this.currentGenerationRecord.child.exitCode === null
      && !this.connected
    ) {
      const error = new Error(
        "Codex App Server has a live unready process and cannot start a replacement",
      );
      error.code = "CODEX_APP_SERVER_PROCESS_STILL_ACTIVE";
      throw error;
    }
    if (this.connected) return;
    const unresolvedProcessDomain = this.#unresolvedProcessDomainFailure();
    if (unresolvedProcessDomain) throw unresolvedProcessDomain;
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
    if (this.launchSpec !== null) {
      if (
        !this.launchSpec
        || typeof this.launchSpec.command !== "string"
        || !path.win32.isAbsolute(this.launchSpec.command)
        || !Array.isArray(this.launchSpec.args)
        || this.launchSpec.args.some((arg) => typeof arg !== "string" || arg.includes("\0"))
      ) {
        const error = new Error("Codex App Server launch specification was not exact");
        error.code = "CODEX_APP_SERVER_LAUNCH_REJECTED";
        throw error;
      }
      ({ command, args } = this.launchSpec);
      this.processDomainContract = this.launchSpec.processFence
        ? wsl2ProcessDomainContract(this.launchSpec)
        : null;
    } else if (process.platform === "win32" && !this.codexBin) {
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
    } else if (process.platform === "win32" && this.codexBin) {
      ({ command, args } = resolveCodexAppServerLaunch(this.codexBin));
    }
    const sessionId = this.sessionIdentityFactory();
    if (!/^app-server-session:sha256:[0-9a-f]{64}$/u.test(sessionId)) {
      const error = new Error("Codex App Server session identity was not exact");
      error.code = "CODEX_APP_SERVER_SESSION_IDENTITY_INVALID";
      throw error;
    }
    const child = this.spawnProcess(command, args, {
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });
    const generation = this.connectionGeneration += 1;
    const generationRecord = {
      generation,
      child,
      disconnectEmitted: false,
      disconnectListenerFailure: false,
      exitObserved: false,
      closing: false,
      lines: null,
    };
    this.process = child;
    this.currentGenerationRecord = generationRecord;
    this.appServerSessionId = sessionId;
    this.ready = false;
    this.notificationSequence = 0;
    this.notificationHistory = [];
    this.activeTurns.clear();
    this.processDomainIdentity = null;
    this.subtreeExitReceipt = null;
    this.outerPostExitReceipt = null;
    this.providerEffectCount = 0;
    this.processDomainFailure = null;
    this.processDomainWaiter = null;
    this.processDomainDeadline = this.processDomainContract
      ? Date.now() + this.lifecycleTimeoutMs
      : null;
    this.#clearServerRequests();
    const lines = readline.createInterface({ input: child.stdout });
    generationRecord.lines = lines;
    this.lines = lines;
    lines.on("line", (line) => this.#receive(line, child, generation));
    child.once("exit", (code, signal) => {
      this.#finalizeGenerationExit(generationRecord, code, signal);
    });
    child.on("error", (cause) => {
      lines.close();
      if (
        this.currentGenerationRecord !== generationRecord
        && this.closingGenerationRecord !== generationRecord
      ) return;
      const error = new Error(`Unable to start Codex App Server: ${cause.message}`);
      error.code = "CODEX_APP_SERVER_TRANSPORT_ERROR";
      error.causeCode = cause.code ?? null;
      this.ready = false;
      // An error event is not an exact process-exit receipt. Retain ownership
      // so close/reconnect cannot lose or replace a child whose exitCode is
      // still null (for example when kill itself reports an error).
      if (this.lines === lines) this.lines = null;
      this.#rejectOutstanding(error);
      this.#emitDisconnectOnce(generationRecord, { code: cause.code ?? null, signal: null });
    });
    const diagnosticLines = readline.createInterface({ input: child.stderr });
    this.diagnosticDrain = new Promise((resolve) => {
      if (child.stderr.readableEnded || child.stderr.destroyed) resolve();
      else diagnosticLines.once("close", resolve);
    });
    diagnosticLines.on("line", (line) => {
      this.#observeProcessDomainLine(line, child, generation);
      this.emit("diagnostic", line);
    });

    try {
      await this.#request("initialize", {
        clientInfo: {
          name: "lattice_control",
          title: "LATTICE Control",
          version: "0.1.0",
        },
      }, { allowUnready: true });
      this.#send({ method: "initialized" }, { allowUnready: true });
      if (this.processDomainContract) {
        await this.#awaitProcessDomainIdentity(child, generation);
      }
      this.ready = true;
    } catch (error) {
      if (this.process === child) await this.close();
      throw error;
    }
  }

  close() {
    if (this.closePromise) return this.closePromise;
    const attempt = Promise.resolve().then(() => this.#closeOnce());
    let tracked;
    tracked = attempt.finally(() => {
      if (this.closePromise === tracked) this.closePromise = null;
    });
    this.closePromise = tracked;
    return tracked;
  }

  async #closeOnce() {
    let record = this.currentGenerationRecord ?? this.closingGenerationRecord;
    if (record && !record.exitObserved && record.child.exitCode !== null) {
      this.#finalizeGenerationExit(
        record,
        record.child.exitCode,
        record.child.signalCode ?? null,
      );
      record = this.currentGenerationRecord ?? this.closingGenerationRecord;
    }
    const child = record?.child ?? this.process ?? this.closingProcess;
    const generation = this.connectionGeneration;
    const lines = this.lines;
    const diagnosticDrain = this.diagnosticDrain;
    this.ready = false;
    this.connectPromise = null;
    const error = new Error("Codex App Server connection closed");
    this.#rejectOutstanding(error);
    this.activeTurns.clear();
    if (!child) {
      const unresolvedProcessDomain = this.#unresolvedProcessDomainFailure();
      if (unresolvedProcessDomain) throw unresolvedProcessDomain;
      return Object.freeze({
        exited: true,
        processId: null,
        ...(this.processDomainContract ? {
          process_marker: this.processDomainIdentity,
          subtree_exit: this.subtreeExitReceipt,
          outer_post_exit: this.outerPostExitReceipt,
        } : {}),
      });
    }
    const processId = Number.isSafeInteger(child.pid) && child.pid > 0 ? child.pid : null;
    if (!record) {
      const error = new Error("Codex App Server process generation was unavailable");
      error.code = "CODEX_APP_SERVER_GENERATION_MISSING";
      throw error;
    }
    record.closing = true;
    this.closingProcess = child;
    this.closingGenerationRecord = record;
    this.#emitDisconnectOnce(record, { code: null, signal: "client-close" });
    if (this.lines === lines) this.lines = null;
    lines?.close();
    const exit = child.exitCode !== null
      ? Promise.resolve({ code: child.exitCode, signal: child.signalCode ?? null })
      : new Promise((resolve) => {
        child.once("exit", (code, signal) => resolve({ code, signal }));
      });
    child.stdin.end();
    if (child.exitCode === null && this.launchSpec?.gracefulClose !== true) child.kill();
    let timer;
    const timeout = new Promise((_, reject) => {
      timer = setTimeout(() => {
        const failure = new Error(
          `Codex App Server did not exit within ${this.lifecycleTimeoutMs}ms`,
        );
        failure.code = "CODEX_APP_SERVER_CLOSE_TIMEOUT";
        reject(failure);
      }, this.lifecycleTimeoutMs);
      timer.unref?.();
    });
    try {
      const terminal = await Promise.race([exit, timeout]);
      if (this.launchSpec?.processFence && diagnosticDrain) {
        await Promise.race([diagnosticDrain, timeout]);
      }
      if (
        this.currentGenerationRecord === record
        || this.closingGenerationRecord === record
      ) {
        if (this.currentGenerationRecord === record) {
          this.currentGenerationRecord = null;
          if (this.process === child) this.process = null;
        }
        if (this.closingGenerationRecord === record) {
          this.closingGenerationRecord = null;
          if (this.closingProcess === child) this.closingProcess = null;
        }
        this.appServerSessionId = null;
        this.diagnosticDrain = null;
      }
      if (this.processDomainContract) {
        const unresolvedProcessDomain = this.#unresolvedProcessDomainFailure();
        if (unresolvedProcessDomain) throw unresolvedProcessDomain;
        this.outerPostExitReceipt = Object.freeze(await this.runPostExitProbe(
          this.launchSpec,
          this.processDomainIdentity,
        ));
      }
      return Object.freeze({
        exited: true,
        processId,
        code: terminal.code ?? null,
        signal: terminal.signal ?? null,
        ...(this.processDomainContract ? {
          process_marker: this.processDomainIdentity,
          subtree_exit: this.subtreeExitReceipt,
          outer_post_exit: this.outerPostExitReceipt,
        } : {}),
      });
    } finally {
      clearTimeout(timer);
    }
  }

  #unresolvedProcessDomainFailure() {
    if (!this.processDomainContract || this.connectionGeneration === 0) return null;
    if (this.processDomainFailure) return this.processDomainFailure;
    if (this.processDomainIdentity && this.subtreeExitReceipt) return null;
    const failure = new Error(
      "WSL2 Codex subtree did not provide an exact zero-descendant 1.1 receipt",
    );
    failure.code = "CODEX_APP_SERVER_SUBTREE_EXIT_REQUIRED";
    this.processDomainFailure = failure;
    return failure;
  }

  #awaitProcessDomainIdentity(child, generation) {
    if (!this.processDomainContract) return Promise.resolve();
    if (this.processDomainFailure) return Promise.reject(this.processDomainFailure);
    if (this.processDomainIdentity) return Promise.resolve();
    if (this.process !== child || child.exitCode !== null) {
      const error = new Error("WSL2 App Server exited before its process fence was observed");
      error.code = "CODEX_APP_SERVER_PROCESS_EXITED";
      return Promise.reject(error);
    }
    const remaining = this.processDomainDeadline - Date.now();
    if (!Number.isFinite(remaining) || remaining <= 0) {
      const error = new Error("WSL2 process fence identity was not observed before readiness");
      error.code = "CODEX_APP_SERVER_PROCESS_FENCE_REQUIRED";
      return Promise.reject(error);
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (this.processDomainWaiter?.child !== child
          || this.processDomainWaiter?.generation !== generation) return;
        this.processDomainWaiter = null;
        const error = new Error("WSL2 process fence identity was not observed before readiness");
        error.code = "CODEX_APP_SERVER_PROCESS_FENCE_REQUIRED";
        this.processDomainFailure ??= error;
        reject(error);
      }, remaining);
      this.processDomainWaiter = { child, generation, resolve, reject, timer };
    });
  }

  #rejectProcessDomainWaiter(error, child, generation) {
    const waiter = this.processDomainWaiter;
    if (!waiter || waiter.child !== child || waiter.generation !== generation) return;
    clearTimeout(waiter.timer);
    this.processDomainWaiter = null;
    waiter.reject(error);
  }

  #recordProcessDomainFailure(error, child, generation) {
    if (
      this.connectionGeneration !== generation
      || (this.process !== child && this.closingProcess !== child)
    ) return;
    this.processDomainFailure ??= error;
    this.#rejectProcessDomainWaiter(this.processDomainFailure, child, generation);
  }

  #observeProcessDomainLine(line, child, generation) {
    if (!this.processDomainContract || typeof line !== "string" || line.length > 4_096) return;
    if (
      this.connectionGeneration !== generation
      || (this.process !== child && this.closingProcess !== child)
    ) return;
    let value;
    try {
      value = JSON.parse(line);
    } catch {
      return;
    }
    if (value?.schema === WSL2_PROCESS_MARKER_SCHEMA) {
      const contract = this.processDomainContract;
      const valid = exactObjectKeys(value, [
        "schema", "fence", "unit", "execution_environment_ref", "credential_seal_digest",
        "boot_id_digest", "pid", "process_start_ticks", "process_group_id", "cgroup_path",
        "cgroup_version", "delegated", "attempt", "retry_of", "reconnect_of",
      ])
        && value.fence === contract.fence
        && value.unit === contract.unit
        && value.execution_environment_ref === contract.executionEnvironmentRef
        && value.credential_seal_digest === contract.credentialSealDigest
        && /^wsl-boot:sha256:[a-f0-9]{64}$/u.test(value.boot_id_digest)
        && Number.isSafeInteger(value.pid) && value.pid > 0
        && typeof value.process_start_ticks === "string" && /^[1-9]\d*$/u.test(value.process_start_ticks)
        && Number.isSafeInteger(value.process_group_id) && value.process_group_id > 0
        && typeof value.cgroup_path === "string" && value.cgroup_path.length <= 1_024
        && value.cgroup_path.startsWith("/user.slice/")
        && value.cgroup_path.endsWith(`/${contract.unit}`)
        && !value.cgroup_path.includes("..") && !value.cgroup_path.includes("\\")
        && value.cgroup_version === 2 && value.delegated === false
        && value.attempt === contract.attempt
        && value.retry_of === contract.retryOf
        && value.reconnect_of === contract.reconnectOf
        && this.processDomainIdentity === null
        && Date.now() <= this.processDomainDeadline;
      if (!valid) {
        const error = new Error("WSL2 process fence 1.1 marker was not exact");
        error.code = "CODEX_APP_SERVER_PROCESS_FENCE_REQUIRED";
        this.#recordProcessDomainFailure(error, child, generation);
        return;
      }
      this.processDomainIdentity = Object.freeze({ ...value });
      this.emit("process-domain-marker", this.processDomainIdentity);
      const waiter = this.processDomainWaiter;
      if (waiter?.child === child && waiter.generation === generation) {
        clearTimeout(waiter.timer);
        this.processDomainWaiter = null;
        waiter.resolve();
      }
    } else if (value?.schema === WSL2_SUBTREE_EXIT_SCHEMA) {
      const marker = this.processDomainIdentity;
      const contract = this.processDomainContract;
      const valid = exactObjectKeys(value, [
        "schema", "fence", "unit", "execution_environment_ref", "credential_seal_digest",
        "cgroup_path", "zero_descendants", "credential_seal_intact", "credential_watch_intact",
        "keyring_daemon_sha256", "keyring_library_manifest_digest", "tool_input_identities",
        "stdout_bytes", "stderr_bytes", "stdout_limit_bytes", "stderr_limit_bytes",
        "output_bound_exceeded", "timeout_ms", "timed_out", "interrupted", "stdin_bytes",
        "stdin_sha256", "stdin_complete", "attempt", "retry_of", "reconnect_of", "exit_code",
        "exit_signal",
      ])
        && marker !== null && this.subtreeExitReceipt === null
        && value.fence === marker.fence && value.fence === contract.fence
        && value.unit === marker.unit && value.unit === contract.unit
        && value.execution_environment_ref === marker.execution_environment_ref
        && value.execution_environment_ref === contract.executionEnvironmentRef
        && value.credential_seal_digest === marker.credential_seal_digest
        && value.credential_seal_digest === contract.credentialSealDigest
        && value.cgroup_path === marker.cgroup_path
        && value.zero_descendants === true && value.credential_seal_intact === true
        && value.credential_watch_intact === true
        && HEX_64.test(value.keyring_daemon_sha256)
        && /^keyring-library-manifest:sha256:[a-f0-9]{64}$/u.test(
          value.keyring_library_manifest_digest,
        )
        && validProviderToolInputs(value.tool_input_identities)
        && Number.isSafeInteger(value.stdout_bytes) && value.stdout_bytes >= 0
        && value.stdout_bytes <= value.stdout_limit_bytes
        && Number.isSafeInteger(value.stderr_bytes) && value.stderr_bytes >= 0
        && value.stderr_bytes <= value.stderr_limit_bytes
        && value.stdout_limit_bytes === contract.stdoutLimitBytes
        && value.stderr_limit_bytes === contract.stderrLimitBytes
        && value.output_bound_exceeded === false
        && value.timeout_ms === contract.timeoutMs
        && value.timed_out === false && value.interrupted === false
        && Number.isSafeInteger(value.stdin_bytes) && value.stdin_bytes >= 0
        && HEX_64.test(value.stdin_sha256) && value.stdin_complete === true
        && value.attempt === marker.attempt && value.attempt === contract.attempt
        && value.retry_of === marker.retry_of && value.retry_of === contract.retryOf
        && value.reconnect_of === marker.reconnect_of
        && value.reconnect_of === contract.reconnectOf
        && (value.exit_code === null
          || (Number.isSafeInteger(value.exit_code) && value.exit_code >= 0 && value.exit_code <= 255))
        && (value.exit_signal === null
          || (typeof value.exit_signal === "string" && /^SIG[A-Z0-9]{1,24}$/u.test(value.exit_signal)));
      if (!valid) {
        const error = new Error("WSL2 subtree exit 1.2 receipt was not exact");
        error.code = "CODEX_APP_SERVER_SUBTREE_EXIT_REQUIRED";
        this.#recordProcessDomainFailure(error, child, generation);
        return;
      }
      this.subtreeExitReceipt = Object.freeze({ ...value });
    } else if (
      typeof value?.schema === "string"
      && (value.schema.startsWith("lattice.wsl2-process-fence/")
        || value.schema.startsWith("lattice.wsl2-subtree-exit/"))
    ) {
      const marker = value.schema.startsWith("lattice.wsl2-process-fence/");
      const error = new Error(`WSL2 ${marker ? "process fence" : "subtree exit"} schema was not ${marker ? "1.1" : "1.2"}`);
      error.code = marker
        ? "CODEX_APP_SERVER_PROCESS_FENCE_REQUIRED"
        : "CODEX_APP_SERVER_SUBTREE_EXIT_REQUIRED";
      this.#recordProcessDomainFailure(error, child, generation);
    }
  }

  #send(
    message,
    {
      allowUnready = false,
      expectedGeneration = null,
      expectedSessionId = null,
    } = {},
  ) {
    const transportOpen = Boolean(this.process && this.process.exitCode === null);
    if (!transportOpen || (!allowUnready && !this.ready)) {
      throw new Error("Codex App Server is not connected");
    }
    if (
      (expectedGeneration !== null && this.connectionGeneration !== expectedGeneration)
      || (expectedSessionId !== null && this.appServerSessionId !== expectedSessionId)
    ) {
      const error = new Error("Codex App Server effect identity changed before dispatch");
      error.code = "CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED";
      throw error;
    }
    if (message?.method === "thread/start" || message?.method === "turn/start") {
      this.providerEffectCount += 1;
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
    {
      timeoutMs = this.requestTimeoutMs,
      allowUnready = false,
      signal = null,
      expectedGeneration = null,
      expectedSessionId = null,
    } = {},
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
        this.#send({ method, id, params }, {
          allowUnready,
          expectedGeneration,
          expectedSessionId,
        });
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

  async listModels({ effectIdentity = null } = {}) {
    await this.connect();
    return this.request("model/list", { limit: 100 }, effectIdentity ?? {});
  }

  async startThread({ cwd, model = "gpt-5.6-terra", effectIdentity = null, ...options }) {
    await this.connect();
    const result = await this.request(
      "thread/start",
      { cwd, model, ...options },
      effectIdentity ?? {},
    );
    return result.thread;
  }

  async listThreads({
    cwd,
    cursor = null,
    limit = 100,
    sortKey = "created_at",
    sortDirection = "desc",
    archived = false,
    sourceKinds = null,
    useStateDbOnly = false,
    effectIdentity = null,
  } = {}) {
    await this.connect();
    const result = await this.request("thread/list", {
      cwd,
      cursor,
      limit,
      sortKey,
      sortDirection,
      archived,
      useStateDbOnly,
      ...(sourceKinds === null ? {} : { sourceKinds }),
    }, effectIdentity ?? {});
    if (!result || !Array.isArray(result.data)) {
      const error = new Error("Codex thread/list returned an invalid page");
      error.code = "CODEX_THREAD_LIST_NOT_RECOVERABLE";
      throw error;
    }
    if (result.nextCursor !== undefined
      && result.nextCursor !== null
      && typeof result.nextCursor !== "string") {
      const error = new Error("Codex thread/list returned an invalid cursor");
      error.code = "CODEX_THREAD_LIST_NOT_RECOVERABLE";
      throw error;
    }
    return {
      data: result.data,
      nextCursor: result.nextCursor ?? null,
    };
  }

  async readThread(
    threadId,
    { includeTurns = true, allowEmpty = false, effectIdentity = null } = {},
  ) {
    await this.connect();
    const result = await this.request(
      "thread/read",
      { threadId, includeTurns },
      effectIdentity ?? {},
    );
    const thread = result?.thread;
    if (!thread || thread.id !== threadId) {
      const error = new Error(`Codex thread ${threadId} reconciliation failed`);
      error.code = "CODEX_THREAD_NOT_RECOVERABLE";
      throw error;
    }
    if (includeTurns && (!Array.isArray(thread.turns) || (!allowEmpty && thread.turns.length === 0))) {
      const error = new Error(`Codex thread ${threadId} has an empty rollout and is not recoverable`);
      error.code = "CODEX_THREAD_NOT_RECOVERABLE";
      throw error;
    }
    return thread;
  }

  async readThreadFresh(
    threadId,
    { includeTurns = true, allowEmpty = false } = {},
  ) {
    if (this.launchSpec?.processFence) {
      const error = new Error("a process-fenced App Server launch cannot be reused for a fresh probe");
      error.code = "CODEX_APP_SERVER_FRESH_READ_UNAVAILABLE";
      throw error;
    }
    // The active App Server can retain an in-memory inProgress view after a
    // terminal notification is lost. A separate read-only process consults
    // persisted Codex state without closing or interrupting the active adapter.
    const probe = new CodexAppServer({
      codexBin: this.codexBin,
      launchSpec: this.launchSpec,
      spawnProcess: this.spawnProcess,
      requestTimeoutMs: this.requestTimeoutMs,
      lifecycleTimeoutMs: this.lifecycleTimeoutMs,
      sessionIdentityFactory: this.sessionIdentityFactory,
      runPostExitProbe: this.runPostExitProbe,
    });
    try {
      return await probe.readThread(threadId, { includeTurns, allowEmpty });
    } finally {
      await probe.close();
    }
  }

  /** Loads one exact persisted thread that is proven to have no turns yet. */
  async resumeEmptyThread(threadId, { effectIdentity = null } = {}) {
    await this.connect();
    const result = await this.request(
      "thread/resume",
      { threadId },
      effectIdentity ?? {},
    );
    const resumed = result?.thread;
    if (resumed?.id !== threadId || !Array.isArray(resumed.turns) || resumed.turns.length !== 0) {
      const error = new Error(`Codex thread ${threadId} is not an exact empty rollout`);
      error.code = "CODEX_THREAD_NOT_RECOVERABLE";
      throw error;
    }
    const reconciled = await this.readThread(threadId, {
      includeTurns: true,
      allowEmpty: true,
      effectIdentity,
    });
    if (reconciled.turns.length !== 0) {
      const error = new Error(`Codex thread ${threadId} changed during empty-thread reconciliation`);
      error.code = "CODEX_THREAD_NOT_RECOVERABLE";
      throw error;
    }
    return reconciled;
  }

  async resumeThread(threadId, { expectedTurnId = null, effectIdentity = null } = {}) {
    await this.connect();
    const result = await this.request(
      "thread/resume",
      { threadId },
      effectIdentity ?? {},
    );
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
    const isActive = latestTurn?.status === "inProgress";
    const isTerminal = ["completed", "interrupted", "failed"].includes(latestTurn?.status);
    if (
      !latestTurn
      || (!isActive && !isTerminal)
      || (isActive && (!expectedTurnId || latestTurn.id !== expectedTurnId))
      || (expectedTurnId && latestTurn.id !== expectedTurnId)
    ) {
      const error = new Error(`Codex thread ${threadId} does not match the retained turn for reconciliation`);
      error.code = "CODEX_THREAD_NOT_RECOVERABLE";
      throw error;
    }
    const reconciled = await this.readThread(threadId, { includeTurns: true, effectIdentity });
    const reconciledLatest = reconciled.turns.at(-1);
    if (
      reconciledLatest?.id !== latestTurn.id
      || reconciledLatest?.status !== latestTurn.status
    ) {
      const error = new Error(`Codex thread ${threadId} changed during resume reconciliation`);
      error.code = "CODEX_THREAD_NOT_RECOVERABLE";
      throw error;
    }
    if (isActive) this.activeTurns.set(threadId, latestTurn.id);
    else this.activeTurns.delete(threadId);
    return reconciled;
  }

  async startTurn(threadId, text, { effectIdentity = null } = {}) {
    await this.connect();
    const result = await this.request("turn/start", {
      threadId,
      input: [{ type: "text", text }],
    }, effectIdentity ?? {});
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

  hasActiveTurnOtherThan(threadId, turnId) {
    return [...this.activeTurns].some(
      ([activeThreadId, activeTurnId]) => activeThreadId !== threadId || activeTurnId !== turnId,
    );
  }

  async interruptTurn(
    threadId,
    turnId,
    { timeoutMs = this.lifecycleTimeoutMs, effectIdentity = null } = {},
  ) {
    if (!this.isTurnActive(threadId, turnId)) {
      throw new Error(`Codex turn ${threadId}/${turnId} is not active`);
    }
    const terminalController = new AbortController();
    const requestController = new AbortController();
    const terminal = this.waitForTurnCompleted(threadId, turnId, {
      timeoutMs,
      statuses: ["completed", "interrupted", "failed"],
      signal: terminalController.signal,
    });
    terminal.catch(() => {});
    const request = this.request(
      "turn/interrupt",
      { threadId, turnId },
      { timeoutMs, signal: requestController.signal, ...(effectIdentity ?? {}) },
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

  async archiveThread(threadId, { effectIdentity = null } = {}) {
    await this.connect();
    return this.request("thread/archive", { threadId }, effectIdentity ?? {});
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
      const waiter = {
        match,
        resolve,
        reject,
        timer: null,
        signal,
        abortListener: null,
        method,
        threadId,
        turnId,
      };
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
    for (const [requestId, pending] of this.pending) {
      pending.cleanup?.();
      const correlated = new Error(error.message);
      correlated.code = error.code ?? null;
      correlated.causeCode = error.causeCode ?? null;
      correlated.method = pending.method;
      correlated.requestId = requestId;
      pending.reject(correlated);
    }
    this.pending.clear();
    for (const waiter of this.notificationWaiters) {
      waiter.cleanup?.();
      const correlated = new Error(error.message);
      correlated.code = error.code ?? null;
      correlated.causeCode = error.causeCode ?? null;
      correlated.method = waiter.method;
      correlated.threadId = waiter.threadId;
      correlated.turnId = waiter.turnId;
      waiter.reject(correlated);
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
        error.code = "CODEX_APP_SERVER_RPC_REJECTED";
        error.rpcCode = Number.isSafeInteger(message.error.code) ? message.error.code : null;
        error.data = message.error.data ?? null;
        error.method = pending.method;
        error.requestId = message.id;
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
