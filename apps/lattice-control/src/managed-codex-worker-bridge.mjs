import readline from "node:readline";
import { createHash } from "node:crypto";

import { CodexAppServer } from "./codex-app-server.mjs";
import {
  ManagedCodexWorkerTransport,
  validateManagedCodexAuthContext,
  validateManagedCodexWorkerPacket,
} from "./managed-codex-worker.mjs";
import {
  buildWsl2CodexLaunch,
  validateWsl2ExecutionEnvironment,
} from "./wsl2-execution-domain.mjs";
import {
  buildWsl2ProviderSubtreeMarker,
  buildWsl2ProviderSubtreeReceipt,
} from "./wsl2-provider-subtree-reconcile.mjs";

const COMMAND_SCHEMA = "lattice.managed-codex-worker-command/1.0";
const CONTROL_SCHEMA = "lattice.managed-codex-worker-control/1.0";
const RESULT_SCHEMA = "lattice.managed-codex-worker-bridge-result/1.0";
const MAX_INPUT_BYTES = 65_536;
const MAX_CONNECTOR_TIMEOUT_MS = 120_000;
const HEX_64 = /^[a-f0-9]{64}$/u;

function sha256Hex(value) {
  return createHash("sha256").update(value, "utf8").digest("hex");
}

export function managedConnectorTimeoutMs(packet) {
  const validated = validateManagedCodexWorkerPacket(packet);
  return Math.min(validated.heartbeat_timeout_ms, MAX_CONNECTOR_TIMEOUT_MS);
}

function bridgeError(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}

function validateCommand(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw bridgeError("MANAGED_CODEX_INVALID_COMMAND", "bridge command must be an object");
  }
  if (value.schema !== COMMAND_SCHEMA || ![
    "probe",
    "start",
    "recover-dispatch",
    "recover-prestart",
    "continue-turn",
    "resume",
    "recover",
  ].includes(value.operation)) {
    throw bridgeError("MANAGED_CODEX_INVALID_COMMAND", "bridge command schema or operation is invalid");
  }
  const expected = new Set(value.operation === "recover"
    ? ["schema", "operation", "packet", "auth_context", "retained", "observation"]
    : value.operation === "resume"
        || value.operation === "continue-turn"
        || value.operation === "recover-prestart"
      ? ["schema", "operation", "packet", "auth_context", "retained"]
      : value.operation === "start" || value.operation === "recover-dispatch"
        ? ["schema", "operation", "packet", "auth_context", "claimed_at"]
        : ["schema", "operation", "packet", "auth_context"]);
  if (
    Object.keys(value).some((key) => !expected.has(key))
    || [...expected].some((key) => !Object.hasOwn(value, key))
  ) {
    throw bridgeError("MANAGED_CODEX_INVALID_COMMAND", "bridge command shape is invalid");
  }
  try {
    validateManagedCodexAuthContext(value.auth_context);
  } catch {
    throw bridgeError("MANAGED_CODEX_INVALID_COMMAND", "bridge auth context is invalid");
  }
  return value;
}

function validateControl(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw bridgeError("MANAGED_CODEX_INVALID_COMMAND", "bridge control must be an object");
  }
  const expected = new Set(value.operation === "authorize_provider_dispatch"
      || value.operation === "probe_provider_readiness"
    ? ["schema", "operation", "task_ref", "attempt", "packet_digest", "marker_digest"]
    : value.operation === "authorize_turn_start"
      ? ["schema", "operation", "task_ref", "attempt", "packet_digest", "thread_id"]
      : ["schema", "operation", "task_ref", "attempt", "packet_digest", "thread_id", "turn_id"]);
  if (
    value.schema !== CONTROL_SCHEMA
    || ![
      "probe_provider_readiness",
      "authorize_provider_dispatch",
      "authorize_turn_start",
      "interrupt",
    ]
      .includes(value.operation)
    || Object.keys(value).some((key) => !expected.has(key))
    || [...expected].some((key) => !Object.hasOwn(value, key))
  ) {
    throw bridgeError("MANAGED_CODEX_INVALID_COMMAND", "bridge control shape is invalid");
  }
  return value;
}

function providerDispatchAuthorization() {
  let settled = false;
  let resolveAuthorization;
  let rejectAuthorization;
  const authorization = new Promise((resolve, reject) => {
    resolveAuthorization = resolve;
    rejectAuthorization = reject;
  });
  return {
    accept(control) {
      if (settled) {
        throw bridgeError(
          "MANAGED_CODEX_PROVIDER_DISPATCH_AUTHORIZATION_REJECTED",
          "managed provider-dispatch authorization was duplicated",
        );
      }
      settled = true;
      resolveAuthorization(control);
    },
    reject(error) {
      if (settled) return;
      settled = true;
      rejectAuthorization(error);
    },
    async wait(identity) {
      const control = await authorization;
      if (
        control.task_ref !== identity.task_ref
        || control.attempt !== identity.attempt
        || control.packet_digest !== identity.packet_digest
        || control.marker_digest !== identity.marker_digest
      ) {
        throw bridgeError(
          "MANAGED_CODEX_PROVIDER_DISPATCH_AUTHORIZATION_REJECTED",
          "managed provider-dispatch authorization did not bind the exact OPEN marker",
        );
      }
    },
  };
}

function turnStartAuthorization() {
  let settled = false;
  let resolveAuthorization;
  let rejectAuthorization;
  const authorization = new Promise((resolve, reject) => {
    resolveAuthorization = resolve;
    rejectAuthorization = reject;
  });
  return {
    accept(control) {
      if (settled) {
        throw bridgeError(
          "MANAGED_CODEX_TURN_START_AUTHORIZATION_REJECTED",
          "managed turn-start authorization was duplicated",
        );
      }
      settled = true;
      resolveAuthorization(control);
    },
    reject(error) {
      if (settled) return;
      settled = true;
      rejectAuthorization(error);
    },
    async wait(identity) {
      const control = await authorization;
      if (
        control.task_ref !== identity.task_ref
        || control.attempt !== identity.attempt
        || control.packet_digest !== identity.packet_digest
        || control.thread_id !== identity.thread_id
      ) {
        throw bridgeError(
          "MANAGED_CODEX_TURN_START_AUTHORIZATION_REJECTED",
          "managed turn-start authorization did not bind the exact accepted thread",
        );
      }
    },
  };
}

function exitCodeFor(error) {
  if (error instanceof SyntaxError || error instanceof TypeError || error?.code === "MANAGED_CODEX_INVALID_COMMAND") {
    return 2;
  }
  if (error?.code === "MANAGED_CODEX_MODEL_UNAVAILABLE") return 3;
  if (
    error?.code === "MANAGED_CODEX_EXACT_LIFECYCLE_MISMATCH"
    || error?.code === "MANAGED_CODEX_RETAINED_IDENTITY_MISMATCH"
    || error?.code === "MANAGED_CODEX_DISPATCH_RECONCILIATION_REQUIRED"
    || error?.code === "CODEX_THREAD_NOT_RECOVERABLE"
  ) {
    return 4;
  }
  return 5;
}

function safeIdentity(command) {
  const packet = command?.packet;
  if (!packet || typeof packet !== "object") return {};
  const identity = {};
  if (
    typeof packet.task_ref === "string"
    && /^[a-z0-9][a-z0-9._:-]{0,127}$/u.test(packet.task_ref)
    && !/(?:^sk-|password|secret|token)/iu.test(packet.task_ref)
  ) {
    identity.task_ref = packet.task_ref;
  }
  if (Number.isSafeInteger(packet.attempt)) identity.attempt = packet.attempt;
  if (
    typeof packet.packet_digest === "string"
    && /^attempt-packet:sha256:[a-f0-9]{64}$/u.test(packet.packet_digest)
  ) {
    identity.packet_digest = packet.packet_digest;
  }
  return identity;
}

function safeErrorCode(error) {
  return typeof error?.code === "string"
    && /^(?:MANAGED_CODEX|CODEX)_[A-Z0-9_]{1,80}$/u.test(error.code)
    ? error.code
    : "MANAGED_CODEX_BRIDGE_FAILED";
}

const SAFE_PROVIDER_METHODS = new Set([
  "account/read",
  "initialize",
  "model/list",
  "thread/list",
  "thread/read",
  "thread/resume",
  "thread/started",
  "thread/start",
  "turn/interrupt",
  "turn/started",
  "turn/start",
]);

function safeProviderErrorEvidence(error) {
  const evidence = {};
  if (typeof error?.method === "string" && SAFE_PROVIDER_METHODS.has(error.method)) {
    evidence.provider_method = error.method;
  }
  if (
    error?.code === "CODEX_APP_SERVER_RPC_REJECTED"
    && Number.isSafeInteger(error?.rpcCode)
    && error.rpcCode >= -32_768
    && error.rpcCode <= 32_767
  ) {
    evidence.provider_rpc_code = error.rpcCode;
  }
  return evidence;
}

function writeRecord(record) {
  const encoded = JSON.stringify(record);
  if (Buffer.byteLength(encoded, "utf8") > 16_384) {
    throw bridgeError("MANAGED_CODEX_UNSAFE_EVIDENCE", "bridge output is unbounded");
  }
  process.stdout.write(`${encoded}\n`);
}

function openCommandChannel() {
  const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
  let settled = false;
  let controlHandler = null;
  const queuedControls = [];
  let resolveInitial;
  let rejectInitial;
  const initial = new Promise((resolve, reject) => {
    resolveInitial = resolve;
    rejectInitial = reject;
  });
  lines.on("line", (line) => {
    if (line.trim().length === 0) return;
    if (Buffer.byteLength(line, "utf8") > MAX_INPUT_BYTES) {
      const error = bridgeError("MANAGED_CODEX_INVALID_COMMAND", "bridge command exceeds the bounded input size");
      if (!settled) rejectInitial(error);
      else controlHandler?.(Promise.reject(error));
      settled = true;
      return;
    }
    try {
      const parsed = JSON.parse(line);
      if (!settled) {
        const validated = validateCommand(parsed);
        settled = true;
        resolveInitial(validated);
      } else {
        const control = validateControl(parsed);
        if (controlHandler) controlHandler(Promise.resolve(control));
        else queuedControls.push(control);
      }
    } catch (error) {
      if (!settled) {
        settled = true;
        rejectInitial(error);
      } else if (controlHandler) {
        controlHandler(Promise.reject(error));
      }
    }
  });
  lines.once("close", () => {
    if (!settled) rejectInitial(bridgeError("MANAGED_CODEX_INVALID_COMMAND", "bridge command is missing"));
  });
  return {
    initial,
    setControlHandler(handler) {
      controlHandler = handler;
      for (const control of queuedControls.splice(0)) handler(Promise.resolve(control));
    },
    close() { lines.close(); },
  };
}

export async function runManagedCodexWorkerBridge({
  codex = null,
  lifecycleTimeoutMs = 30_000,
} = {}) {
  let command = null;
  let connector = codex;
  const ownsConnector = codex === null;
  let connectorClosed = false;
  let providerOpenMarker = null;
  let providerClosedWritten = false;
  let providerReadinessStarted = false;
  let wslLaunch = null;
  const channel = openCommandChannel();
  try {
    command = await channel.initial;
    const connectorTimeoutMs = managedConnectorTimeoutMs(command.packet);
    wslLaunch = null;
    let providerReceiptContext = null;
    if (process.env.LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON) {
      let environment;
      let preflightReceipt;
      try {
        const descriptorJson = process.env.LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON;
        const descriptorDigest = process.env
          .LATTICE_MANAGED_EXECUTION_ENVIRONMENT_DESCRIPTOR_DIGEST;
        const preflightJson = process.env.LATTICE_MANAGED_EXECUTION_PREFLIGHT_JSON ?? "null";
        const preflightDescriptorDigest = process.env
          .LATTICE_MANAGED_EXECUTION_PREFLIGHT_DESCRIPTOR_DIGEST;
        const preflightContentDigest = process.env
          .LATTICE_MANAGED_EXECUTION_PREFLIGHT_CONTENT_DIGEST;
        if (!HEX_64.test(descriptorDigest ?? "") || sha256Hex(descriptorJson) !== descriptorDigest
          || !HEX_64.test(preflightDescriptorDigest ?? "")
          || !HEX_64.test(preflightContentDigest ?? "")
          || sha256Hex(preflightJson) !== preflightContentDigest) {
          throw bridgeError(
            "MANAGED_CODEX_EXECUTION_ENVIRONMENT_MISMATCH",
            "managed execution environment artifact digests differ",
          );
        }
        const configured = validateWsl2ExecutionEnvironment(JSON.parse(descriptorJson));
        if (configured.identity_digest !== command.packet.execution_environment_ref) {
          throw bridgeError(
            "MANAGED_CODEX_EXECUTION_ENVIRONMENT_MISMATCH",
            "managed packet and execution environment identities differ",
          );
        }
        preflightReceipt = JSON.parse(
          preflightJson,
        );
        if (
          preflightReceipt?.schema !== "lattice.wsl2-zero-model-preflight/1.0"
          || preflightReceipt.status !== "PASS"
          || preflightReceipt.task_ref !== command.packet.task_ref
          || preflightReceipt.attempt !== command.packet.attempt
          || preflightReceipt.worktree_ref !== command.packet.worktree_ref
          || preflightReceipt.repository_head !== command.packet.base_commit
          || preflightReceipt.execution_environment_ref !== configured.identity_digest
          || preflightReceipt.provider_effect_count !== 0
        ) {
          throw bridgeError(
            "MANAGED_CODEX_EXECUTION_ENVIRONMENT_MISMATCH",
            "managed preflight and packet identities differ",
          );
        }
        environment = configured;
        providerReceiptContext = {
          task_ref: command.packet.task_ref,
          attempt: command.packet.attempt,
          packet_digest: command.packet.packet_digest,
          worktree_ref: command.packet.worktree_ref,
          repository_head: command.packet.base_commit,
          execution_environment_ref: configured.identity_digest,
          descriptor_digest: descriptorDigest,
          source_preflight_descriptor_digest: preflightDescriptorDigest,
          source_preflight_content_digest: preflightContentDigest,
          source_preflight_receipt_digest: preflightReceipt.receipt_digest,
        };
      } catch (error) {
        if (error?.code === "MANAGED_CODEX_EXECUTION_ENVIRONMENT_MISMATCH") throw error;
        throw bridgeError(
          "MANAGED_CODEX_EXECUTION_ENVIRONMENT_REJECTED",
          "managed execution environment was not exact",
        );
      }
      if (environment.identity_digest !== command.packet.execution_environment_ref) {
        throw bridgeError(
          "MANAGED_CODEX_EXECUTION_ENVIRONMENT_MISMATCH",
          "managed packet and execution environment identities differ",
        );
      }
      wslLaunch = buildWsl2CodexLaunch(environment, {
        fence: preflightReceipt.process_fence.fence,
        preflightReceipt,
        attempt: command.packet.attempt,
        retryOf: preflightReceipt.continuation.retry_of,
        reconnectOf: preflightReceipt.continuation.reconnect_of,
        timeoutMs: Math.min(connectorTimeoutMs, 300_000),
        stdoutLimitBytes: 1_048_576,
        stderrLimitBytes: 1_048_576,
      });
    }
    connector ??= new CodexAppServer({
      codexBin: wslLaunch === null ? process.env.LATTICE_CODEX_BIN || null : null,
      launchSpec: wslLaunch,
      requestTimeoutMs: connectorTimeoutMs,
      lifecycleTimeoutMs: connectorTimeoutMs,
    });
    let resolveProviderMarker;
    let rejectProviderMarker;
    const providerMarkerReady = new Promise((resolve, reject) => {
      resolveProviderMarker = resolve;
      rejectProviderMarker = reject;
    });
    if (wslLaunch !== null && command.operation === "start") {
      connector.on?.("process-domain-marker", (processMarker) => {
        try {
          if (providerOpenMarker !== null) {
            throw bridgeError(
              "MANAGED_CODEX_PROVIDER_SUBTREE_MARKER_REJECTED",
              "managed provider OPEN marker was duplicated",
            );
          }
          providerOpenMarker = buildWsl2ProviderSubtreeMarker(
            providerReceiptContext,
            processMarker,
          );
          writeRecord({ kind: "provider_subtree_marker", marker: providerOpenMarker });
          resolveProviderMarker(providerOpenMarker);
        } catch (error) {
          rejectProviderMarker(error);
        }
      });
    }
    let diagnosticCount = 0;
    connector.on?.("diagnostic", () => {
      if (diagnosticCount >= 8) return;
      diagnosticCount += 1;
      process.stderr.write("Codex App Server diagnostic observed\n");
    });
    const authorization = turnStartAuthorization();
    const providerAuthorization = providerDispatchAuthorization();
    const worker = new ManagedCodexWorkerTransport({
      codex: connector,
      authContext: command.auth_context,
      lifecycleTimeoutMs: Math.min(lifecycleTimeoutMs, connectorTimeoutMs),
      turnStartAuthorizer: (identity) => authorization.wait(identity),
      eventSink: async (event) => writeRecord({ kind: "event", event }),
    });
    let resolveControl;
    let rejectControl;
    const controlTerminal = new Promise((resolve, reject) => {
      resolveControl = resolve;
      rejectControl = reject;
    });
    channel.setControlHandler((pending) => {
      pending
        .then((control) => {
          if (control.operation === "probe_provider_readiness") {
            if (wslLaunch === null || command.operation !== "start"
              || providerOpenMarker === null || providerReadinessStarted
              || control.task_ref !== command.packet.task_ref
              || control.attempt !== command.packet.attempt
              || control.packet_digest !== command.packet.packet_digest
              || control.marker_digest !== providerOpenMarker.marker_digest) {
              throw bridgeError(
                "MANAGED_CODEX_PROVIDER_READINESS_REJECTED",
                "managed provider-readiness control did not bind the exact OPEN segment",
              );
            }
            providerReadinessStarted = true;
            return worker.probe(command.packet).then((result) => {
              writeRecord({
                kind: "provider_model_availability",
                status: "AVAILABLE",
                task_ref: command.packet.task_ref,
                attempt: command.packet.attempt,
                packet_digest: command.packet.packet_digest,
                marker_digest: providerOpenMarker.marker_digest,
                model: result.model,
                auth_readiness: result.auth_readiness,
                code: null,
              });
            }).catch((error) => {
              writeRecord({
                kind: "provider_model_availability",
                status: error?.code === "MANAGED_CODEX_MODEL_UNAVAILABLE"
                  ? "UNAVAILABLE"
                  : "ERROR",
                task_ref: command.packet.task_ref,
                attempt: command.packet.attempt,
                packet_digest: command.packet.packet_digest,
                marker_digest: providerOpenMarker.marker_digest,
                model: command.packet.model,
                auth_readiness: null,
                code: safeErrorCode(error),
              });
            });
          }
          if (control.operation === "authorize_provider_dispatch") {
            providerAuthorization.accept(control);
            return undefined;
          }
          if (control.operation === "authorize_turn_start") {
            authorization.accept(control);
            return undefined;
          }
          return worker.interruptActive(command.packet, control).then(resolveControl);
        })
        .catch((error) => {
          providerAuthorization.reject(error);
          authorization.reject(error);
          rejectControl(error);
        });
    });
    if (wslLaunch !== null && command.operation === "start") {
      await connector.connect();
      const marker = await providerMarkerReady;
      await providerAuthorization.wait({
        task_ref: command.packet.task_ref,
        attempt: command.packet.attempt,
        packet_digest: command.packet.packet_digest,
        marker_digest: marker.marker_digest,
      });
    }
    const operationPromise = command.operation === "probe"
      ? await worker.probe(command.packet)
      : command.operation === "start"
        ? worker.start(command.packet, command.claimed_at)
        : command.operation === "recover-dispatch"
          ? worker.recoverClaimedDispatch(command.packet, command.claimed_at)
        : command.operation === "recover-prestart"
          ? worker.recoverPrestart(command.packet, command.retained)
        : command.operation === "continue-turn"
          ? worker.continueTurn(command.packet, command.retained)
        : command.operation === "resume"
          ? worker.resume(command.packet, command.retained)
          : worker.recoverTimedStall(command.packet, command.retained, command.observation);
    const result = command.operation === "probe"
      ? operationPromise
      : await Promise.race([operationPromise, controlTerminal]);
    channel.close();
    const shutdown = await connector.close?.();
    connectorClosed = true;
    if (ownsConnector && shutdown?.exited !== true) {
      throw bridgeError(
        "MANAGED_CODEX_CONNECTOR_STILL_ACTIVE",
        "owned Codex App Server did not provide an exact process-exit receipt",
      );
    }
    if (connector.connected === true) {
      throw bridgeError("MANAGED_CODEX_CONNECTOR_STILL_ACTIVE", "Codex connector remained active after close");
    }
    if (wslLaunch !== null && command.operation === "start") {
      if (providerOpenMarker === null || providerClosedWritten
        || shutdown?.process_marker === undefined || shutdown?.subtree_exit === undefined
        || shutdown?.outer_post_exit === undefined) {
        throw bridgeError(
          "MANAGED_CODEX_PROVIDER_SUBTREE_RECEIPT_REJECTED",
          "managed provider subtree close was not exact",
        );
      }
      const receipt = buildWsl2ProviderSubtreeReceipt(
        providerOpenMarker,
        shutdown.subtree_exit,
        shutdown.outer_post_exit,
        connector.providerEffects,
      );
      writeRecord({ kind: "provider_subtree_receipt", receipt });
      providerClosedWritten = true;
    }
    writeRecord({
      schema: RESULT_SCHEMA,
      kind: "result",
      operation: command.operation,
      ...safeIdentity(command),
      result,
    });
    return TERMINAL_FAILURES.has(result.status) ? 6 : 0;
  } catch (error) {
    let reportedError = error;
    if (!connectorClosed) {
      try {
        const shutdown = await connector?.close?.();
        connectorClosed = true;
        if (wslLaunch !== null && command?.operation === "start"
          && providerOpenMarker !== null && !providerClosedWritten) {
          const receipt = buildWsl2ProviderSubtreeReceipt(
            providerOpenMarker,
            shutdown?.subtree_exit,
            shutdown?.outer_post_exit,
            connector.providerEffects,
          );
          writeRecord({ kind: "provider_subtree_receipt", receipt });
          providerClosedWritten = true;
        }
      } catch (closeError) {
        reportedError = closeError;
      }
    }
    const exitCode = exitCodeFor(reportedError);
    writeRecord({
      schema: RESULT_SCHEMA,
      kind: "error",
      category: exitCode,
      code: safeErrorCode(reportedError),
      ...safeProviderErrorEvidence(reportedError),
      ...safeIdentity(command),
      message: "managed Codex worker bridge failed closed",
    });
    return exitCode;
  } finally {
    channel.close();
    if (!connectorClosed) await connector?.close?.().catch(() => {});
  }
}

const TERMINAL_FAILURES = new Set(["interrupted", "failed"]);

if (process.argv[1] && import.meta.filename === process.argv[1]) {
  process.exitCode = await runManagedCodexWorkerBridge();
}
