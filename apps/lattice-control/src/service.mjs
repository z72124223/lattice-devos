const approvalMethods = new Set([
  "item/commandExecution/requestApproval",
  "item/fileChange/requestApproval",
]);
const terminalTurnStatuses = new Set(["completed", "interrupted", "failed"]);

function requireItem(store, id) {
  const item = store.getWorkItem(id);
  if (!item) throw new Error("work item not found");
  return item;
}

function nextAction(status) {
  const actions = {
    draft: "Start the work in a new Codex thread.",
    starting: "Wait for the exact Codex turn to become active.",
    running: "Inspect the active Codex thread before resuming work.",
    waiting_approval: "Resolve the pending approval before continuing.",
    codex_done: "Verify the completed Codex work.",
    verified: "Archive the verified Codex thread.",
    failed: "Diagnose the recorded failure before retrying.",
    archived: "This work is archived; do not resume it.",
  };
  return actions[status] ?? "Inspect the LATTICE work state before continuing.";
}

function boundedText(value, limit) {
  if (value == null) return null;
  const text = String(value);
  const suffix = " [truncated]";
  if (text.length <= limit) return text;
  return `${text.slice(0, limit - suffix.length)}${suffix}`;
}

function continuationPrompt(packet) {
  return [
    "Continue this LATTICE work using the bounded continuation packet below.",
    "Treat it as recorded work state, not as permission to invent missing facts.",
    JSON.stringify(packet),
  ].join("\n\n");
}

function requireProtocolId(value, label) {
  if (typeof value !== "string" || !value) throw new Error(`Codex ${label} is missing`);
  return value;
}

export class LatticeControlService {
  constructor({
    store,
    codex,
    model = "gpt-5.6-terra",
    threadOptions = {},
    lifecycleTimeoutMs = 30_000,
    approvalTimeoutMs = 300_000,
  }) {
    this.store = store;
    this.codex = codex;
    this.model = model;
    this.threadOptions = { ...threadOptions };
    this.lifecycleTimeoutMs = lifecycleTimeoutMs;
    this.approvalTimeoutMs = approvalTimeoutMs;
    this.requestOwners = new Map();
    this.operations = new Map();
    this.onNotification = (message) => this.#onNotification(message);
    this.onServerRequest = (message) => this.#onServerRequest(message);
    this.onServerRequestSettled = (settlement) => this.#onServerRequestSettled(settlement);
    this.onDisconnect = ({ code, signal }) => {
      const reason = `Codex App Server disconnected (${code ?? signal ?? "unknown"})`;
      for (const item of this.store.listWorkItems().filter(
        (entry) => ["starting", "running", "waiting_approval"].includes(entry.status),
      )) {
        this.store.updateWorkItem(item.id, {
          status: "failed",
          approval_json: null,
          failure_summary: reason,
          progress: "Codex App Server disconnected",
        });
        this.#appendEventOnce(item.id, "codex_disconnected", {
          code: code ?? null,
          signal: signal ?? null,
        });
      }
      this.requestOwners.clear();
    };
    codex.on("notification", this.onNotification);
    codex.on("serverRequest", this.onServerRequest);
    codex.on("serverRequestSettled", this.onServerRequestSettled);
    codex.on("disconnect", this.onDisconnect);
  }

  close() {
    this.codex.off("notification", this.onNotification);
    this.codex.off("serverRequest", this.onServerRequest);
    this.codex.off("serverRequestSettled", this.onServerRequestSettled);
    this.codex.off("disconnect", this.onDisconnect);
    this.requestOwners.clear();
    this.operations.clear();
  }

  createProject(input) {
    return this.store.createProject(input);
  }

  createWorkItem(input) {
    return this.store.createWorkItem(input);
  }

  recordInstallationReceipt(input) {
    return this.store.createInstallationReceipt(input);
  }

  installationReceipts(options) {
    return this.store.listInstallationReceipts(options);
  }

  installationReceipt(id) {
    return this.store.getInstallationReceipt(id);
  }

  state() {
    return {
      codexConnected: this.codex.connected,
      projects: this.store.listProjects(),
      workItems: this.store.listWorkItems(),
      installationReceiptCount: this.store.countInstallationReceipts(),
    };
  }

  workItem(id) {
    return {
      item: requireItem(this.store, id),
      events: this.store.listEvents(id),
    };
  }

  continuation(id) {
    const item = requireItem(this.store, id);
    const project = this.store.getProject(item.project_id);
    const events = this.store.listEvents(id);
    return {
      schema_version: "lattice.control.continuation.v1",
      project: {
        name: boundedText(project.name, 256),
        root_path: boundedText(project.root_path, 1_024),
      },
      work: {
        id: item.id,
        title: boundedText(item.title, 256),
        objective: boundedText(item.objective, 2_048),
        priority: item.priority,
        status: item.status,
        codex_thread_id: item.codex_thread_id ?? null,
      },
      current: {
        progress: boundedText(item.progress, 512),
        failure_summary: boundedText(item.failure_summary, 2_048),
        verification_notes: boundedText(item.verification_notes, 2_048),
        next_action: nextAction(item.status),
      },
      evidence: { latest_event: boundedText(events.at(-1)?.kind, 128) },
    };
  }

  start(id) {
    return this.#runExclusive(id, "start", () => this.#start(id));
  }

  async #start(id) {
    const item = requireItem(this.store, id);
    if (item.codex_thread_id) throw new Error("work item already has a Codex thread");
    const project = this.store.getProject(item.project_id);
    const packet = this.continuation(id);
    const claimed = this.store.transitionWorkItem(id, ["draft"], {
      status: "starting",
      progress: "Waiting for Codex thread acceptance",
      failure_summary: null,
    });
    if (!claimed) throw new Error("work item is already starting or has already started");

    try {
      const thread = await this.codex.startThread({
        ...this.threadOptions,
        cwd: project.root_path,
        model: this.model,
      });
      const threadId = requireProtocolId(thread?.id, "thread ID");
      const threadStarted = this.codex.waitForThreadStarted(threadId, {
        timeoutMs: this.lifecycleTimeoutMs,
      });
      threadStarted.catch(() => {});
      if (!this.store.transitionWorkItem(id, ["starting"], {
        codex_thread_id: threadId,
        progress: "Codex thread RPC accepted; waiting for thread/started",
      })) {
        throw new Error("work item left starting state before its Codex thread was saved");
      }
      this.store.appendEvent(id, "codex_thread_accepted", { threadId });

      await threadStarted;
      this.#appendEventOnce(id, "codex_thread_started", { threadId });
      this.#replayMcpDiagnostics(id, threadId);

      const turn = await this.codex.startTurn(threadId, continuationPrompt(packet));
      const turnId = requireProtocolId(turn?.id, "turn ID");
      const turnStarted = this.codex.waitForTurnStarted(threadId, turnId, {
        timeoutMs: this.lifecycleTimeoutMs,
      });
      turnStarted.catch(() => {});
      if (!this.store.transitionWorkItem(id, ["starting", "waiting_approval"], {
        codex_turn_id: turnId,
        progress: "Codex turn RPC accepted; waiting for turn/started",
      })) {
        throw new Error("work item left its start state before its Codex turn was saved");
      }
      this.store.appendEvent(id, "codex_turn_accepted", { threadId, turnId });

      const activeTurn = await turnStarted;
      this.#markTurnStarted(id, threadId, activeTurn);
      this.#replayTerminal(id, threadId, turnId);
      return this.store.getWorkItem(id);
    } catch (error) {
      this.#failLifecycle(id, error);
      throw error;
    }
  }

  resume(id, prompt = "Continue the work from the current state.") {
    return this.#runExclusive(id, "resume", () => this.#resume(id, prompt));
  }

  async #resume(id, prompt) {
    const { turn } = await this.#reconcileThread(id);
    if (turn.status === "completed") return this.store.getWorkItem(id);
    if (!["interrupted", "failed"].includes(turn.status)) {
      throw new Error(`Codex turn ${turn.id} is not in a retryable terminal state`);
    }
    const retryClaims = this.store.listEvents(id)
      .filter(({ kind }) => kind === "codex_retry_claimed");
    if (retryClaims.length >= 1) throw new Error("the bounded Codex retry was already used");

    const item = requireItem(this.store, id);
    if (!this.store.transitionWorkItem(id, ["failed"], {
      status: "starting",
      approval_json: null,
      progress: "Retrying reconciled Codex thread",
      failure_summary: null,
    })) {
      throw new Error("work item is not in a retryable failed state");
    }
    this.store.appendEvent(id, "codex_retry_claimed", {
      threadId: item.codex_thread_id,
      previousTurnId: item.codex_turn_id,
    });

    try {
      const retryPrompt = typeof prompt === "string" && prompt.trim()
        ? boundedText(prompt.trim(), 8_192)
        : "Continue the work from the current state.";
      const retryTurn = await this.codex.startTurn(item.codex_thread_id, retryPrompt);
      const turnId = requireProtocolId(retryTurn?.id, "retry turn ID");
      const turnStarted = this.codex.waitForTurnStarted(item.codex_thread_id, turnId, {
        timeoutMs: this.lifecycleTimeoutMs,
      });
      turnStarted.catch(() => {});
      if (!this.store.transitionWorkItem(id, ["starting"], {
        codex_turn_id: turnId,
        progress: "Codex retry RPC accepted; waiting for turn/started",
      })) {
        throw new Error("work item left starting state before its retry turn was saved");
      }
      this.store.appendEvent(id, "codex_retry_accepted", {
        threadId: item.codex_thread_id,
        turnId,
      });
      const activeTurn = await turnStarted;
      this.#markTurnStarted(id, item.codex_thread_id, activeTurn);
      this.#appendEventOnce(id, "codex_retry_started", {
        threadId: item.codex_thread_id,
        turnId,
      });
      this.#replayTerminal(id, item.codex_thread_id, turnId);
      return this.store.getWorkItem(id);
    } catch (error) {
      this.#failLifecycle(id, error);
      throw error;
    }
  }

  reconcile(id) {
    return this.#runExclusive(id, "reconcile", async () => {
      await this.#reconcileThread(id);
      return this.store.getWorkItem(id);
    });
  }

  async #reconcileThread(id) {
    const item = requireItem(this.store, id);
    const threadId = requireProtocolId(item.codex_thread_id, "saved thread ID");
    const turnId = requireProtocolId(item.codex_turn_id, "saved turn ID");
    const thread = await this.codex.resumeThread(threadId);
    if (thread?.id !== threadId || !Array.isArray(thread.turns) || thread.turns.length === 0) {
      throw new Error(`Codex thread ${threadId} reconciliation returned an empty rollout`);
    }
    const turn = thread.turns.find((candidate) => candidate.id === turnId);
    const latest = thread.turns.at(-1);
    if (!turn || latest?.id !== turnId || !terminalTurnStatuses.has(turn.status)) {
      throw new Error(`Codex thread ${threadId} does not reconcile to saved terminal turn ${turnId}`);
    }
    if (!this.#hasConfirmedStart(id, threadId, turnId)) {
      throw new Error(`Codex turn ${threadId}/${turnId} has no confirmed turn/started evidence`);
    }
    this.#applyTerminal(id, {
      method: "turn/completed",
      params: { threadId, turn },
    });
    this.#appendEventOnce(id, "codex_reconciled", {
      threadId,
      turnId,
      status: turn.status,
    });
    this.#replayMcpDiagnostics(id, threadId);
    return { thread, turn };
  }

  interrupt(id) {
    return this.#runExclusive(id, "interrupt", async () => {
      const item = requireItem(this.store, id);
      if (item.status !== "running") throw new Error("work item has no active Codex turn");
      const threadId = requireProtocolId(item.codex_thread_id, "active thread ID");
      const turnId = requireProtocolId(item.codex_turn_id, "active turn ID");
      if (!this.codex.isTurnActive(threadId, turnId)) {
        throw new Error(`Codex turn ${threadId}/${turnId} is not confirmed active`);
      }
      try {
        const terminal = await this.codex.interruptTurn(threadId, turnId, {
          timeoutMs: this.lifecycleTimeoutMs,
        });
        this.#applyTerminal(id, {
          method: "turn/completed",
          params: { threadId, turn: terminal },
        });
        return this.store.getWorkItem(id);
      } catch (error) {
        this.#failLifecycle(id, error);
        throw error;
      }
    });
  }

  async approve(id, decision) {
    const item = requireItem(this.store, id);
    const approval = item.approval;
    if (!approval) throw new Error("work item has no pending approval");
    if (!new Set(["accept", "acceptForSession", "decline", "cancel"]).has(decision)) {
      throw new TypeError("invalid approval decision");
    }
    const cancelled = decision === "cancel";
    const active = this.#hasConfirmedStart(id, item.codex_thread_id, item.codex_turn_id);
    const controller = cancelled ? new AbortController() : null;
    const terminal = cancelled
      ? this.codex.waitForTurnCompleted(item.codex_thread_id, item.codex_turn_id, {
          timeoutMs: this.lifecycleTimeoutMs,
          statuses: ["interrupted", "failed"],
          signal: controller.signal,
        })
      : null;
    terminal?.catch(() => {});
    try {
      this.codex.respond(approval.requestId, { decision });
    } catch (error) {
      controller?.abort();
      throw error;
    }
    this.requestOwners.delete(approval.requestId);
    this.store.updateWorkItem(id, {
      status: active ? "running" : "starting",
      approval_json: null,
      progress: cancelled
        ? "Cancellation requested; waiting for Codex turn terminal"
        : `Approval ${decision}`,
      failure_summary: null,
    });
    this.store.appendEvent(id, "approval_resolved", { decision, terminalPending: cancelled });
    if (!cancelled) return this.store.getWorkItem(id);
    try {
      const completed = await terminal;
      this.#applyTerminal(id, {
        method: "turn/completed",
        params: { threadId: item.codex_thread_id, turn: completed },
      });
      return this.store.getWorkItem(id);
    } catch (error) {
      await this.codex.close();
      this.#failLifecycle(id, error);
      throw error;
    }
  }

  verify(id, notes) {
    const item = requireItem(this.store, id);
    if (item.status !== "codex_done") throw new Error("Codex work is not ready for verification");
    const updated = this.store.updateWorkItem(id, {
      status: "verified",
      verification_notes: typeof notes === "string" ? notes.trim() : "",
      progress: "Verified",
    });
    this.store.appendEvent(id, "verified", { notes: updated.verification_notes });
    return updated;
  }

  async archive(id) {
    const item = requireItem(this.store, id);
    if (item.status !== "verified") throw new Error("only verified work can be archived");
    await this.codex.archiveThread(item.codex_thread_id);
    const updated = this.store.updateWorkItem(id, {
      status: "archived",
      archived_at: new Date().toISOString(),
      progress: "Archived",
    });
    this.store.appendEvent(id, "archived", { threadId: item.codex_thread_id });
    return updated;
  }

  #runExclusive(id, kind, operation) {
    const existing = this.operations.get(id);
    if (existing) {
      if (existing.kind === kind) return existing.promise;
      throw new Error(`work item ${existing.kind} operation is already in progress`);
    }
    const promise = Promise.resolve().then(operation);
    const record = { kind, promise };
    this.operations.set(id, record);
    const cleanup = () => {
      if (this.operations.get(id) === record) this.operations.delete(id);
    };
    promise.then(cleanup, cleanup);
    return promise;
  }

  #findByThread(threadId) {
    return this.store.listWorkItems().find((item) => item.codex_thread_id === threadId) ?? null;
  }

  #appendEventOnce(id, kind, payload) {
    const encoded = JSON.stringify(payload);
    const exists = this.store.listEvents(id)
      .some((event) => event.kind === kind && JSON.stringify(event.payload) === encoded);
    if (!exists) this.store.appendEvent(id, kind, payload);
  }

  #hasConfirmedStart(id, threadId, turnId) {
    return this.store.listEvents(id).some(({ kind, payload }) => (
      kind === "codex_started"
      && payload.threadId === threadId
      && payload.turnId === turnId
      && payload.confirmedBy === "turn/started"
    ));
  }

  #markTurnStarted(id, threadId, turn) {
    const turnId = turn?.id;
    const item = this.store.getWorkItem(id);
    if (
      !item
      || item.codex_thread_id !== threadId
      || item.codex_turn_id !== turnId
      || turn?.status !== "inProgress"
    ) return false;
    if (item.status === "starting") {
      this.store.transitionWorkItem(id, ["starting"], {
        status: "running",
        progress: "Codex turn is active",
        failure_summary: null,
      });
    } else if (!["running", "waiting_approval"].includes(item.status)) {
      return false;
    }
    this.#appendEventOnce(id, "codex_started", {
      threadId,
      turnId,
      status: "inProgress",
      confirmedBy: "turn/started",
    });
    return true;
  }

  #applyTerminal(id, message) {
    const threadId = message.params?.threadId;
    const turn = message.params?.turn;
    const turnId = turn?.id;
    const status = turn?.status;
    const item = this.store.getWorkItem(id);
    if (
      !item
      || item.codex_thread_id !== threadId
      || item.codex_turn_id !== turnId
      || !terminalTurnStatuses.has(status)
      || !this.#hasConfirmedStart(id, threadId, turnId)
    ) return false;

    const existingTerminal = this.store.listEvents(id).find(({ kind, payload }) => (
      kind === "turn_completed"
      && payload.threadId === threadId
      && payload.turnId === turnId
    ));
    if (existingTerminal) {
      if (existingTerminal.payload.status !== status) {
        this.#appendEventOnce(id, "turn_terminal_conflict_ignored", {
          threadId,
          turnId,
          authoritativeStatus: existingTerminal.payload.status,
          ignoredStatus: status,
        });
      }
      return existingTerminal.payload.status === status;
    }

    const completed = status === "completed";
    const targetStatus = completed ? "codex_done" : "failed";
    if (!["verified", "archived"].includes(item.status) && item.status !== targetStatus) {
      this.store.updateWorkItem(id, {
        status: targetStatus,
        approval_json: null,
        progress: completed ? "Codex turn completed" : `Codex turn ${status}`,
        failure_summary: completed
          ? null
          : turn.error?.message ?? `Codex turn ${status}`,
      });
    }
    this.#appendEventOnce(id, "turn_completed", {
      threadId,
      turnId,
      status,
      error: turn.error ?? null,
    });
    if (item.approval?.requestId != null) {
      try {
        this.codex.respond(item.approval.requestId, { decision: "cancel" });
      } catch {
        // The App Server may have already closed the request with the turn terminal.
      }
      this.requestOwners.delete(item.approval.requestId);
    }
    return true;
  }

  #replayTerminal(id, threadId, turnId) {
    const entries = this.codex.notificationSnapshot?.({
      method: "turn/completed",
      threadId,
      turnId,
    }) ?? [];
    const terminal = entries.at(-1)?.message;
    if (terminal) this.#applyTerminal(id, terminal);
  }

  #mcpDiagnosticPayload(params = {}) {
    return {
      threadId: params.threadId ?? null,
      name: params.name ?? null,
      status: params.status ?? null,
      error: params.error ?? null,
      failureReason: params.failureReason ?? null,
    };
  }

  #persistMcpDiagnostic(id, params) {
    this.#appendEventOnce(
      id,
      "mcp_server_startup_status_updated",
      this.#mcpDiagnosticPayload(params),
    );
  }

  #replayMcpDiagnostics(id, threadId) {
    const entries = this.codex.notificationSnapshot?.({
      method: "mcpServer/startupStatus/updated",
      threadId,
    }) ?? [];
    for (const entry of entries) this.#persistMcpDiagnostic(id, entry.message.params);
  }

  #failLifecycle(id, error) {
    const item = this.store.getWorkItem(id);
    if (!item || !["starting", "running", "waiting_approval"].includes(item.status)) return;
    this.store.updateWorkItem(id, {
      status: "failed",
      approval_json: null,
      progress: "Codex lifecycle failed",
      failure_summary: boundedText(error?.message ?? error, 2_048),
    });
    this.store.appendEvent(id, "codex_lifecycle_failed", {
      message: boundedText(error?.message ?? error, 2_048),
      code: error?.code ?? null,
    });
  }

  #onServerRequest(message) {
    if (!approvalMethods.has(message.method)) {
      this.codex.rejectServerRequest?.(message.id, {
        code: -32601,
        message: `Unsupported Codex App Server request: ${message.method}`,
      });
      return;
    }
    const threadId = message.params?.threadId;
    const turnId = message.params?.turnId;
    const item = this.#findByThread(threadId);
    const exactActiveTurn = Boolean(
      item
      && turnId
      && item.codex_turn_id === turnId
      && this.codex.isTurnActive?.(threadId, turnId),
    );
    if (!exactActiveTurn || item.approval) {
      this.codex.respond(message.id, { decision: "decline" });
      return;
    }
    this.codex.deferServerRequest(message.id, { timeoutMs: this.approvalTimeoutMs });
    const approval = {
      requestId: message.id,
      method: message.method,
      threadId,
      turnId,
      reason: message.params?.reason ?? null,
      command: message.params?.command ?? null,
      cwd: message.params?.cwd ?? null,
    };
    this.requestOwners.set(message.id, item.id);
    this.store.updateWorkItem(item.id, {
      status: "waiting_approval",
      approval_json: JSON.stringify(approval),
      progress: approval.reason || "Waiting for approval",
    });
    this.store.appendEvent(item.id, "approval_requested", approval);
  }

  #onServerRequestSettled({ id, response }) {
    const owner = this.requestOwners.get(id);
    if (!owner || !response?.error) return;
    const item = this.store.getWorkItem(owner);
    if (item?.approval?.requestId === id) {
      this.store.updateWorkItem(owner, {
        status: "failed",
        approval_json: null,
        progress: "Codex approval request failed",
        failure_summary: response.error.message,
      });
      this.store.appendEvent(owner, "approval_request_failed", {
        requestId: id,
        code: response.error.code ?? null,
        message: response.error.message,
      });
    }
    this.requestOwners.delete(id);
  }

  #onNotification(message) {
    const threadId = message.params?.threadId ?? message.params?.thread?.id;
    const item = threadId ? this.#findByThread(threadId) : null;
    if (!item) return;

    if (message.method === "mcpServer/startupStatus/updated") {
      this.#persistMcpDiagnostic(item.id, message.params);
      return;
    }
    if (message.method === "turn/started") {
      this.#markTurnStarted(item.id, threadId, message.params?.turn);
      return;
    }
    if (message.method === "turn/completed") {
      this.#applyTerminal(item.id, message);
      return;
    }

    const eventTurnId = message.params?.turnId ?? null;
    if (["item/started", "item/completed"].includes(message.method)
      && eventTurnId !== item.codex_turn_id) return;
    if (message.method === "item/started") {
      const type = message.params?.item?.type ?? "item";
      this.store.updateWorkItem(item.id, { progress: `Running ${type}` });
      this.store.appendEvent(item.id, "item_started", { type });
    } else if (message.method === "item/completed") {
      const type = message.params?.item?.type ?? "item";
      this.store.updateWorkItem(item.id, { progress: `Completed ${type}` });
      this.store.appendEvent(item.id, "item_completed", { type });
    }
  }
}
