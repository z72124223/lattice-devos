import { randomUUID } from "node:crypto";
import { fork } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { createLatticeServer } from "../apps/lattice-control/src/server.mjs";

const scriptPath = fileURLToPath(import.meta.url);
const repositoryRoot = path.resolve(path.dirname(scriptPath), "..");
const childMode = process.argv[2] === "--control-child";

async function runControlChild() {
  const databasePath = process.env.LATTICE_CONVERSATION_ACCEPTANCE_DB;
  const port = Number(process.env.LATTICE_CONVERSATION_ACCEPTANCE_PORT);
  if (!databasePath || !Number.isInteger(port) || port < 1 || port > 65_535) {
    throw new Error("acceptance child configuration is invalid");
  }
  const application = createLatticeServer({ databasePath });
  const adapterKind = application.codex?.constructor?.name;
  if (adapterKind !== "CodexAppServer") {
    throw new Error(`acceptance child used unexpected adapter ${adapterKind ?? "unknown"}`);
  }
  await new Promise((resolve, reject) => {
    application.server.once("error", reject);
    application.server.listen(port, "127.0.0.1", resolve);
  });
  process.send?.({ type: "ready", pid: process.pid, port, adapterKind });
  let closing = false;
  process.on("message", async (message) => {
    if (message?.type !== "shutdown" || closing) return;
    closing = true;
    try {
      await new Promise((resolve) => application.server.close(resolve));
      await application.codex.close();
      await new Promise((resolve) => {
        if (process.send) process.send({ type: "stopped", pid: process.pid }, resolve);
        else resolve();
      });
      process.disconnect?.();
      process.exit(0);
    } catch (error) {
      await new Promise((resolve) => {
        if (process.send) process.send({ type: "stop-failed", message: error.message }, resolve);
        else resolve();
      });
      process.disconnect?.();
      process.exit(1);
    }
  });
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function freeLoopbackPort() {
  const { createServer } = await import("node:net");
  const probe = createServer();
  await new Promise((resolve, reject) => {
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", resolve);
  });
  const port = probe.address().port;
  await new Promise((resolve) => probe.close(resolve));
  return port;
}

function boundedErrorText(value, limit = 4_096) {
  const text = String(value ?? "");
  return text.length <= limit ? text : `${text.slice(0, limit)} [truncated]`;
}

async function startControl({ databasePath, port }) {
  const child = fork(scriptPath, ["--control-child"], {
    cwd: repositoryRoot,
    env: {
      ...process.env,
      LATTICE_CONVERSATION_ACCEPTANCE_DB: databasePath,
      LATTICE_CONVERSATION_ACCEPTANCE_PORT: String(port),
    },
    stdio: ["ignore", "pipe", "pipe", "ipc"],
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => {
    stderr = boundedErrorText(`${stderr}${chunk.toString("utf8")}`);
  });
  let ready;
  try {
    ready = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error(`Control child readiness timed out: ${stderr}`));
      }, 30_000);
      child.once("error", (error) => {
        clearTimeout(timer);
        reject(error);
      });
      child.once("exit", (code, signal) => {
        clearTimeout(timer);
        reject(new Error(`Control child exited before ready (${code ?? signal}): ${stderr}`));
      });
      child.on("message", (message) => {
        if (message?.type !== "ready") return;
        clearTimeout(timer);
        resolve(message);
      });
    });
  } catch (error) {
    if (child.exitCode === null) child.kill("SIGKILL");
    if (child.exitCode === null) {
      await new Promise((resolve) => {
        const timer = setTimeout(resolve, 5_000);
        child.once("exit", () => {
          clearTimeout(timer);
          resolve();
        });
      });
    }
    throw error;
  }
  return { child, ready, stderr: () => stderr };
}

async function waitForChildExit(child, timeoutMs) {
  if (child.exitCode !== null || child.signalCode !== null) return true;
  return new Promise((resolve) => {
    const onExit = () => {
      clearTimeout(timer);
      resolve(true);
    };
    const timer = setTimeout(() => {
      child.off("exit", onExit);
      resolve(false);
    }, timeoutMs);
    child.once("exit", onExit);
  });
}

async function stopControl(control) {
  if (
    !control?.child
    || control.child.exitCode !== null
    || control.child.signalCode !== null
  ) return;
  let shutdownError = null;
  await new Promise((resolve) => {
    const finish = () => {
      clearTimeout(timer);
      control.child.off("message", onMessage);
      control.child.off("exit", onExit);
      resolve();
    };
    const onMessage = (message) => {
      if (message?.type === "stopped") {
        finish();
      } else if (message?.type === "stop-failed") {
        shutdownError = new Error(message.message);
        control.child.kill("SIGKILL");
        finish();
      }
    };
    const onExit = (code, signal) => {
      if (code !== 0) {
        shutdownError = new Error(`Control child exited during shutdown (${code ?? signal})`);
      }
      finish();
    };
    const timer = setTimeout(() => {
      shutdownError = new Error("Control child graceful shutdown timed out");
      control.child.kill("SIGKILL");
      finish();
    }, 30_000);
    control.child.on("message", onMessage);
    control.child.once("exit", onExit);
    try {
      control.child.send({ type: "shutdown" });
    } catch (error) {
      shutdownError = error;
      control.child.kill("SIGKILL");
      finish();
    }
  });
  let reaped = await waitForChildExit(control.child, 5_000);
  if (!reaped) {
    control.child.kill("SIGKILL");
    reaped = await waitForChildExit(control.child, 5_000);
  }
  if (!reaped) throw new Error("Control child could not be reaped after shutdown");
  if (shutdownError) throw shutdownError;
  if (control.child.exitCode !== 0) {
    throw new Error(`Control child shutdown exited ${control.child.exitCode ?? control.child.signalCode}`);
  }
}

async function request(origin, pathname, { method = "GET", body } = {}) {
  const response = await fetch(`${origin}${pathname}`, {
    method,
    ...(body === undefined ? {} : {
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    }),
  });
  const payload = await response.json();
  if (!response.ok) {
    const error = new Error(payload.error || `HTTP ${response.status}`);
    error.code = payload.code;
    throw error;
  }
  return payload;
}

async function waitForConversation(origin, predicate, label, timeoutMs = 180_000) {
  const deadline = Date.now() + timeoutMs;
  let latest = null;
  while (Date.now() < deadline) {
    latest = await request(origin, "/api/conversation");
    if (predicate(latest)) return latest;
    if (latest.status === "failed") {
      throw new Error(`${label} failed: ${latest.last_error ?? latest.status_text}`);
    }
    await delay(500);
  }
  throw new Error(`${label} timed out from state ${latest?.status ?? "unknown"}`);
}

function latestAssistant(conversation) {
  return conversation.messages.filter(({ role }) => role === "assistant").at(-1) ?? null;
}

async function runAcceptance() {
  const runId = `${new Date().toISOString().replace(/[:.]/gu, "-")}-${randomUUID().slice(0, 8)}`;
  const artifactRoot = path.join(repositoryRoot, ".lattice", "acceptance", runId);
  const evidencePath = path.join(artifactRoot, "primary-conversation.json");
  const temporaryRoot = await mkdtemp(path.join(tmpdir(), "lattice-primary-live-"));
  const projectRoot = path.join(temporaryRoot, "project");
  const databasePath = path.join(temporaryRoot, "control.db");
  await mkdir(projectRoot, { recursive: true });
  await mkdir(artifactRoot, { recursive: true });
  const port = await freeLoopbackPort();
  const origin = `http://127.0.0.1:${port}`;
  const evidence = {
    schema_version: "lattice.control.primary-conversation-acceptance.v3",
    run_id: runId,
    started_at: new Date().toISOString(),
    status: "FAIL",
    transport: "official Codex App Server through LATTICE Control",
    mock_used: false,
    loopback_origin: origin,
    first_control: null,
    second_control: null,
    first_turn: null,
    restart_observation: null,
    second_turn: null,
    duplicate_replay: null,
    concurrent_writer_gate: null,
    active_restart: null,
    direct_interrupt: null,
    third_control: null,
    failure: null,
  };
  let control = null;
  let parallelControl = null;
  try {
    control = await startControl({ databasePath, port });
    evidence.first_control = {
      pid: control.ready.pid,
      adapter_kind: control.ready.adapterKind,
    };
    const project = await request(origin, "/api/projects", {
      method: "POST",
      body: { name: "Primary conversation live acceptance", rootPath: projectRoot },
    });
    const firstMessage = {
      projectId: project.id,
      clientMessageId: `live-first-${randomUUID()}`,
      text: "Reply with exactly LATTICE_PRIMARY_SESSION_1. Do not inspect or modify files. Do not call tools.",
    };
    await request(origin, "/api/conversation/messages", { method: "POST", body: firstMessage });
    const first = await waitForConversation(
      origin,
      (conversation) => conversation.status === "codex_done"
        && latestAssistant(conversation)?.text === "LATTICE_PRIMARY_SESSION_1",
      "first real Codex turn",
    );
    evidence.first_turn = {
      conversation_id: first.id,
      thread_id: first.codex_thread_id,
      turn_id: first.codex_turn_id,
      final_response: latestAssistant(first).text,
    };

    const parallelPort = await freeLoopbackPort();
    parallelControl = await startControl({ databasePath, port: parallelPort });
    let writerGateCode = null;
    try {
      await request(
        `http://127.0.0.1:${parallelPort}`,
        "/api/conversation/messages",
        { method: "POST", body: firstMessage },
      );
    } catch (error) {
      writerGateCode = error.code ?? null;
    }
    evidence.concurrent_writer_gate = {
      owner_pid: control.ready.pid,
      rejected_pid: parallelControl.ready.pid,
      rejection_code: writerGateCode,
    };
    if (
      parallelControl.ready.pid === control.ready.pid
      || writerGateCode !== "CONVERSATION_WRITER_BUSY"
    ) throw new Error("a second Control process was not rejected by the SQLite writer lease");
    await stopControl(parallelControl);
    parallelControl = null;

    await stopControl(control);
    control = await startControl({ databasePath, port });
    evidence.second_control = {
      pid: control.ready.pid,
      adapter_kind: control.ready.adapterKind,
    };
    const afterRestart = await request(origin, "/api/conversation");
    evidence.restart_observation = {
      conversation_id: afterRestart.id,
      thread_id: afterRestart.codex_thread_id,
      turn_id: afterRestart.codex_turn_id,
      message_count: afterRestart.messages.length,
      first_reply_retained: latestAssistant(afterRestart)?.text === "LATTICE_PRIMARY_SESSION_1",
    };
    if (
      evidence.first_control.pid === evidence.second_control.pid
      || afterRestart.id !== first.id
      || afterRestart.codex_thread_id !== first.codex_thread_id
      || !evidence.restart_observation.first_reply_retained
    ) throw new Error("Control restart did not retain the exact conversation binding");

    const secondMessage = {
      projectId: project.id,
      clientMessageId: `live-second-${randomUUID()}`,
      text: "Reply with exactly LATTICE_PRIMARY_SESSION_2. Do not inspect or modify files. Do not call tools.",
    };
    await request(origin, "/api/conversation/messages", { method: "POST", body: secondMessage });
    const second = await waitForConversation(
      origin,
      (conversation) => conversation.status === "codex_done"
        && latestAssistant(conversation)?.text === "LATTICE_PRIMARY_SESSION_2",
      "second real Codex turn after restart",
    );
    evidence.second_turn = {
      conversation_id: second.id,
      thread_id: second.codex_thread_id,
      turn_id: second.codex_turn_id,
      final_response: latestAssistant(second).text,
    };
    if (second.id !== first.id || second.codex_thread_id !== first.codex_thread_id) {
      throw new Error("second turn did not continue the retained conversation binding");
    }

    const duplicate = await request(origin, "/api/conversation/messages", {
      method: "POST",
      body: secondMessage,
    });
    await delay(1_000);
    const afterDuplicate = await request(origin, "/api/conversation");
    evidence.duplicate_replay = {
      conversation_id: duplicate.id,
      turn_id_before: second.codex_turn_id,
      turn_id_after: afterDuplicate.codex_turn_id,
      client_message_count: afterDuplicate.messages.filter(
        ({ id }) => id === secondMessage.clientMessageId,
      ).length,
    };
    if (
      afterDuplicate.codex_turn_id !== second.codex_turn_id
      || evidence.duplicate_replay.client_message_count !== 1
    ) throw new Error("duplicate client message started or recorded a second turn");

    const activeMessage = {
      projectId: project.id,
      clientMessageId: `live-active-${randomUUID()}`,
      text: [
        "This is a bounded interruption acceptance turn.",
        "Use the shell exactly once to run this read-only command:",
        "powershell -NoProfile -Command \"Start-Sleep -Seconds 30\"",
        "After it finishes, reply with exactly LATTICE_PRIMARY_ACTIVE_DONE.",
        "Do not inspect, create, modify, or delete files.",
      ].join("\n"),
    };
    const activeBeforeRestart = await request(origin, "/api/conversation/messages", {
      method: "POST",
      body: activeMessage,
    });
    if (activeBeforeRestart.status !== "running") {
      throw new Error(`active acceptance turn was not running (${activeBeforeRestart.status})`);
    }
    const activeIdentity = {
      conversation_id: activeBeforeRestart.id,
      thread_id: activeBeforeRestart.codex_thread_id,
      turn_id: activeBeforeRestart.codex_turn_id,
      client_message_count: activeBeforeRestart.messages.filter(
        ({ id }) => id === activeMessage.clientMessageId,
      ).length,
    };
    await stopControl(control);
    control = await startControl({ databasePath, port });
    evidence.third_control = {
      pid: control.ready.pid,
      adapter_kind: control.ready.adapterKind,
    };
    const disconnectedActive = await request(origin, "/api/conversation");
    const reconnectedActive = await request(origin, "/api/conversation/reconnect", {
      method: "POST",
      body: {},
    });
    if (
      evidence.third_control.pid === evidence.second_control.pid
      || disconnectedActive.id !== activeIdentity.conversation_id
      || disconnectedActive.codex_thread_id !== activeIdentity.thread_id
      || disconnectedActive.codex_turn_id !== activeIdentity.turn_id
      || reconnectedActive.id !== activeIdentity.conversation_id
      || reconnectedActive.codex_thread_id !== activeIdentity.thread_id
      || reconnectedActive.codex_turn_id !== activeIdentity.turn_id
      || reconnectedActive.messages.filter(({ id }) => id === activeMessage.clientMessageId).length !== 1
    ) throw new Error("active-turn restart did not retain the exact conversation binding");
    let interrupted = reconnectedActive;
    let restartOutcome = "official_terminal_after_app_server_shutdown";
    if (reconnectedActive.status === "running") {
      restartOutcome = "resumed_running_then_interrupted";
      interrupted = await request(origin, "/api/conversation/interrupt", {
        method: "POST",
        body: {},
      });
    } else if (reconnectedActive.status !== "failed") {
      throw new Error(`active turn became unexpected state ${reconnectedActive.status}`);
    }
    if (
      interrupted.status !== "failed"
      || interrupted.codex_thread_id !== activeIdentity.thread_id
      || interrupted.codex_turn_id !== activeIdentity.turn_id
      || interrupted.messages.filter(({ id }) => id === activeMessage.clientMessageId).length !== 1
    ) throw new Error("exact active turn was not interrupted without replay");
    evidence.active_restart = {
      ...activeIdentity,
      outcome: restartOutcome,
      disconnected_status: disconnectedActive.status,
      reconnected_status: reconnectedActive.status,
      interrupted_status: interrupted.status,
      duplicate_message_count_after_interrupt: interrupted.messages.filter(
        ({ id }) => id === activeMessage.clientMessageId,
      ).length,
    };

    if (restartOutcome === "resumed_running_then_interrupted") {
      evidence.direct_interrupt = {
        source: "resumed_active_turn",
        conversation_id: interrupted.id,
        thread_id: interrupted.codex_thread_id,
        turn_id: interrupted.codex_turn_id,
        terminal_status: interrupted.status,
        duplicate_message_count: interrupted.messages.filter(
          ({ id }) => id === activeMessage.clientMessageId,
        ).length,
      };
    } else {
      const interruptMessage = {
        projectId: project.id,
        clientMessageId: `live-interrupt-${randomUUID()}`,
        text: [
          "This is a bounded direct interruption acceptance turn.",
          "Use the shell exactly once to run this read-only command:",
          "powershell -NoProfile -Command \"Start-Sleep -Seconds 30\"",
          "After it finishes, reply with exactly LATTICE_PRIMARY_INTERRUPT_DONE.",
          "Do not inspect, create, modify, or delete files.",
        ].join("\n"),
      };
      const beforeInterrupt = await request(origin, "/api/conversation/messages", {
        method: "POST",
        body: interruptMessage,
      });
      if (beforeInterrupt.status !== "running") {
        throw new Error(`direct interrupt turn was not running (${beforeInterrupt.status})`);
      }
      const directlyInterrupted = await request(origin, "/api/conversation/interrupt", {
        method: "POST",
        body: {},
      });
      if (
        directlyInterrupted.status !== "failed"
        || directlyInterrupted.id !== activeIdentity.conversation_id
        || directlyInterrupted.codex_thread_id !== activeIdentity.thread_id
        || directlyInterrupted.messages.filter(
          ({ id }) => id === interruptMessage.clientMessageId,
        ).length !== 1
      ) throw new Error("direct active turn was not interrupted without replay");
      evidence.direct_interrupt = {
        source: "fresh_active_turn_after_restart_terminal",
        conversation_id: directlyInterrupted.id,
        thread_id: directlyInterrupted.codex_thread_id,
        turn_id: directlyInterrupted.codex_turn_id,
        terminal_status: directlyInterrupted.status,
        duplicate_message_count: directlyInterrupted.messages.filter(
          ({ id }) => id === interruptMessage.clientMessageId,
        ).length,
      };
    }

    evidence.status = "PASS";
    evidence.completed_at = new Date().toISOString();
  } catch (error) {
    evidence.failure = { message: boundedErrorText(error.message), code: error.code ?? null };
    evidence.completed_at = new Date().toISOString();
  } finally {
    const shutdownFailures = [];
    for (const runningControl of [parallelControl, control]) {
      try {
        await stopControl(runningControl);
      } catch (error) {
        shutdownFailures.push(boundedErrorText(error.message));
      }
    }
    if (shutdownFailures.length > 0) {
      evidence.shutdown_failure = shutdownFailures.join(" | ");
      evidence.status = "FAIL";
    }
    try {
      await rm(temporaryRoot, {
        recursive: true,
        force: true,
        maxRetries: 20,
        retryDelay: 250,
      });
    } catch (error) {
      evidence.cleanup_failure = boundedErrorText(error.message);
      evidence.status = "FAIL";
    }
    evidence.completed_at = new Date().toISOString();
    await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8");
  }
  process.stdout.write(`${JSON.stringify({ status: evidence.status, evidence_path: evidencePath })}\n`);
  if (evidence.status !== "PASS") {
    const error = new Error(evidence.failure?.message || evidence.shutdown_failure || "acceptance failed");
    error.code = evidence.failure?.code;
    throw error;
  }
  return evidencePath;
}

if (childMode) await runControlChild();
else await runAcceptance();
