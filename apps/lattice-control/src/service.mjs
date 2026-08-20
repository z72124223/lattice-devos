const approvalMethods = new Set([
  "item/commandExecution/requestApproval",
  "item/fileChange/requestApproval",
]);

function requireItem(store, id) {
  const item = store.getWorkItem(id);
  if (!item) throw new Error("work item not found");
  return item;
}

export class LatticeControlService {
  constructor({ store, codex, model = "gpt-5.6-terra" }) {
    this.store = store;
    this.codex = codex;
    this.model = model;
    this.requestOwners = new Map();
    codex.on("notification", (message) => this.#onNotification(message));
    codex.on("serverRequest", (message) => this.#onServerRequest(message));
    codex.on("disconnect", ({ code, signal }) => {
      for (const item of this.store.listWorkItems().filter((entry) => entry.status === "running")) {
        this.store.updateWorkItem(item.id, {
          status: "failed",
          failure_summary: `Codex App Server disconnected (${code ?? signal ?? "unknown"})`,
        });
      }
    });
  }

  createProject(input) {
    return this.store.createProject(input);
  }

  createWorkItem(input) {
    return this.store.createWorkItem(input);
  }

  state() {
    return {
      codexConnected: this.codex.connected,
      projects: this.store.listProjects(),
      workItems: this.store.listWorkItems(),
    };
  }

  workItem(id) {
    return {
      item: requireItem(this.store, id),
      events: this.store.listEvents(id),
    };
  }

  async start(id) {
    const item = requireItem(this.store, id);
    if (item.codex_thread_id) throw new Error("work item already has a Codex thread");
    const project = this.store.getProject(item.project_id);
    this.store.updateWorkItem(id, { status: "running", progress: "Starting Codex thread" });
    try {
      const thread = await this.codex.startThread({ cwd: project.root_path, model: this.model });
      this.store.updateWorkItem(id, {
        codex_thread_id: thread.id,
        progress: "Starting Codex turn",
      });
      this.store.appendEvent(id, "codex_thread_started", { threadId: thread.id });
      const turn = await this.codex.startTurn(thread.id, item.objective);
      const latest = requireItem(this.store, id);
      const changes = { codex_turn_id: turn?.id ?? null };
      if (latest.status === "running") changes.progress = "Codex is working";
      this.store.updateWorkItem(id, changes);
      this.store.appendEvent(id, "codex_started", { threadId: thread.id, turnId: turn?.id ?? null });
      return this.store.getWorkItem(id);
    } catch (error) {
      this.store.updateWorkItem(id, { status: "failed", failure_summary: error.message });
      throw error;
    }
  }

  async resume(id, prompt = "Continue the work from the current state.") {
    const item = requireItem(this.store, id);
    if (!item.codex_thread_id) throw new Error("work item has no Codex thread");
    if (item.status === "archived") throw new Error("archived work cannot resume");
    this.store.updateWorkItem(id, {
      status: "running",
      progress: "Resuming Codex thread",
      failure_summary: null,
    });
    try {
      await this.codex.resumeThread(item.codex_thread_id);
      const turn = await this.codex.startTurn(item.codex_thread_id, prompt);
      const latest = requireItem(this.store, id);
      const changes = { codex_turn_id: turn?.id ?? null };
      if (latest.status === "running") changes.progress = "Codex is working";
      this.store.updateWorkItem(id, changes);
      this.store.appendEvent(id, "codex_resumed", { turnId: turn?.id ?? null });
      return this.store.getWorkItem(id);
    } catch (error) {
      this.store.updateWorkItem(id, { status: "failed", failure_summary: error.message });
      throw error;
    }
  }

  approve(id, decision) {
    const item = requireItem(this.store, id);
    const approval = item.approval;
    if (!approval) throw new Error("work item has no pending approval");
    if (!new Set(["accept", "acceptForSession", "decline", "cancel"]).has(decision)) {
      throw new TypeError("invalid approval decision");
    }
    this.codex.respond(approval.requestId, { decision });
    this.requestOwners.delete(approval.requestId);
    const cancelled = decision === "cancel";
    this.store.updateWorkItem(id, {
      status: cancelled ? "failed" : "running",
      approval_json: null,
      progress: `Approval ${decision}`,
      failure_summary: cancelled ? "Approval cancelled the turn" : null,
    });
    this.store.appendEvent(id, "approval_resolved", { decision });
    return this.store.getWorkItem(id);
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

  #findByThread(threadId) {
    return this.store.listWorkItems().find((item) => item.codex_thread_id === threadId) ?? null;
  }

  #onServerRequest(message) {
    if (!approvalMethods.has(message.method)) return;
    const item = this.#findByThread(message.params?.threadId);
    if (!item) {
      this.codex.respond(message.id, { decision: "decline" });
      return;
    }
    const approval = {
      requestId: message.id,
      method: message.method,
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

  #onNotification(message) {
    const threadId = message.params?.threadId ?? message.params?.thread?.id;
    const item = threadId ? this.#findByThread(threadId) : null;
    if (!item) return;

    if (message.method === "item/started") {
      const type = message.params?.item?.type ?? "item";
      this.store.updateWorkItem(item.id, { progress: `Running ${type}` });
      this.store.appendEvent(item.id, "item_started", { type });
    } else if (message.method === "item/completed") {
      const type = message.params?.item?.type ?? "item";
      this.store.updateWorkItem(item.id, { progress: `Completed ${type}` });
      this.store.appendEvent(item.id, "item_completed", { type });
    } else if (message.method === "turn/completed") {
      const status = message.params?.turn?.status ?? "completed";
      const completed = status === "completed";
      this.store.updateWorkItem(item.id, {
        status: completed ? "codex_done" : "failed",
        progress: completed ? "Codex turn completed" : `Codex turn ${status}`,
        failure_summary: completed
          ? null
          : message.params?.turn?.error?.message ?? `Codex turn ${status}`,
      });
      this.store.appendEvent(item.id, "turn_completed", { status });
    }
  }
}
