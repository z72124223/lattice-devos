import { createHash, randomUUID } from "node:crypto";
import process from "node:process";
import {
  canonicalizeProjectPath,
  inspectProject,
  ProjectInspectionError,
} from "./project-inspector.mjs";
import { normalizeProjectDisplayName } from "./store.mjs";

const approvalMethods = new Set([
  "item/commandExecution/requestApproval",
  "item/fileChange/requestApproval",
]);
const terminalTurnStatuses = new Set(["completed", "interrupted", "failed"]);
const primaryConversationId = "primary";
const primaryConversationLeaseTtlMs = 15_000;
const primaryConversationDiscoveryPasses = 3;

function conversationError(code, message, status = 409) {
  const error = new Error(message);
  error.code = code;
  error.status = status;
  return error;
}

function requireLegacyWorkItemId(id) {
  if (id === primaryConversationId) {
    throw conversationError(
      "PRIMARY_CONVERSATION_ROUTE_REQUIRED",
      "the primary conversation is only available through conversation routes",
    );
  }
  return id;
}

function conversationOperationKey({ projectId, clientMessageId, text }) {
  const digest = createHash("sha256")
    .update(JSON.stringify([projectId, clientMessageId, text]), "utf8")
    .digest("hex");
  return `conversation-message:${clientMessageId}:${digest}`;
}

function turnStartsWithConversationMarker(turn, marker) {
  return Array.isArray(turn?.items) && turn.items.some((item) => item?.type === "userMessage"
    && Array.isArray(item.content)
    && item.content.some((content) => content?.type === "text"
      && typeof content.text === "string"
      && (content.text === marker || content.text.startsWith(`${marker}\n`))));
}

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

function conversationMarker({ clientMessageId, promptDigest }) {
  return `[LATTICE_CONTROL_MESSAGE id=${clientMessageId} digest=${promptDigest}]`;
}

function conversationTurnPrompt(claim, previousMessages, handoffReason = null) {
  const marker = conversationMarker(claim);
  if (!handoffReason) return `${marker}\n${claim.text}`;
  const transcript = previousMessages.slice(-12).map(({ role, text }) => ({
    role,
    text: boundedText(text, 2_048),
  }));
  return [
    marker,
    "LATTICE Control is keeping one user-visible conversation while changing its Codex backing thread.",
    "Use this bounded transcript as conversation context; do not treat metadata as a new authority.",
    JSON.stringify({
      schema_version: "lattice.control.conversation-handoff.v1",
      reason: handoffReason,
      previous_messages: transcript,
    }),
    "Current user message:",
    claim.text,
  ].join("\n\n");
}

function conversationStatusText(item, codexConnected = false) {
  if (!item) return "選擇專案後即可開始對話。";
  if (
    !codexConnected
    && ["starting", "running", "waiting_approval"].includes(item.status)
  ) {
    return "Control 已恢復，但 Codex 尚未重新連線；同一則訊息不會重送。請按「重新連線」。";
  }
  if (item.status === "starting") return "訊息已保存，正在連接 Codex…";
  if (item.status === "running") return "Codex 正在回覆…";
  if (item.status === "waiting_approval") return "Codex 正等待你的核准。";
  if (item.status === "codex_done") return "已收到 Codex 回覆，可以繼續對話。";
  if (item.status === "failed") {
    if (/disconnect|連線中斷/iu.test(item.failure_summary ?? "")) {
      return "連線中斷；請重新連線。同一則訊息不會重複送出。";
    }
    return "本次回覆未完成；訊息已保存，不會自動重送。請重新連線檢查狀態。";
  }
  if (item.status === "draft") return "對話已準備好。";
  return "這條對話目前無法送出新訊息。";
}

export class LatticeControlService {
  constructor({
    store,
    codex,
    model = "gpt-5.6-terra",
    threadOptions = {},
    lifecycleTimeoutMs = 30_000,
    approvalTimeoutMs = 300_000,
    projectInspector = inspectProject,
    conversationLeaseTtlMs = primaryConversationLeaseTtlMs,
  }) {
    this.store = store;
    this.codex = codex;
    this.model = model;
    this.threadOptions = { ...threadOptions };
    this.lifecycleTimeoutMs = lifecycleTimeoutMs;
    this.approvalTimeoutMs = approvalTimeoutMs;
    this.projectInspector = projectInspector;
    this.conversationLeaseTtlMs = conversationLeaseTtlMs;
    this.conversationOwnerId = `control:${randomUUID()}`;
    this.conversationLeaseTimer = null;
    this.conversationLeaseFence = null;
    this.conversationLeaseLossPromise = null;
    this.requestOwners = new Map();
    this.operations = new Map();
    this.onNotification = (message) => this.#onNotification(message);
    this.onServerRequest = (message) => this.#onServerRequest(message);
    this.onServerRequestSettled = (settlement) => this.#onServerRequestSettled(settlement);
    this.onDisconnect = ({ code, signal }) => {
      const reason = `Codex App Server disconnected (${code ?? signal ?? "unknown"})`;
      try {
        for (const item of this.store.listWorkItems().filter(
          (entry) => ["starting", "running", "waiting_approval"].includes(entry.status),
        )) {
          const primaryConversation = item.id === primaryConversationId;
          const fence = primaryConversation ? this.conversationLeaseFence : null;
          if (primaryConversation && !this.#ownsPrimaryConversationLease(fence)) continue;
          try {
            this.store.updateWorkItem(item.id, {
              status: "failed",
              approval_json: null,
              failure_summary: primaryConversation ? "Codex App Server 連線中斷" : reason,
              progress: primaryConversation
                ? "連線中斷；可安全重新連線，訊息不會重送"
                : "Codex App Server disconnected",
            }, fence);
            this.#appendEventOnce(item.id, "codex_disconnected", {
              code: code ?? null,
              signal: signal ?? null,
            }, fence);
          } catch (error) {
            if (primaryConversation && error?.code === "CONVERSATION_WRITER_LOST") {
              void this.#handlePrimaryConversationLeaseLoss(fence);
              continue;
            }
            throw error;
          }
        }
      } finally {
        this.requestOwners.clear();
      }
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
    if (this.conversationLeaseTimer) clearInterval(this.conversationLeaseTimer);
    this.conversationLeaseTimer = null;
    const fence = this.conversationLeaseFence;
    this.conversationLeaseFence = null;
    if (fence) {
      try {
        this.store.releasePrimaryConversationLease(fence);
      } catch {
        // Store shutdown may already be in progress.
      }
    }
  }

  createProject(input) {
    // Compatibility-only harness path. The HTTP project route uses registerProject
    // and exposes legacy rows as LEGACY_CONTROL_PROJECT until they are adopted.
    return this.store.createProject(input);
  }

  async registerProject({ name, rootPath }) {
    const normalizedName = normalizeProjectDisplayName(name);
    const canonicalPath = await canonicalizeProjectPath(rootPath);
    const registrationClaim = this.store.beginProjectRegistration(canonicalPath);
    const inspection = await this.projectInspector(canonicalPath);
    return this.store.registerProject({
      name: normalizedName,
      inspection,
      registrationGeneration: registrationClaim.registration_generation,
      projectRefreshGeneration: registrationClaim.project_refresh_generation,
      claimedCanonicalPath: canonicalPath,
    });
  }

  project(id) {
    const project = this.store.getProjectRegistration(id);
    if (!project) throw new Error("registered project not found");
    return project;
  }

  async refreshProject(id) {
    const project = this.project(id);
    const attemptStartedAt = new Date().toISOString();
    const attemptGeneration = this.store.beginProjectRefresh(project.id);
    try {
      const inspection = await this.projectInspector(project.canonical_path);
      return this.store.refreshProject({
        projectId: project.id,
        inspection,
        attemptGeneration,
      });
    } catch (error) {
      if (error instanceof ProjectInspectionError) {
        this.store.recordProjectRefreshFailure({
          projectId: project.id,
          code: error.code,
          message: error.message,
          observedAt: attemptStartedAt,
          attemptGeneration,
        });
      }
      throw error;
    }
  }

  createWorkItem(input) {
    return this.store.createWorkItem(input);
  }

  primaryConversation() {
    const item = this.store.getWorkItem(primaryConversationId);
    if (!item) {
      return {
        schema_version: "lattice.control.primary-conversation.v1",
        id: primaryConversationId,
        project_id: null,
        codex_thread_id: null,
        codex_turn_id: null,
        status: "not_started",
        status_text: conversationStatusText(null, this.codex.connected),
        codex_connected: Boolean(this.codex.connected),
        can_send: false,
        can_interrupt: false,
        can_reconnect: false,
        messages: [],
        handoffs: [],
        last_error: null,
      };
    }
    const events = this.store.listEvents(primaryConversationId);
    const acceptedByMessage = new Map(events
      .filter(({ kind }) => kind === "conversation_message_accepted")
      .map((event) => [event.payload.clientMessageId, event]));
    const failedMessages = new Set(events
      .filter(({ kind }) => kind === "conversation_message_failed")
      .map(({ payload }) => payload.clientMessageId));
    const unresolvedMessage = events.findLast(({ kind, payload }) => (
      kind === "conversation_message_claimed"
      && !acceptedByMessage.has(payload.clientMessageId)
    )) ?? null;
    const missingCurrentFinal = events.some(({ kind, payload }) => (
      kind === "conversation_terminal_missing_reply"
      && payload.threadId === item.codex_thread_id
      && payload.turnId === item.codex_turn_id
    )) && !events.some(({ kind, payload }) => (
      kind === "conversation_assistant_message"
      && payload.threadId === item.codex_thread_id
      && payload.turnId === item.codex_turn_id
    ));
    const messages = [];
    for (const event of events) {
      if (event.kind === "conversation_message_claimed") {
        const accepted = acceptedByMessage.get(event.payload.clientMessageId);
        messages.push({
          event_id: event.id,
          id: event.payload.clientMessageId,
          role: "user",
          text: event.payload.text,
          delivery_status: accepted
            ? "accepted"
            : failedMessages.has(event.payload.clientMessageId)
              ? "failed"
              : "saved",
          created_at: event.created_at,
          turn_id: accepted?.payload.turnId ?? null,
        });
      } else if (event.kind === "conversation_assistant_message") {
        messages.push({
          event_id: event.id,
          id: event.payload.messageId,
          role: "assistant",
          text: event.payload.text,
          created_at: event.created_at,
          turn_id: event.payload.turnId,
        });
      }
    }
    messages.sort((left, right) => left.event_id - right.event_id);
    for (const message of messages) delete message.event_id;
    const handoffs = events
      .filter(({ kind }) => kind === "conversation_thread_handoff")
      .map((event) => ({
        from_thread_id: event.payload.fromThreadId,
        to_thread_id: event.payload.toThreadId,
        reason: event.payload.reason,
        created_at: event.created_at,
      }));
    return {
      schema_version: "lattice.control.primary-conversation.v1",
      id: item.id,
      project_id: item.project_id,
      codex_thread_id: item.codex_thread_id ?? null,
      codex_turn_id: item.codex_turn_id ?? null,
      status: item.status,
      status_text: conversationStatusText(item, this.codex.connected),
      codex_connected: Boolean(this.codex.connected),
      can_send: ["draft", "codex_done", "failed"].includes(item.status)
        && unresolvedMessage === null
        && !missingCurrentFinal,
      can_interrupt: ["running", "waiting_approval"].includes(item.status) && Boolean(
        this.codex.isTurnActive?.(item.codex_thread_id, item.codex_turn_id),
      ),
      can_reconnect: Boolean(
        ["starting", "running", "waiting_approval", "failed"].includes(item.status)
        && ((item.codex_thread_id && item.codex_turn_id) || unresolvedMessage)
      ),
      messages,
      handoffs,
      last_error: item.failure_summary ?? null,
    };
  }

  async sendPrimaryConversationMessage({ projectId, clientMessageId, text }) {
    this.store.ensurePrimaryConversation(projectId);
    return this.#runExclusive(
      primaryConversationId,
      conversationOperationKey({ projectId, clientMessageId, text }),
      async () => {
        const fence = this.#acquirePrimaryConversationLease();
        let claim = null;
        try {
          const existing = this.store.primaryConversationMessage(clientMessageId);
          if (existing) {
            claim = this.store.claimPrimaryConversationMessage({
              projectId,
              clientMessageId,
              text,
              fence,
            });
            const effectIdentity = await this.#conversationEffectIdentity(fence);
            return await this.#dispatchPrimaryConversationClaim(claim, {
              effectIdentity,
              fresh: false,
              fence,
            });
          }

          const effectIdentity = await this.#conversationEffectIdentity(fence);
          await this.#reconcilePrimaryConversationBeforeClaim(projectId, effectIdentity, fence);
          claim = this.store.claimPrimaryConversationMessage({
            projectId,
            clientMessageId,
            text,
            fence,
          });
          return await this.#dispatchPrimaryConversationClaim(claim, {
            effectIdentity,
            fresh: claim.claimed,
            fence,
          });
        } catch (error) {
          if (claim && this.#ownsPrimaryConversationLease(fence)) {
            this.#failLifecycle(primaryConversationId, error, fence);
            this.#appendEventOnce(primaryConversationId, "conversation_message_failed", {
              clientMessageId: claim.event.payload.clientMessageId,
              message: boundedText(error?.message ?? error, 2_048),
              code: error?.code ?? null,
            }, fence);
          }
          throw error;
        }
      },
    );
  }

  async #conversationEffectIdentity(fence) {
    if (typeof this.codex.readAuthReadiness !== "function") return null;
    const readiness = await this.#fencedConversationEffect(
      fence,
      () => this.codex.readAuthReadiness(),
    );
    if (readiness?.ready !== true || readiness.authMode !== "chatgpt") {
      throw conversationError(
        "CONVERSATION_CODEX_AUTH_REQUIRED",
        "Codex ChatGPT account readiness could not be verified",
        503,
      );
    }
    return {
      expectedGeneration: readiness.appServerGeneration,
      expectedSessionId: readiness.appServerSessionId,
    };
  }

  #conversationBinding() {
    return this.store.listEvents(primaryConversationId)
      .filter(({ kind }) => kind === "conversation_thread_bound")
      .at(-1) ?? null;
  }

  #conversationAcceptedEvent(clientMessageId) {
    return this.store.listEvents(primaryConversationId).find(({ kind, payload }) => (
      kind === "conversation_message_accepted"
      && payload.clientMessageId === clientMessageId
    )) ?? null;
  }

  #conversationTerminalEvent(threadId, turnId) {
    return this.store.listEvents(primaryConversationId).find(({ kind, payload }) => (
      kind === "turn_completed"
      && payload.threadId === threadId
      && payload.turnId === turnId
      && terminalTurnStatuses.has(payload.status)
    )) ?? null;
  }

  async #reconcilePrimaryConversationBeforeClaim(projectId, effectIdentity, fence) {
    const before = requireItem(this.store, primaryConversationId);
    if (!before.codex_thread_id && !before.codex_turn_id) return;
    if (!before.codex_thread_id || !before.codex_turn_id) {
      throw conversationError(
        "CONVERSATION_RECONCILIATION_REQUIRED",
        "saved conversation binding is incomplete; reconnect the saved message first",
      );
    }
    try {
      const resumed = await this.#fencedConversationEffect(
        fence,
        () => this.codex.resumeThread(before.codex_thread_id, {
          expectedTurnId: before.codex_turn_id,
          effectIdentity,
        }),
      );
      const latest = resumed?.turns?.at(-1);
      if (resumed?.id !== before.codex_thread_id || latest?.id !== before.codex_turn_id) {
        throw new Error("Codex returned a different saved conversation binding");
      }
      this.#persistConversationRepliesFromTurn(
        primaryConversationId,
        before.codex_thread_id,
        latest,
        fence,
      );
      if (latest.status === "inProgress") {
        this.store.updateWorkItem(primaryConversationId, {
          status: "running",
          progress: "已重新連線；Codex 正在回覆",
          failure_summary: null,
        }, fence);
        throw conversationError("CONVERSATION_BUSY", "Codex is still replying to the previous message");
      }
      if (!terminalTurnStatuses.has(latest.status)) {
        throw new Error("saved Codex conversation has an unknown turn state");
      }
      if (!this.#applyTerminal(primaryConversationId, {
        method: "turn/completed",
        params: { threadId: before.codex_thread_id, turn: latest },
      }, fence)) {
        throw conversationError(
          "CONVERSATION_RECONCILIATION_REQUIRED",
          "Codex completed the saved turn without a recoverable final reply",
        );
      }
    } catch (error) {
      if (error?.code === "CODEX_THREAD_NOT_RECOVERABLE") {
        throw conversationError(
          "CONVERSATION_RECONCILIATION_REQUIRED",
          "the saved Codex conversation is not recoverable; no replacement was started",
        );
      }
      throw error;
    }
  }

  async #dispatchPrimaryConversationClaim(claim, { effectIdentity, fresh, fence }) {
    const accepted = this.#conversationAcceptedEvent(claim.event.payload.clientMessageId);
    if (accepted) {
      const item = requireItem(this.store, primaryConversationId);
      if (
        item.codex_thread_id !== accepted.payload.threadId
        || item.codex_turn_id !== accepted.payload.turnId
        || this.#conversationTerminalEvent(accepted.payload.threadId, accepted.payload.turnId)
        || this.codex.isTurnActive?.(accepted.payload.threadId, accepted.payload.turnId)
      ) {
        return this.primaryConversation();
      }
      return this.#reconcileAcceptedConversationClaim(claim, accepted, effectIdentity, fence);
    }
    if (!fresh) {
      return this.#recoverUnacceptedConversationClaim(claim, effectIdentity, fence);
    }

    const binding = this.#conversationBinding();
    const item = requireItem(this.store, primaryConversationId);
    const projectChanged = Boolean(binding && binding.payload.projectId !== claim.event.payload.projectId);
    let threadId = item.codex_thread_id ?? null;
    let handoffReason = null;
    if (!threadId || projectChanged) {
      handoffReason = projectChanged ? "project_changed" : "initial";
      threadId = await this.#startAndBindConversationThread({
        claim,
        previousThreadId: item.codex_thread_id ?? null,
        reason: handoffReason,
        effectIdentity,
        fence,
      });
    }
    return this.#startConversationClaimTurn({
      claim,
      threadId,
      handoffReason,
      effectIdentity,
      fence,
    });
  }

  async #startAndBindConversationThread({
    claim,
    previousThreadId,
    reason,
    effectIdentity,
    fence,
  }) {
    const project = this.store.getProject(claim.event.payload.projectId);
    if (!project) throw new Error("project not found");
    const thread = await this.#fencedConversationEffect(
      fence,
      () => this.codex.startThread({
        ...this.threadOptions,
        cwd: project.root_path,
        model: this.model,
        sandbox: "read-only",
        approvalPolicy: "never",
        effectIdentity,
      }),
    );
    const threadId = requireProtocolId(thread?.id, "conversation thread ID");
    const threadStarted = this.codex.waitForThreadStarted(threadId, {
      timeoutMs: this.lifecycleTimeoutMs,
    });
    threadStarted.catch(() => {});
    this.store.bindPrimaryConversationThread({
      projectId: claim.event.payload.projectId,
      threadId,
      previousThreadId,
      reason,
      fence,
    });
    await threadStarted;
    this.#assertPrimaryConversationFence(fence);
    this.#replayMcpDiagnostics(primaryConversationId, threadId, fence);
    return threadId;
  }

  async #startConversationClaimTurn({
    claim,
    threadId,
    handoffReason = null,
    effectIdentity,
    fence,
    retryAfterProvenNotSent = false,
  }) {
    const clientMessageId = claim.event.payload.clientMessageId;
    const priorMessages = this.primaryConversation().messages
      .filter((message) => message.id !== clientMessageId);
    const prompt = conversationTurnPrompt(claim.event.payload, priorMessages, handoffReason);
    const intent = this.store.recordPrimaryConversationDispatchIntent({
      clientMessageId,
      threadId,
      promptDigest: claim.event.payload.promptDigest,
      fence,
    });
    if (!intent.created) {
      if (!retryAfterProvenNotSent) {
        throw conversationError(
          "CONVERSATION_RECONCILIATION_REQUIRED",
          "the saved message already has a turn dispatch intent; reconnect before retrying",
        );
      }
      const retryCreated = this.#appendEventOnce(
        primaryConversationId,
        "conversation_turn_dispatch_retry_intended",
        {
          clientMessageId,
          threadId,
          promptDigest: claim.event.payload.promptDigest,
          originalIntentEventId: intent.event.id,
          attempt: 2,
        },
        fence,
      );
      if (!retryCreated) {
        throw conversationError(
          "CONVERSATION_RECONCILIATION_REQUIRED",
          "the proven pre-dispatch retry was already attempted",
        );
      }
    }
    let turn;
    try {
      turn = await this.#fencedConversationEffect(
        fence,
        () => this.codex.startTurn(threadId, prompt, { effectIdentity }),
      );
    } catch (error) {
      if (error?.code === "CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED") {
        this.#appendEventOnce(primaryConversationId, "conversation_turn_dispatch_not_sent", {
          clientMessageId,
          threadId,
          promptDigest: claim.event.payload.promptDigest,
          originalIntentEventId: intent.event.id,
          attempt: retryAfterProvenNotSent ? 2 : 1,
          errorCode: error.code,
        }, fence);
      }
      throw error;
    }
    const turnId = requireProtocolId(turn?.id, "conversation turn ID");
    const turnStarted = this.codex.waitForTurnStarted(threadId, turnId, {
      timeoutMs: this.lifecycleTimeoutMs,
    });
    turnStarted.catch(() => {});
    this.store.acceptPrimaryConversationTurn({ clientMessageId, threadId, turnId, fence });
    const activeTurn = await turnStarted;
    this.#assertPrimaryConversationFence(fence);
    this.#markTurnStarted(primaryConversationId, threadId, activeTurn, fence);
    this.#replayConversationReplies(primaryConversationId, threadId, turnId, fence);
    this.#replayTerminal(primaryConversationId, threadId, turnId, fence);
    return this.primaryConversation();
  }

  async #readConversationThreadForClaim(threadId, marker, effectIdentity, fence) {
    let observed = null;
    for (let pass = 0; pass < primaryConversationDiscoveryPasses; pass += 1) {
      observed = await this.#fencedConversationEffect(
        fence,
        () => this.codex.readThread(threadId, {
          includeTurns: true,
          allowEmpty: true,
          effectIdentity,
        }),
      );
      if (observed?.turns?.some((turn) => turnStartsWithConversationMarker(turn, marker))) break;
      if (pass + 1 < primaryConversationDiscoveryPasses) {
        await new Promise((resolve) => setTimeout(resolve, 50 * (pass + 1)));
      }
    }
    return observed;
  }

  async #recoverUnacceptedConversationClaim(claim, effectIdentity, fence) {
    if (!this.store.getProject(claim.event.payload.projectId)) throw new Error("project not found");
    const binding = this.#conversationBinding();
    const item = requireItem(this.store, primaryConversationId);
    const projectChanged = Boolean(binding && binding.payload.projectId !== claim.event.payload.projectId);
    const marker = conversationMarker(claim.event.payload);

    if (!item.codex_thread_id || projectChanged) {
      const reason = projectChanged ? "project_changed" : "initial";
      this.#appendEventOnce(primaryConversationId, "conversation_unbound_claim_restarted", {
        clientMessageId: claim.event.payload.clientMessageId,
        projectId: claim.event.payload.projectId,
        previousThreadId: item.codex_thread_id ?? null,
        reason: "no_saved_target_thread_binding",
      }, fence);
      const threadId = await this.#startAndBindConversationThread({
        claim,
        previousThreadId: item.codex_thread_id ?? null,
        reason,
        effectIdentity,
        fence,
      });
      return this.#startConversationClaimTurn({
        claim,
        threadId,
        handoffReason: reason,
        effectIdentity,
        fence,
      });
    }

    const thread = await this.#readConversationThreadForClaim(
      item.codex_thread_id,
      marker,
      effectIdentity,
      fence,
    );
    if (thread?.id !== item.codex_thread_id || !Array.isArray(thread.turns)) {
      throw conversationError(
        "CONVERSATION_RECONCILIATION_REQUIRED",
        "Codex did not return the exact saved conversation thread",
      );
    }
    const markerTurns = thread.turns.filter(
      (turn) => turnStartsWithConversationMarker(turn, marker),
    );
    if (markerTurns.length === 1 && markerTurns[0] === thread.turns.at(-1)) {
      const resumed = await this.#fencedConversationEffect(
        fence,
        () => this.codex.resumeThread(item.codex_thread_id, {
          expectedTurnId: markerTurns[0].id,
          effectIdentity,
        }),
      );
      const recoveredTurn = resumed.turns.at(-1);
      if (!turnStartsWithConversationMarker(recoveredTurn, marker)) {
        throw conversationError(
          "CONVERSATION_RECONCILIATION_REQUIRED",
          "Codex changed the saved-message marker during resume",
        );
      }
      this.store.acceptPrimaryConversationTurn({
        clientMessageId: claim.event.payload.clientMessageId,
        threadId: item.codex_thread_id,
        turnId: recoveredTurn.id,
        fence,
      });
      return this.#adoptRecoveredConversationTurn(
        claim,
        recoveredTurn,
        item.codex_thread_id,
        fence,
      );
    }
    if (markerTurns.length > 0) {
      throw conversationError(
        "CONVERSATION_RECONCILIATION_REQUIRED",
        "Codex returned an ambiguous saved-message turn",
      );
    }
    const events = this.store.listEvents(primaryConversationId);
    const exactEmptyBinding = Boolean(
      thread.turns.length === 0
      && item.codex_turn_id == null
      && !events.some(({ kind, payload }) => (
        kind === "conversation_message_accepted"
        && payload.threadId === item.codex_thread_id
      )),
    );
    const dispatchIntent = this.store.primaryConversationDispatchIntent(
      claim.event.payload.clientMessageId,
    );
    if (dispatchIntent) {
      if (
        dispatchIntent.payload.threadId !== item.codex_thread_id
        || dispatchIntent.payload.promptDigest !== claim.event.payload.promptDigest
      ) {
        throw conversationError(
          "CONVERSATION_RECONCILIATION_REQUIRED",
          "the saved turn dispatch intent does not match the bound conversation",
        );
      }
      const dispatchNotSent = events.find(({ id, kind, payload }) => (
        id > dispatchIntent.id
        && kind === "conversation_turn_dispatch_not_sent"
        && payload.clientMessageId === claim.event.payload.clientMessageId
        && payload.threadId === item.codex_thread_id
        && payload.promptDigest === claim.event.payload.promptDigest
        && payload.originalIntentEventId === dispatchIntent.id
        && payload.attempt === 1
        && payload.errorCode === "CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED"
      ));
      const retryIntent = events.find(({ id, kind, payload }) => (
        id > (dispatchNotSent?.id ?? dispatchIntent.id)
        && kind === "conversation_turn_dispatch_retry_intended"
        && payload.clientMessageId === claim.event.payload.clientMessageId
        && payload.threadId === item.codex_thread_id
        && payload.promptDigest === claim.event.payload.promptDigest
        && payload.originalIntentEventId === dispatchIntent.id
        && payload.attempt === 2
      ));
      const previousTurn = thread.turns.at(-1) ?? null;
      const previousTerminal = previousTurn
        ? this.#conversationTerminalEvent(item.codex_thread_id, previousTurn.id)
        : null;
      const exactRetryBase = exactEmptyBinding || Boolean(
        previousTurn
        && previousTurn.id === item.codex_turn_id
        && terminalTurnStatuses.has(previousTurn.status)
        && previousTerminal?.payload.status === previousTurn.status,
      );
      if (dispatchNotSent && !retryIntent && exactRetryBase) {
        if (exactEmptyBinding) {
          await this.#fencedConversationEffect(
            fence,
            () => this.codex.resumeEmptyThread(item.codex_thread_id, { effectIdentity }),
          );
        } else {
          const resumed = await this.#fencedConversationEffect(
            fence,
            () => this.codex.resumeThread(item.codex_thread_id, {
              expectedTurnId: previousTurn.id,
              effectIdentity,
            }),
          );
          const resumedLatest = resumed?.turns?.at(-1);
          if (
            resumed?.id !== item.codex_thread_id
            || resumedLatest?.id !== previousTurn.id
            || resumedLatest?.status !== previousTurn.status
          ) {
            throw conversationError(
              "CONVERSATION_RECONCILIATION_REQUIRED",
              "the saved terminal conversation changed before the proven pre-dispatch retry",
            );
          }
        }
        return this.#startConversationClaimTurn({
          claim,
          threadId: item.codex_thread_id,
          effectIdentity,
          fence,
          retryAfterProvenNotSent: true,
        });
      }
      throw conversationError(
        "CONVERSATION_RECONCILIATION_REQUIRED",
        retryIntent
          ? "the proven pre-dispatch retry was already attempted; reconnect after Codex exposes the exact marker"
          : "a turn dispatch was already attempted; reconnect after Codex exposes the exact marker",
      );
    }
    if (exactEmptyBinding) {
      await this.#fencedConversationEffect(
        fence,
        () => this.codex.resumeEmptyThread(item.codex_thread_id, { effectIdentity }),
      );
      return this.#startConversationClaimTurn({
        claim,
        threadId: item.codex_thread_id,
        effectIdentity,
        fence,
      });
    }
    if (thread.turns.length === 0) {
      throw conversationError(
        "CONVERSATION_RECONCILIATION_REQUIRED",
        "Codex returned an empty conversation after durable turn history was saved",
      );
    }

    const latest = thread.turns.at(-1);
    const terminal = this.#conversationTerminalEvent(item.codex_thread_id, latest.id);
    if (
      latest.id !== item.codex_turn_id
      || !terminalTurnStatuses.has(latest.status)
      || !terminal
    ) {
      throw conversationError(
        "CONVERSATION_RECONCILIATION_REQUIRED",
        "the saved Codex thread changed before the message could be recovered",
      );
    }
    await this.#fencedConversationEffect(
      fence,
      () => this.codex.resumeThread(item.codex_thread_id, {
        expectedTurnId: latest.id,
        effectIdentity,
      }),
    );
    return this.#startConversationClaimTurn({
      claim,
      threadId: item.codex_thread_id,
      effectIdentity,
      fence,
    });
  }

  #adoptRecoveredConversationTurn(claim, turn, threadId, fence) {
    const marker = conversationMarker(claim.event.payload);
    if (!turnStartsWithConversationMarker(turn, marker)) {
      throw conversationError(
        "CONVERSATION_RECONCILIATION_REQUIRED",
        "recovered Codex turn does not contain the exact saved-message marker",
      );
    }
    this.#persistConversationRepliesFromTurn(primaryConversationId, threadId, turn, fence);
    if (turn.status === "inProgress") {
      this.store.updateWorkItem(primaryConversationId, {
        status: "running",
        progress: "已找回同一則訊息；Codex 正在回覆",
        failure_summary: null,
      }, fence);
      this.#appendEventOnce(primaryConversationId, "codex_started", {
        threadId,
        turnId: turn.id,
        status: "inProgress",
        confirmedBy: "thread/resume",
      }, fence);
    } else if (terminalTurnStatuses.has(turn.status)) {
      this.#appendEventOnce(primaryConversationId, "codex_started", {
        threadId,
        turnId: turn.id,
        status: "recovered",
        confirmedBy: "marker-thread/read",
      }, fence);
      if (!this.#applyTerminal(primaryConversationId, {
        method: "turn/completed",
        params: { threadId, turn },
      }, fence)) {
        throw conversationError(
          "CONVERSATION_RECONCILIATION_REQUIRED",
          "the recovered Codex turn has no final reply",
        );
      }
    } else {
      throw conversationError(
        "CONVERSATION_RECONCILIATION_REQUIRED",
        "the recovered Codex turn has an unknown state",
      );
    }
    return this.primaryConversation();
  }

  async #reconcileAcceptedConversationClaim(claim, accepted, effectIdentity, fence) {
    const item = requireItem(this.store, primaryConversationId);
    const { threadId, turnId } = accepted.payload;
    if (item.codex_thread_id !== threadId || item.codex_turn_id !== turnId) {
      return this.primaryConversation();
    }
    let thread;
    try {
      thread = await this.#fencedConversationEffect(
        fence,
        () => this.codex.resumeThread(threadId, { expectedTurnId: turnId, effectIdentity }),
      );
    } catch (error) {
      if (error?.code === "CODEX_THREAD_NOT_RECOVERABLE") {
        throw conversationError(
          "CONVERSATION_RECONCILIATION_REQUIRED",
          "the exact Codex conversation could not be resumed; no replacement was started",
        );
      }
      throw error;
    }
    const turn = thread?.turns?.at(-1);
    if (thread?.id !== threadId || turn?.id !== turnId) {
      throw conversationError(
        "CONVERSATION_RECONCILIATION_REQUIRED",
        "Codex returned a different conversation during resume",
      );
    }
    this.#persistConversationRepliesFromTurn(primaryConversationId, threadId, turn, fence);
    if (turn.status === "inProgress") {
      this.store.updateWorkItem(primaryConversationId, {
        status: "running",
        progress: "已重新連線；Codex 正在回覆",
        failure_summary: null,
      }, fence);
      this.#appendEventOnce(primaryConversationId, "codex_started", {
        threadId,
        turnId,
        status: "inProgress",
        confirmedBy: "thread/resume",
      }, fence);
    } else if (terminalTurnStatuses.has(turn.status)) {
      if (!this.#hasConfirmedStart(primaryConversationId, threadId, turnId)) {
        const marker = conversationMarker(claim.event.payload);
        if (!turnStartsWithConversationMarker(turn, marker)) {
          throw conversationError(
            "CONVERSATION_RECONCILIATION_REQUIRED",
            "terminal Codex turn has no exact start or saved-message marker evidence",
          );
        }
        this.#appendEventOnce(primaryConversationId, "codex_started", {
          threadId,
          turnId,
          status: "recovered",
          confirmedBy: "marker-thread/read",
        }, fence);
      }
      if (!this.#applyTerminal(primaryConversationId, {
        method: "turn/completed",
        params: { threadId, turn },
      }, fence)) {
        throw conversationError(
          "CONVERSATION_RECONCILIATION_REQUIRED",
          "Codex completed the conversation without a recoverable final reply",
        );
      }
    } else {
      throw conversationError(
        "CONVERSATION_RECONCILIATION_REQUIRED",
        "Codex returned an unknown conversation state during resume",
      );
    }
    return this.primaryConversation();
  }

  reconnectPrimaryConversation() {
    return this.#runExclusive(primaryConversationId, "conversation-reconnect", async () => {
      const fence = this.#acquirePrimaryConversationLease();
      try {
        const events = this.store.listEvents(primaryConversationId);
        const acceptedIds = new Set(events
          .filter(({ kind }) => kind === "conversation_message_accepted")
          .map(({ payload }) => payload.clientMessageId));
        const unresolved = events.findLast(({ kind, payload }) => (
          kind === "conversation_message_claimed"
          && !acceptedIds.has(payload.clientMessageId)
        ));
        const effectIdentity = await this.#conversationEffectIdentity(fence);
        if (unresolved) {
          const claim = this.store.claimPrimaryConversationMessage({
            projectId: unresolved.payload.projectId,
            clientMessageId: unresolved.payload.clientMessageId,
            text: unresolved.payload.text,
            fence,
          });
          const result = await this.#recoverUnacceptedConversationClaim(claim, effectIdentity, fence);
          this.#appendEventOnce(primaryConversationId, "conversation_reconnected", {
            threadId: result.codex_thread_id,
            turnId: result.codex_turn_id,
            status: result.status,
          }, fence);
          return result;
        }
        const item = requireItem(this.store, primaryConversationId);
        const accepted = events.findLast(({ kind, payload }) => (
          kind === "conversation_message_accepted"
          && payload.threadId === item.codex_thread_id
          && payload.turnId === item.codex_turn_id
        ));
        if (!accepted) {
          throw conversationError(
            "CONVERSATION_RECONCILIATION_REQUIRED",
            "the saved conversation has no exact accepted message binding",
          );
        }
        const claimed = events.find(({ kind, payload }) => (
          kind === "conversation_message_claimed"
          && payload.clientMessageId === accepted.payload.clientMessageId
        ));
        if (!claimed) {
          throw conversationError(
            "CONVERSATION_RECONCILIATION_REQUIRED",
            "the accepted Codex turn has no saved user message",
          );
        }
        const result = await this.#reconcileAcceptedConversationClaim(
          { claimed: false, event: claimed, item },
          accepted,
          effectIdentity,
          fence,
        );
        this.#appendEventOnce(primaryConversationId, "conversation_reconnected", {
          threadId: result.codex_thread_id,
          turnId: result.codex_turn_id,
          status: result.status,
        }, fence);
        return result;
      } catch (error) {
        if (this.#ownsPrimaryConversationLease(fence)) {
          this.store.updateWorkItem(primaryConversationId, {
            status: "failed",
            progress: "重新連線失敗；訊息不會自動重送",
            failure_summary: boundedText(error?.message ?? error, 2_048),
          }, fence);
          this.#appendEventOnce(primaryConversationId, "conversation_reconnect_failed", {
            threadId: this.store.getWorkItem(primaryConversationId)?.codex_thread_id ?? null,
            turnId: this.store.getWorkItem(primaryConversationId)?.codex_turn_id ?? null,
            message: boundedText(error?.message ?? error, 2_048),
            code: error?.code ?? null,
          }, fence);
        }
        throw error;
      }
    });
  }

  async interruptPrimaryConversation() {
    return this.#runExclusive(primaryConversationId, "conversation-interrupt", async () => {
      const fence = this.#acquirePrimaryConversationLease();
      const item = requireItem(this.store, primaryConversationId);
      if (!["running", "waiting_approval"].includes(item.status)) {
        throw new Error("the primary conversation has no active Codex turn");
      }
      const threadId = requireProtocolId(item.codex_thread_id, "active conversation thread ID");
      const turnId = requireProtocolId(item.codex_turn_id, "active conversation turn ID");
      if (!this.codex.isTurnActive(threadId, turnId)) {
        throw new Error(`Codex turn ${threadId}/${turnId} is not confirmed active`);
      }
      try {
        const effectIdentity = await this.#conversationEffectIdentity(fence);
        const terminal = await this.#fencedConversationEffect(
          fence,
          () => this.codex.interruptTurn(threadId, turnId, {
            timeoutMs: this.lifecycleTimeoutMs,
            effectIdentity,
          }),
        );
        this.#applyTerminal(primaryConversationId, {
          method: "turn/completed",
          params: { threadId, turn: terminal },
        }, fence);
        return this.primaryConversation();
      } catch (error) {
        if (this.#ownsPrimaryConversationLease(fence)) {
          this.#failLifecycle(primaryConversationId, error, fence);
        }
        throw error;
      }
    });
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

  replaceDevelopmentRadar(input) {
    return this.store.replaceDevelopmentRadar(input);
  }

  developmentRadar() {
    return this.store.getDevelopmentRadar();
  }

  state() {
    return {
      codexConnected: this.codex.connected,
      projects: this.store.listProjects(),
      workItems: this.store.listWorkItems()
        .filter((item) => item.id !== primaryConversationId),
      installationReceiptCount: this.store.countInstallationReceipts(),
      developmentRadar: this.developmentRadar(),
    };
  }

  workItem(id) {
    requireLegacyWorkItemId(id);
    return {
      item: requireItem(this.store, id),
      events: this.store.listEvents(id),
    };
  }

  continuation(id) {
    requireLegacyWorkItemId(id);
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
    requireLegacyWorkItemId(id);
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
    requireLegacyWorkItemId(id);
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
    requireLegacyWorkItemId(id);
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
    requireLegacyWorkItemId(id);
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
    requireLegacyWorkItemId(id);
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
    requireLegacyWorkItemId(id);
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
    requireLegacyWorkItemId(id);
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

  #acquirePrimaryConversationLease() {
    if (this.conversationLeaseLossPromise) {
      throw conversationError(
        "CONVERSATION_WRITER_LOST",
        "前一個 Codex 連線仍在關閉；請稍後重新連線",
      );
    }
    const current = this.conversationLeaseFence;
    if (current && this.#ownsPrimaryConversationLease(current)) return current;
    const lease = this.store.acquirePrimaryConversationLease({
      ownerId: this.conversationOwnerId,
      ownerPid: process.pid,
      ttlMs: this.conversationLeaseTtlMs,
    });
    const fence = Object.freeze({
      ownerId: lease.owner_id,
      generation: lease.generation,
    });
    this.conversationLeaseFence = fence;
    if (this.conversationLeaseTimer) return fence;
    this.conversationLeaseTimer = setInterval(() => {
      const activeFence = this.conversationLeaseFence;
      if (!activeFence) return;
      try {
        const renewed = this.store.renewPrimaryConversationLease({
          ...activeFence,
          ttlMs: this.conversationLeaseTtlMs,
        });
        if (!renewed) throw new Error("primary conversation writer lease was lost");
      } catch {
        this.#handlePrimaryConversationLeaseLoss(activeFence).catch(() => {});
      }
    }, Math.max(1_000, Math.floor(this.conversationLeaseTtlMs / 3)));
    this.conversationLeaseTimer.unref?.();
    return fence;
  }

  #assertPrimaryConversationFence(fence) {
    this.store.assertPrimaryConversationLease(fence);
    return fence;
  }

  #ownsPrimaryConversationLease(fence = this.conversationLeaseFence) {
    return Boolean(
      fence
      && this.store.ownsPrimaryConversationLease(fence.ownerId, fence.generation)
    );
  }

  #handlePrimaryConversationLeaseLoss(fence) {
    if (
      fence
      && this.conversationLeaseFence?.ownerId === fence.ownerId
      && this.conversationLeaseFence?.generation === fence.generation
    ) this.conversationLeaseFence = null;
    if (this.conversationLeaseTimer) clearInterval(this.conversationLeaseTimer);
    this.conversationLeaseTimer = null;
    if (!this.conversationLeaseLossPromise) {
      this.conversationLeaseLossPromise = Promise.resolve()
        .then(() => this.codex.close?.())
        .catch(() => {})
        .finally(() => {
          this.conversationLeaseLossPromise = null;
        });
    }
    return this.conversationLeaseLossPromise;
  }

  #fencedConversationServerEffect(fence, operation, onSuccess = () => {}) {
    if (!this.#ownsPrimaryConversationLease(fence)) {
      void this.#handlePrimaryConversationLeaseLoss(fence);
      return false;
    }
    try {
      this.#assertPrimaryConversationFence(fence);
      operation();
      this.#assertPrimaryConversationFence(fence);
      onSuccess();
      this.#assertPrimaryConversationFence(fence);
      return true;
    } catch (error) {
      try {
        this.#assertPrimaryConversationFence(fence);
      } catch {
        void this.#handlePrimaryConversationLeaseLoss(fence);
        return false;
      }
      this.#failLifecycle(primaryConversationId, error, fence);
      this.#appendEventOnce(primaryConversationId, "conversation_server_response_failed", {
        message: boundedText(error?.message ?? error, 2_048),
        code: error?.code ?? null,
      }, fence);
      return false;
    }
  }

  async #fencedConversationEffect(fence, operation) {
    this.#assertPrimaryConversationFence(fence);
    try {
      const result = await operation();
      this.#assertPrimaryConversationFence(fence);
      return result;
    } catch (error) {
      try {
        this.#assertPrimaryConversationFence(fence);
      } catch (leaseError) {
        await this.#handlePrimaryConversationLeaseLoss(fence);
        throw leaseError;
      }
      throw error;
    }
  }

  #runExclusive(id, kind, operation) {
    const existing = this.operations.get(id);
    if (existing) {
      if (existing.kind === kind) return existing.promise;
      if (id === primaryConversationId) {
        throw conversationError(
          "CONVERSATION_MESSAGE_CONFLICT",
          "另一個不同的對話操作正在進行；本次沒有送出",
        );
      }
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

  #appendEventOnce(id, kind, payload, fence = null) {
    const encoded = JSON.stringify(payload);
    const exists = this.store.listEvents(id)
      .some((event) => event.kind === kind && JSON.stringify(event.payload) === encoded);
    if (exists) return false;
    const conversationFence = id === primaryConversationId
      ? this.#assertPrimaryConversationFence(fence ?? this.conversationLeaseFence)
      : null;
    this.store.appendEvent(id, kind, payload, conversationFence);
    return true;
  }

  #persistConversationReply(id, threadId, turnId, entry, fence = null) {
    if (
      id !== primaryConversationId
      || entry?.type !== "agentMessage"
      || entry?.phase !== "final_answer"
      || typeof entry?.id !== "string"
      || typeof entry?.text !== "string"
      || !entry.text.trim()
    ) return false;
    return this.store.recordPrimaryConversationReply({
      threadId,
      turnId,
      messageId: entry.id,
      text: entry.text,
      fence: this.#assertPrimaryConversationFence(fence ?? this.conversationLeaseFence),
    });
  }

  #persistConversationRepliesFromTurn(id, threadId, turn, fence = null) {
    if (!Array.isArray(turn?.items)) return;
    for (const entry of turn.items) {
      this.#persistConversationReply(id, threadId, turn.id, entry, fence);
    }
  }

  #replayConversationReplies(id, threadId, turnId, fence = null) {
    const entries = this.codex.notificationSnapshot?.({
      method: "item/completed",
      threadId,
      turnId,
    }) ?? [];
    for (const entry of entries) {
      this.#persistConversationReply(id, threadId, turnId, entry.message.params?.item, fence);
    }
  }

  #hasConfirmedStart(id, threadId, turnId) {
    return this.store.listEvents(id).some(({ kind, payload }) => (
      kind === "codex_started"
      && payload.threadId === threadId
      && payload.turnId === turnId
      && ["turn/started", "thread/resume", "marker-thread/read"].includes(payload.confirmedBy)
    ));
  }

  #markTurnStarted(id, threadId, turn, fence = null) {
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
      }, id === primaryConversationId ? fence : null);
    } else if (!["running", "waiting_approval"].includes(item.status)) {
      return false;
    }
    this.#appendEventOnce(id, "codex_started", {
      threadId,
      turnId,
      status: "inProgress",
      confirmedBy: "turn/started",
    }, id === primaryConversationId ? fence : null);
    return true;
  }

  #applyTerminal(id, message, fence = null) {
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

    if (id === primaryConversationId && status === "completed") {
      const hasFinalReply = this.store.listEvents(id).some(({ kind, payload }) => (
        kind === "conversation_assistant_message"
        && payload.threadId === threadId
        && payload.turnId === turnId
      ));
      if (!hasFinalReply) {
        this.store.updateWorkItem(id, {
          status: "failed",
          approval_json: null,
          progress: "Codex 已結束，但最終回覆尚未完整保存",
          failure_summary: "Codex 完成事件缺少可驗證的最終回覆",
        }, fence);
        this.#appendEventOnce(id, "conversation_terminal_missing_reply", {
          threadId,
          turnId,
          status,
        }, fence);
        return false;
      }
    }

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
        }, fence);
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
      }, id === primaryConversationId ? fence : null);
    }
    this.#appendEventOnce(id, "turn_completed", {
      threadId,
      turnId,
      status,
      error: turn.error ?? null,
    }, id === primaryConversationId ? fence : null);
    if (item.approval?.requestId != null) {
      if (id === primaryConversationId) {
        this.#fencedConversationServerEffect(
          fence,
          () => this.codex.respond(item.approval.requestId, { decision: "cancel" }),
          () => this.requestOwners.delete(item.approval.requestId),
        );
      } else {
        try {
          this.codex.respond(item.approval.requestId, { decision: "cancel" });
        } catch {
          // The App Server may have already closed the request with the turn terminal.
        }
        this.requestOwners.delete(item.approval.requestId);
      }
    }
    return true;
  }

  #replayTerminal(id, threadId, turnId, fence = null) {
    const entries = this.codex.notificationSnapshot?.({
      method: "turn/completed",
      threadId,
      turnId,
    }) ?? [];
    const terminal = entries.at(-1)?.message;
    if (terminal) this.#applyTerminal(id, terminal, fence);
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

  #persistMcpDiagnostic(id, params, fence = null) {
    this.#appendEventOnce(
      id,
      "mcp_server_startup_status_updated",
      this.#mcpDiagnosticPayload(params),
      fence,
    );
  }

  #replayMcpDiagnostics(id, threadId, fence = null) {
    const entries = this.codex.notificationSnapshot?.({
      method: "mcpServer/startupStatus/updated",
      threadId,
    }) ?? [];
    for (const entry of entries) this.#persistMcpDiagnostic(id, entry.message.params, fence);
  }

  #failLifecycle(id, error, fence = null) {
    const item = this.store.getWorkItem(id);
    if (!item || !["starting", "running", "waiting_approval"].includes(item.status)) return;
    this.store.updateWorkItem(id, {
      status: "failed",
      approval_json: null,
      progress: id === primaryConversationId
        ? "Codex 連線流程失敗；訊息不會自動重送"
        : "Codex lifecycle failed",
      failure_summary: boundedText(error?.message ?? error, 2_048),
    }, id === primaryConversationId ? fence : null);
    this.store.appendEvent(id, "codex_lifecycle_failed", {
      message: boundedText(error?.message ?? error, 2_048),
      code: error?.code ?? null,
    }, id === primaryConversationId ? fence : null);
  }

  #onServerRequest(message) {
    const threadId = message.params?.threadId;
    const turnId = message.params?.turnId;
    const item = this.#findByThread(threadId);
    if (!approvalMethods.has(message.method)) {
      const reject = () => this.codex.rejectServerRequest?.(message.id, {
          code: -32601,
          message: `Unsupported Codex App Server request: ${message.method}`,
        });
      if (item?.id === primaryConversationId) {
        this.#fencedConversationServerEffect(this.conversationLeaseFence, reject);
      } else {
        reject();
      }
      return;
    }
    if (item?.id === primaryConversationId) {
      const fence = this.conversationLeaseFence;
      this.#fencedConversationServerEffect(
        fence,
        () => this.codex.respond(message.id, { decision: "decline" }),
        () => {
          this.#appendEventOnce(primaryConversationId, "conversation_approval_declined", {
            requestId: message.id,
            method: message.method,
            threadId: threadId ?? null,
            turnId: turnId ?? null,
            reason: "primary_conversation_is_read_only",
          }, fence);
        },
      );
      return;
    }
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
      const fence = owner === primaryConversationId ? this.conversationLeaseFence : null;
      if (owner === primaryConversationId && !this.#ownsPrimaryConversationLease(fence)) return;
      this.store.updateWorkItem(owner, {
        status: "failed",
        approval_json: null,
        progress: "Codex approval request failed",
        failure_summary: response.error.message,
      }, fence);
      this.store.appendEvent(owner, "approval_request_failed", {
        requestId: id,
        code: response.error.code ?? null,
        message: response.error.message,
      }, fence);
    }
    this.requestOwners.delete(id);
  }

  #onNotification(message) {
    try {
      this.#handleNotification(message);
    } catch (error) {
      const threadId = message.params?.threadId ?? message.params?.thread?.id;
      const item = threadId ? this.#findByThread(threadId) : null;
      const fence = this.conversationLeaseFence;
      if (item?.id !== primaryConversationId || !this.#ownsPrimaryConversationLease(fence)) return;
      this.#failLifecycle(primaryConversationId, error, fence);
      this.#appendEventOnce(primaryConversationId, "conversation_notification_failed", {
        method: message.method ?? null,
        threadId: threadId ?? null,
        turnId: message.params?.turnId ?? message.params?.turn?.id ?? null,
        message: boundedText(error?.message ?? error, 2_048),
        code: error?.code ?? null,
      }, fence);
    }
  }

  #handleNotification(message) {
    const threadId = message.params?.threadId ?? message.params?.thread?.id;
    const item = threadId ? this.#findByThread(threadId) : null;
    if (!item) return;
    const fence = item.id === primaryConversationId ? this.conversationLeaseFence : null;
    if (item.id === primaryConversationId && !this.#ownsPrimaryConversationLease(fence)) return;

    if (message.method === "mcpServer/startupStatus/updated") {
      this.#persistMcpDiagnostic(item.id, message.params, fence);
      return;
    }
    if (message.method === "turn/started") {
      this.#markTurnStarted(item.id, threadId, message.params?.turn, fence);
      return;
    }
    if (message.method === "turn/completed") {
      this.#persistConversationRepliesFromTurn(item.id, threadId, message.params?.turn, fence);
      this.#applyTerminal(item.id, message, fence);
      return;
    }

    const eventTurnId = message.params?.turnId ?? null;
    if (["item/started", "item/completed"].includes(message.method)
      && eventTurnId !== item.codex_turn_id) return;
    if (message.method === "item/started") {
      const type = message.params?.item?.type ?? "item";
      this.store.updateWorkItem(item.id, { progress: `Running ${type}` }, fence);
      this.store.appendEvent(item.id, "item_started", { type }, fence);
    } else if (message.method === "item/completed") {
      const type = message.params?.item?.type ?? "item";
      this.#persistConversationReply(
        item.id,
        threadId,
        eventTurnId,
        message.params?.item,
        fence,
      );
      this.store.updateWorkItem(item.id, { progress: `Completed ${type}` }, fence);
      this.store.appendEvent(item.id, "item_completed", { type }, fence);
    }
  }
}
