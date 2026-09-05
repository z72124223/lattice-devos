import { createHash, randomUUID } from "node:crypto";
import { execFile, spawn } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, realpath, readFile, writeFile, stat } from "node:fs/promises";
import path from "node:path";
import { CodexAppServer } from "./codex-app-server.mjs";
import { formalWorkError } from "./formal-work-store.mjs";
import { closedChildEnvironment, loadLatticeRuntimeConfiguration } from "./lattice-runtime-health.mjs";
import { startResultPreview, closeResultPreview, isOwnedResultPreview } from "./result-preview.mjs";
import { recoveryPrompt, recoverySummary, openCircuitSummary, isExecutionDenied, deniedItemIds } from "./execution-recovery.mjs";

const execute = promisify(execFile);
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
const terminal = new Set(["TURN_COMPLETED", "TURN_FAILED", "INTERRUPTED"]);
const executionOutput = { type: "object", additionalProperties: false,
  required: ["summary", "artifact_path", "test_path"], properties: {
    summary: { type: "string" }, artifact_path: { type: "string" }, test_path: { type: "string" },
  } };
const verificationOutput = { type: "object", additionalProperties: false,
  required: ["passed", "summary"], properties: { passed: { type: "boolean" }, summary: { type: "string" } } };
const bounded = (value, maximum = 2048) => [...String(value ?? "")].slice(0, maximum).join("");
const byteBounded = (value, maximum) => {
  let result = "", length = 0;
  for (const character of String(value ?? "")) { length += Buffer.byteLength(character); if (length > maximum) break; result += character; }
  return result;
};
function marker(claim, input) { return `[LATTICE_TASK:${claim.task_ref}:${claim.claim_id}:${input}]`; }
function hasMarker(turn, text) {
  return turn?.items?.some((item) => item.type === "userMessage" && item.content?.some((part) =>
    part.type === "text" && (part.text === text || part.text?.startsWith(`${text}\n`))));
}
function finalText(turn) {
  return turn?.items?.filter((item) => item.type === "agentMessage").at(-1)?.text ?? "";
}
function outputObject(turn) {
  try { return JSON.parse(finalText(turn)); }
  catch { throw formalWorkError("CONTROL_RESULT_FORMAT_REJECTED", "Codex 成果格式尚未通過核對，可在原工作補正。"); }
}
async function git(cwd, args) {
  return (await execute("git", ["-c", "core.longpaths=true", ...args], { cwd, windowsHide: true, timeout: 30000, maxBuffer: 1024 * 1024 })).stdout.trim();
}
async function existingFile(workspace, relative) {
  if (typeof relative !== "string" || !relative || path.isAbsolute(relative)) {
    throw formalWorkError("CONTROL_RESULT_PATH_REJECTED", "成果必須是這項工作中的檔案。");
  }
  const root = await realpath(workspace), target = await realpath(path.resolve(root, relative));
  const scoped = path.relative(root, target);
  if (scoped === ".." || scoped.startsWith(`..${path.sep}`) || path.isAbsolute(scoped) || !(await stat(target)).isFile()) {
    throw formalWorkError("CONTROL_RESULT_PATH_REJECTED", "成果檔案超出這項工作的範圍。");
  }
  return { path: target, sha256: sha(await readFile(target)) };
}

// Native Codex remains the execution harness. This service only records its exact
// identities/events and invokes the fixed, evidence-producing result importer.
export class FormalTaskService {
  constructor({ store, codex = new CodexAppServer(), configurationLoader = loadLatticeRuntimeConfiguration }) {
    Object.assign(this, { store, codex, configurationLoader });
    this.operations = new Map();
    this.owners = new Map();
    this.questions = new Map();
    this.progressAt = new Map();
    this.previews = new Map();
    this.deniedTurns = new Map();
    this.closed = false;
    this.onNotification = (message) => {
      const threadId = message.params?.threadId;
      const owner = this.owners.get(threadId);
      if (!owner || this.closed) return;
      if (message.method === "item/completed" && isExecutionDenied(message.params?.item)) {
        void this.serial(owner.taskRef, () => this.redirectDeniedTurn(owner, message.params))
          .catch((error) => this.recordFailure(owner, error));
      } else if (message.method === "turn/completed") {
        void this.serial(owner.taskRef, () => this.reconcile(owner.projectId, owner.taskRef, { advance: true }))
          .catch((error) => this.recordFailure(owner, error));
      } else if (message.method === "item/completed" && message.params?.item?.type === "agentMessage"
        && Date.now() - (this.progressAt.get(threadId) ?? 0) >= 5000) {
        this.progressAt.set(threadId, Date.now());
        void this.serial(owner.taskRef, async () => {
          const detail = await this.store.detail(owner.projectId, owner.taskRef);
          const claim = detail.claims.find((row) => row.claim_id === owner.claimId);
          if (!claim || terminal.has(claim.turn_status) || claim.turn_id !== message.params.turnId) return;
          let text = message.params.item.text;
          try { text = JSON.parse(text).summary ?? text; } catch { /* ordinary progress */ }
          if (text) await this.observe(claim, "PROGRESS", { summary: byteBounded(text, 2048) });
        }).catch((error) => this.recordFailure(owner, error));
      }
    };
    this.onRequest = (message) => {
      const owner = this.owners.get(message.params?.threadId);
      if (!owner || this.closed) return this.codex.rejectServerRequest(message.id);
      this.codex.deferServerRequest(message.id, { timeoutMs: 3600000 });
      void this.serial(owner.taskRef, () => this.recordQuestion(owner, message)).catch(() => {
        try { this.codex.rejectServerRequest(message.id); } catch { /* connection already ended */ }
      });
    };
    codex.on("notification", this.onNotification);
    codex.on("serverRequest", this.onRequest);
  }
  serial(key, action) {
    const operation = (this.operations.get(key) ?? Promise.resolve()).catch(() => {}).then(() => {
      if (this.closed) throw formalWorkError("CONTROL_CLOSED", "服務正在關閉。");
      return action();
    });
    this.operations.set(key, operation);
    void operation.finally(() => { if (this.operations.get(key) === operation) this.operations.delete(key); }).catch(() => {});
    return operation;
  }
  async observe(claim, kind, values = {}) {
    const result = await this.store.update({ action: "OBSERVE", task_ref: claim.task_ref,
      claim_id: claim.claim_id, request_id: values.request_id ?? randomUUID(),
      expected_sequence: claim.last_sequence ?? 0, kind, summary: values.summary ?? "",
      ...(claim.thread_id ? { thread_id: claim.thread_id } : {}),
      ...(claim.turn_id ? { turn_id: claim.turn_id } : {}),
      ...(claim.input_id ? { input_id: claim.input_id } : {}), ...values });
    claim.last_sequence = result.record.sequence;
    return result.record;
  }
  async redirectDeniedTurn(owner, { turnId, item }) {
    const detail = await this.store.detail(owner.projectId, owner.taskRef);
    const claim = detail.claims.find((row) => row.claim_id === owner.claimId);
    if (detail.completion_verified || !claim || claim.archived || terminal.has(claim.turn_status) || claim.turn_id !== turnId) return;
    const key = `${claim.thread_id}:${turnId}`;
    let denied = this.deniedTurns.get(key);
    if (!denied) {
      const thread = await this.codex.readThread(claim.thread_id);
      denied = deniedItemIds(thread.turns?.find((turn) => turn.id === turnId));
      this.deniedTurns.set(key, denied);
    } else if (denied.has(item.id) || denied.size >= 2) return;
    denied.add(item.id);
    const open = denied.size >= 2;
    const requestId = `recovery:${sha(JSON.stringify([claim.claim_id, turnId, open ? "open" : "redirect"]))}`;
    // Persist intent before a native side effect. Never resend an uncertain
    // redirect after restart; every dispatched turn already carries the policy.
    if (detail.product.observations.some((row) => row.request_id === requestId)) return;
    const previousSequence = claim.last_sequence;
    const observation = await this.observe(claim, "PROGRESS", { request_id: requestId,
      summary: open ? openCircuitSummary : recoverySummary });
    if (observation.sequence <= previousSequence) return;
    if (!this.codex.isTurnActive(claim.thread_id, turnId)) return;
    if (open) await this.codex.interruptTurn(claim.thread_id, turnId);
    else await this.codex.request("turn/steer", { threadId: claim.thread_id,
      expectedTurnId: turnId, input: [{ type: "text", text: recoveryPrompt }] });
  }
  create({ projectId, objective, clientRequestId }) {
    if (typeof objective !== "string" || !objective.trim() || [...objective].length > 512
      || !/^[A-Za-z0-9._:-]{1,64}$/u.test(clientRequestId ?? "")) {
      throw new TypeError("請用 512 字以內描述想完成的工作。");
    }
    return this.serial(`create:${clientRequestId}`, async () => {
      const registered = await this.store.submit({ client_request_id: clientRequestId,
        project_id: projectId, objective: objective.trim() });
      let detail = await this.store.detail(projectId, registered.task_ref);
      if (!detail.metadata) {
        await this.store.update({ action: "METADATA", task_ref: detail.id,
          request_id: `metadata:${clientRequestId}`, expected_revision: 0,
          title: byteBounded(objective.trim().split(/\r?\n/u)[0], 240), priority: 2,
          success_criteria: `完成需求：${objective.trim()}\n提供可執行的成果與使用方式。\n實際測試主要操作、錯誤輸入與需求中的資料保存行為。\n由獨立 Codex 回合核對需求，並由固定測試程序驗證後保存成果。` });
        detail = await this.store.detail(projectId, detail.id);
      }
      // A repeated submission returns its existing identity. Starting/recovering
      // it is a separate explicit action, so a lost response cannot send a turn twice.
      return detail;
    });
  }
  async ensureWorkspace(claim, repositoryPath) {
    const target = path.resolve(claim.worktree_path);
    await mkdir(path.dirname(target), { recursive: true });
    const common = await realpath(await git(repositoryPath, ["rev-parse", "--path-format=absolute", "--git-common-dir"]));
    try {
      const current = await realpath(await git(target, ["rev-parse", "--path-format=absolute", "--git-common-dir"]));
      if (current !== common) throw formalWorkError("CONTROL_WORKSPACE_REJECTED", "工作目錄不屬於這個專案。");
    } catch (error) {
      if (error.code === "CONTROL_WORKSPACE_REJECTED") throw error;
      try { await stat(target); throw formalWorkError("CONTROL_WORKSPACE_REJECTED", "工作目錄已存在，無法安全建立。"); }
      catch (missing) { if (missing.code !== "ENOENT") throw missing; }
      await git(repositoryPath, ["worktree", "add", "--detach", target, "HEAD"]);
    }
    return target;
  }
  executionPrompt(detail) {
    const preview = "網頁成果請匯出 async startServer({port,host})，只綁定 127.0.0.1，回傳已監聽的 node:http Server，允許 port=0；匯入模組時不要自行監聽。這讓使用者完成後可在 App 直接試用，無須輸入命令。";
    detail = { ...detail, success_criteria: `${detail.success_criteria}\n${preview}` };
    return `你正在執行已正式登記的 LATTICE 工作。task_ref=${detail.id}。此工作身份已由 Runtime 保存，不要另建任務或改動 LATTICE 任務狀態。\n需求：${detail.objective}\n驗收條件：${detail.success_criteria}\n在目前隔離工作目錄完成可操作的小型軟體。優先沿用專案；若無相關功能，使用 Node.js 標準函式庫完成。產出真正可執行的成果以及 node --test 可執行的實質測試，涵蓋需求主要操作與錯誤情境。不要因測試通過而省略使用介面或使用說明。所有產物留在工作目錄，保留原有檔案。除非需求明確授權，不做 push、merge、發布、付款、帳戶變更或外部訊息。必要產品資訊才透過 request_user_input 詢問。不要要求使用者審查程式碼。完成後回傳指定 JSON：summary 用繁體中文解釋成果與啟動方式，artifact_path 是主要可執行成果的相對檔案路徑，test_path 是實際 Node 測試檔相對路徑。`;
  }
  start(projectId, taskRef) {
    return this.serial(taskRef, async () => {
      let detail = await this.store.detail(projectId, taskRef);
      if (detail.completion_verified) return detail;
      const existing = detail.claims.find((claim) => claim.phase === "EXECUTION");
      if (existing) return this.reconcile(projectId, taskRef, { resumePrepared: true });
      const result = await this.store.update({ action: "CLAIM", task_ref: taskRef,
        claim_id: randomUUID(), phase: "EXECUTION", prompt: this.executionPrompt(detail) });
      const claim = { ...result.record, last_sequence: 0 };
      try { await this.launch(claim, result.record.repository_path, projectId); }
      catch (error) {
        if (!claim.thread_id) await this.observe(claim, "CLAIM_FAILED", { summary: "啟動中斷；續接前會先找回原生空白對話。" }).catch(() => {});
        throw error;
      }
      return this.store.detail(projectId, taskRef);
    });
  }
  async requestStart(projectId, taskRef) {
    const detail = await this.serial(taskRef, async () => {
      const current = await this.store.detail(projectId, taskRef);
      if (!current.completion_verified && !current.claims.some((claim) => claim.phase === "EXECUTION")) {
        await this.store.update({ action: "CLAIM", task_ref: taskRef,
          claim_id: randomUUID(), phase: "EXECUTION", prompt: this.executionPrompt(current) });
      }
      return this.store.detail(projectId, taskRef);
    });
    // The HTTP acknowledgement follows durable intent, while native startup
    // continues on the existing per-task queue and is recoverable after restart.
    if (!detail.completion_verified) void this.start(projectId, taskRef)
      .catch((error) => this.recordFailure({ projectId, taskRef }, error));
    return detail;
  }
  async restore(projectIds) {
    for (const projectId of projectIds) {
      if (this.closed) break;
      try {
        const { rows } = await this.store.readProject(projectId, { fresh: true });
        for (const row of rows) {
          if (row.completion_verified || !row.claims.length || row.claims.every((claim) => claim.archived)) continue;
          await this.serial(row.id, () => this.reconcile(projectId, row.id, { advance: true, resumePrepared: true }))
            .catch((error) => this.recordFailure({ projectId, taskRef: row.id }, error));
        }
      } catch (error) { await this.recordFailure({ projectId, taskRef: null }, error); }
    }
  }
  async launch(claim, repositoryPath, projectId) {
    const cwd = await this.ensureWorkspace(claim, repositoryPath);
    const thread = await this.codex.startThread({ cwd, model: claim.model,
      approvalPolicy: "on-request", sandbox: claim.phase === "VERIFICATION" ? "read-only" : "workspace-write",
      config: { model_reasoning_effort: "ultra" } });
    await this.observe(claim, "THREAD_BOUND", { thread_id: thread.id, turn_id: null, input_id: null,
      summary: claim.phase === "EXECUTION" ? "已建立原生 Codex 執行對話。" : "已建立獨立 Codex 驗收對話。" });
    claim.thread_id = thread.id;
    this.owners.set(thread.id, { projectId, taskRef: claim.task_ref, claimId: claim.claim_id });
    await this.dispatch(claim, claim.claim_id, claim.prompt);
  }
  async dispatch(claim, inputId, prompt) {
    if (claim.turn_id) this.deniedTurns.delete(`${claim.thread_id}:${claim.turn_id}`);
    await this.observe(claim, "DISPATCH_STARTED", { turn_id: null, input_id: inputId, summary: "已保存本次派送身份，正在等待 Codex 回合確認。" });
    claim.turn_id = null;
    claim.input_id = inputId;
    const turn = await this.codex.startTurn(claim.thread_id, `${marker(claim, inputId)}\n${prompt}\n\n${recoveryPrompt}`, {
      model: claim.model, outputSchema: claim.phase === "EXECUTION" ? executionOutput : verificationOutput,
    });
    await this.observe(claim, "TURN_BOUND", { turn_id: turn.id, input_id: inputId, summary: "Codex 已開始本次工作回合。" });
    claim.turn_id = turn.id;
  }
  async recoverPrepared(detail, claim) {
    if (!claim.thread_id) {
      // No turn can be sent before THREAD_BOUND commits. An interrupted
      // preparation may create a fresh empty conversation for the same task;
      // unrelated empty conversations are never adopted by path/time guesses.
      const result = await this.store.update({ action: "CLAIM", task_ref: claim.task_ref,
        claim_id: claim.claim_id, phase: claim.phase, prompt: claim.prompt });
      await this.launch(claim, result.record.repository_path, detail.project_id);
      return;
    }
    if (!claim.dispatch_started) {
      await this.codex.resumeEmptyThread(claim.thread_id);
      this.owners.set(claim.thread_id, { projectId: detail.project_id, taskRef: detail.id, claimId: claim.claim_id });
      await this.dispatch(claim, claim.claim_id, claim.prompt);
    }
  }
  async reconcile(projectId, taskRef, { advance = false, resumePrepared = false } = {}) {
    let detail = await this.store.detail(projectId, taskRef);
    let circuitOpen = false;
    for (const claim of detail.claims) {
      if (resumePrepared && !claim.dispatch_started && !claim.archived) {
        await this.recoverPrepared(detail, claim);
        continue;
      }
      if (!claim.thread_id) continue;
      this.owners.set(claim.thread_id, { projectId, taskRef, claimId: claim.claim_id });
      if (claim.archived) continue;
      let thread;
      const ownedActive = claim.turn_id && this.codex.isTurnActive(claim.thread_id, claim.turn_id);
      try {
        thread = claim.turn_id && !this.codex.isTurnActive(claim.thread_id, claim.turn_id)
          ? await this.codex.resumeThread(claim.thread_id, { expectedTurnId: claim.turn_id })
          : await this.codex.readThread(claim.thread_id, { allowEmpty: true });
      }
      catch (error) {
        if (error.code === "CODEX_THREAD_ARCHIVED") {
          await this.observe(claim, "ARCHIVED", { summary: "原生 Codex 對話已封存；工作身份保留。" });
          continue;
        }
        throw error;
      }
      const matches = (thread.turns ?? []).filter((turn) => hasMarker(turn, marker(claim, claim.input_id)));
      if (matches.length > 1) throw formalWorkError("CONTROL_TURN_AMBIGUOUS", "偵測到重複回合身份，已停止派送。");
      const turn = claim.turn_id ? thread.turns.find((row) => row.id === claim.turn_id) : matches[0];
      if (!turn) continue;
      if (!hasMarker(turn, marker(claim, claim.input_id))) throw formalWorkError("CONTROL_TURN_MISMATCH", "原生回合與正式工作身份不符。");
      if (!claim.turn_id) {
        if (thread.turns.at(-1)?.id !== turn.id) throw formalWorkError("CONTROL_TURN_MISMATCH", "原生對話已有其他回合，未採納過時回合。");
        await this.codex.resumeThread(claim.thread_id, { expectedTurnId: turn.id });
        await this.observe(claim, "TURN_BOUND", { turn_id: turn.id, summary: "已核對回應遺失前建立的原回合。" });
        claim.turn_id = turn.id;
      }
      // Reconstruct the bound from native history, including after a restart.
      // Two denied tool results must never advance into automatic repair,
      // queued input, verification or a result import, even after a final reply.
      const supersededVerification = claim.phase === "VERIFICATION"
        && claim.execution_sequence !== detail.claims.find((row) => row.phase === "EXECUTION")?.dispatch_sequence;
      if (!supersededVerification && deniedItemIds(turn).size >= 2) {
        circuitOpen = true;
        const stopped = this.codex.isTurnActive(claim.thread_id, turn.id)
          ? await this.codex.interruptTurn(claim.thread_id, turn.id) : turn;
        if (!terminal.has(claim.turn_status) && ["completed", "failed", "interrupted"].includes(stopped?.status)) {
          const kind = { completed: "TURN_COMPLETED", failed: "TURN_FAILED", interrupted: "INTERRUPTED" }[stopped.status];
          await this.observe(claim, kind, { summary: openCircuitSummary });
          claim.turn_status = kind;
        }
        continue;
      }
      // Cold persisted reads can normalize an ongoing turn to interrupted.
      // A still-live owner's notification has precedence over that projection.
      if (ownedActive && this.codex.isTurnActive(claim.thread_id, claim.turn_id)) continue;
      if (!terminal.has(claim.turn_status) && ["completed", "failed", "interrupted"].includes(turn.status)) {
        const kind = { completed: "TURN_COMPLETED", failed: "TURN_FAILED", interrupted: "INTERRUPTED" }[turn.status];
        await this.observe(claim, kind, { summary: bounded(finalText(turn) || `Codex 回合狀態：${turn.status}`) });
        claim.turn_status = kind;
      }
    }
    detail = await this.store.detail(projectId, taskRef);
    if (circuitOpen) return detail;
    // INPUT_QUEUED is the durable send intent. Recover that exact input before
    // advancing an old result into verification; DISPATCH_STARTED still uses
    // native marker reconciliation and is never blindly sent again.
    if ((advance || resumePrepared) && !detail.completion_verified) {
      const queuedClaim = detail.claims.find((claim) => !claim.archived && terminal.has(claim.turn_status)
        && claim.pending_inputs?.length);
      if (queuedClaim) {
        const queued = queuedClaim.pending_inputs[0];
        await this.codex.resumeThread(queuedClaim.thread_id, { expectedTurnId: queuedClaim.turn_id });
        await this.dispatch(queuedClaim, queued.input_id, queued.summary);
        return this.store.detail(projectId, taskRef);
      }
    }
    if (advance && !detail.completion_verified) {
      const executor = detail.claims.find((claim) => claim.phase === "EXECUTION");
      const verifier = detail.claims.find((claim) => claim.phase === "VERIFICATION");
      if (executor?.turn_status === "TURN_COMPLETED" && !executor.archived
        && (!verifier || (terminal.has(verifier.turn_status) && verifier.execution_sequence !== executor.dispatch_sequence))) {
        await this.beginVerification(detail, executor, verifier);
      } else if (verifier?.turn_status === "TURN_COMPLETED" && !verifier.archived
        && verifier.execution_sequence === executor?.dispatch_sequence) await this.finishVerification(detail, verifier);
    }
    return this.store.detail(projectId, taskRef);
  }
  async beginVerification(detail, executor, existing = null) {
    const thread = await this.codex.readThread(executor.thread_id);
    const output = outputObject(thread.turns.find((turn) => turn.id === executor.turn_id));
    await existingFile(executor.worktree_path, output.artifact_path);
    await existingFile(executor.worktree_path, output.test_path);
    const prompt = `你是這項 LATTICE 工作的獨立驗收者。task_ref=${detail.id}，不要建立新任務或修改任務狀態。\n需求：${detail.objective}\n驗收條件：${detail.success_criteria}\n執行者成果：${JSON.stringify(output)}\n唯讀檢查現有成果與測試，核對每項需求是否被實作、測試是否有實質覆蓋及使用說明是否可操作。不要修改檔案、安裝依賴或執行會寫入資料的測試；Runtime 隨後會以固定 Node 程序實際執行測試，保存日誌並檢查成果與測試檔案未遭改動。只在上述唯讀檢查通過時回傳 passed=true。summary 用繁體中文說明實際查核或可定位缺口，清楚區分查閱與執行。指定JSON中的 passed 不會直接結案。`;
    if (existing) {
      if (existing.archived) {
        await this.codex.unarchiveThread(existing.thread_id);
        await this.observe(existing, "REOPENED", { summary: "為補正後成果重開原驗收對話。" });
      }
      await this.codex.resumeThread(existing.thread_id, { expectedTurnId: existing.turn_id });
      const inputId = `verify:${executor.input_id}`;
      await this.observe(existing, "INPUT_QUEUED", { turn_id: null, input_id: inputId, summary: prompt });
      this.owners.set(existing.thread_id, { projectId: detail.project_id, taskRef: detail.id, claimId: existing.claim_id });
      await this.dispatch(existing, inputId, prompt);
      return;
    }
    const result = await this.store.update({ action: "CLAIM", task_ref: detail.id,
      claim_id: randomUUID(), phase: "VERIFICATION", prompt });
    await this.launch({ ...result.record, last_sequence: 0 }, result.repository_path ?? result.record.repository_path, detail.project_id);
  }
  async finishVerification(detail, verifier) {
    const verifiedThread = await this.codex.readThread(verifier.thread_id);
    const verdict = outputObject(verifiedThread.turns.find((turn) => turn.id === verifier.turn_id));
    const already = verifier.verification_outcome ?? detail.product.observations.filter((row) => row.claim_id === verifier.claim_id
      && row.turn_id === verifier.turn_id && ["VERIFICATION_PASSED", "VERIFICATION_FAILED"].includes(row.kind)).at(-1);
    if (verdict.passed !== true) {
      if (!already) await this.observe(verifier, "VERIFICATION_FAILED", { summary: bounded(verdict.summary) });
      const executor = detail.claims.find((claim) => claim.phase === "EXECUTION");
      if (executor && !executor.archived && terminal.has(executor.turn_status) && (executor.repair_attempts ?? 0) < 2) {
        const inputId = `repair:${verifier.turn_id}`;
        const prompt = `獨立驗收發現下列缺口，請在原工作補正並執行相關測試，保留既有需求與工作身份。\n${byteBounded(verdict.summary, 4096)}\n完成後回傳原指定成果 JSON。`;
        await this.codex.resumeThread(executor.thread_id, { expectedTurnId: executor.turn_id });
        if (!executor.pending_inputs?.some((row) => row.input_id === inputId)) {
          await this.observe(executor, "INPUT_QUEUED", { turn_id: null, input_id: inputId, summary: prompt });
        }
        await this.dispatch(executor, inputId, prompt);
      }
      return;
    }
    const executor = detail.claims.find((claim) => claim.phase === "EXECUTION");
    const executionThread = await this.codex.readThread(executor.thread_id);
    const output = outputObject(executionThread.turns.find((turn) => turn.id === executor.turn_id));
    const artifact = await existingFile(executor.worktree_path, output.artifact_path);
    const testFile = await existingFile(executor.worktree_path, output.test_path);
    const configuration = await this.configurationLoader();
    const root = path.join(configuration.environment.LATTICE_DELIVERY_ROOT, "control-results", detail.id, verifier.turn_id);
    await mkdir(root, { recursive: true });
    const retain = async (record) => {
      const bytes = Buffer.from(JSON.stringify(record));
      const hash = sha(bytes);
      const destination = path.join(root, `${hash}.json`);
      try { await writeFile(destination, bytes, { flag: "wx" }); }
      catch (error) { if (error.code !== "EEXIST" || sha(await readFile(destination)) !== hash) throw error; }
      return `evidence:sha256:${hash}`;
    };
    const artifactRef = await retain({ schema: "lattice.local-result.artifact.v1", task_ref: detail.id,
      path: artifact.path, sha256: artifact.sha256 });
    const acceptanceRef = await retain({ schema: "lattice.local-result.acceptance.v1", task_ref: detail.id,
      artifact_ref: artifactRef, test_path: testFile.path, test_sha256: testFile.sha256,
      executor_id: `codex:${executor.thread_id}:${executor.turn_id}`,
      verifier_id: `codex:${verifier.thread_id}:${verifier.turn_id}`, runner_profile: "NODE_TEST_V1" });
    if (!already || already.kind === "VERIFICATION_FAILED") await this.observe(verifier, "VERIFICATION_PASSED", { summary: bounded(verdict.summary), evidence_ref: acceptanceRef });
    else if (already.evidence_ref !== acceptanceRef) throw formalWorkError("CONTROL_RESULT_CHANGED", "驗收後檔案已變更，必須在原工作重新驗證。");
    const manifest = { schema: "lattice.local-result-import.v1", evidence_root: root,
      workspace: executor.worktree_path, task_ref: detail.id,
      client_request_id: detail.task.client_request_id, expected_ledger_head_digest: detail.ledger_head_digest,
      artifact_ref: artifactRef, acceptance_ref: acceptanceRef };
    const manifestPath = path.join(root, "local-result-import.json");
    await writeFile(manifestPath, JSON.stringify(manifest, null, 2));
    try { await this.importResult(configuration, manifestPath); }
    catch (error) {
      const current = await this.store.detail(detail.project_id, detail.id);
      if (current.completion_verified) return;
      const currentVerifier = current.claims.find((row) => row.claim_id === verifier.claim_id);
      if (currentVerifier?.turn_id === verifier.turn_id) {
        await this.observe(currentVerifier, "VERIFICATION_FAILED", {
          summary: "固定測試或成果保存未完成；工作保留，可重新核對驗收。",
        });
      }
      throw error;
    }
    this.store.invalidate();
  }
  importResult(configuration, manifestPath) {
    return new Promise((resolve, reject) => {
      const child = spawn(configuration.executablePath, ["--local-result-import", manifestPath], {
        env: closedChildEnvironment({ ...configuration.environment, LATTICE_LOCAL_RESULT_NODE_EXE: process.execPath }),
        stdio: ["ignore", "pipe", "pipe"], windowsHide: true,
      });
      let bytes = 0, output = "";
      const timer = setTimeout(() => child.kill(), 180000);
      child.stdout.on("data", (chunk) => { bytes += chunk.length; if (bytes > 1048576) child.kill(); else output += chunk.toString("utf8"); });
      child.stderr.on("data", (chunk) => { bytes += chunk.length; if (bytes > 1048576) child.kill(); });
      child.once("error", (error) => { clearTimeout(timer); reject(error); });
      child.once("close", (code) => {
        clearTimeout(timer);
        if (code !== 0) reject(formalWorkError("CONTROL_RESULT_IMPORT_FAILED", "固定驗收程序尚未通過，成果未結案。"));
        else resolve(output);
      });
    });
  }
  async recordQuestion(owner, message) {
    const detail = await this.store.detail(owner.projectId, owner.taskRef);
    const claim = detail.claims.find((row) => row.claim_id === owner.claimId);
    if (!claim || message.params?.threadId !== claim.thread_id || message.params?.turnId !== claim.turn_id) {
      throw formalWorkError("CONTROL_QUESTION_TURN_MISMATCH", "提問來自不同的原生回合。");
    }
    const approval = ["item/commandExecution/requestApproval", "item/fileChange/requestApproval"].includes(message.method);
    if (!approval && message.method !== "item/tool/requestUserInput") throw formalWorkError("CONTROL_REQUEST_UNSUPPORTED", "Codex 要求了尚未支援的互動。");
    const id = `q:${sha(JSON.stringify([message.method, message.params]))}`;
    const pending = claim.pending_questions?.find((row) => row.approval_id === id);
    this.questions.set(id, { nativeId: message.id, owner, method: message.method });
    if (pending) return;
    const resolved = await this.store.questionResolution?.(owner.projectId, owner.taskRef, id);
    if (resolved) {
      if (resolved.turn_id !== claim.turn_id) throw formalWorkError("CONTROL_QUESTION_TURN_MISMATCH", "保存的回覆不屬於目前回合。");
      const response = approval ? { decision: resolved.approval_decision } : resolved.payload;
      this.codex.respond(message.id, response);
      this.questions.delete(id);
      return;
    }
    await this.observe(claim, approval ? "APPROVAL_REQUESTED" : "QUESTION_REQUESTED", {
      approval_id: id, summary: bounded(message.params?.reason ?? "Codex 需要補充資訊才能繼續。", 512),
      payload: { method: message.method, params: message.params },
    });
  }
  async answer(projectId, taskRef, { questionId, decision, answers }) {
    const retained = this.questions.get(questionId);
    if (!retained || retained.owner.projectId !== projectId || retained.owner.taskRef !== taskRef) {
      throw formalWorkError("CONTROL_QUESTION_RECONNECT_REQUIRED", "原生提問連線已結束，請先核對原工作回合。");
    }
    const detail = await this.store.detail(projectId, taskRef);
    const claim = detail.claims.find((row) => row.claim_id === retained.owner.claimId);
    const approval = retained.method !== "item/tool/requestUserInput";
    const response = approval ? { decision } : { answers };
    await this.observe(claim, approval ? "APPROVAL_RESOLVED" : "QUESTION_RESOLVED", {
      request_id: `resolve:${questionId}`,
      approval_id: questionId, ...(approval ? { decision } : { payload: response }), summary: "已保存使用者回覆。",
    });
    this.codex.respond(retained.nativeId, response);
    this.questions.delete(questionId);
    return this.store.detail(projectId, taskRef);
  }
  async recordFailure(owner, error) {
    // Keep the native identity and failure visible. A read/reconcile can recover
    // an uncertain write; no automatic turn retry is dispatched here.
    this.store.invalidate();
    this.lastError = { task_ref: owner.taskRef, code: error.code ?? "CONTROL_TASK_FAILED", message: bounded(error.message) };
  }
  async conversation(projectId, taskRef, phase = "EXECUTION") {
    const detail = await this.store.detail(projectId, taskRef);
    const claim = detail.claims.find((row) => row.phase === phase);
    if (!claim?.thread_id) return { task_ref: taskRef, thread_id: null, messages: [] };
    const thread = await this.codex.readThread(claim.thread_id, { allowEmpty: true });
    const messages = (thread.turns ?? []).slice(-4).flatMap((turn) => turn.items
      .filter((item) => item.type === "agentMessage").slice(-8).map((item) => {
        let text = item.text;
        try { const output = JSON.parse(text); if (typeof output.summary === "string") text = output.summary; } catch { /* ordinary commentary */ }
        return { turn_id: turn.id, status: turn.status, text: byteBounded(text, 8192) };
      }));
    return { task_ref: taskRef, thread_id: claim.thread_id, messages,
      latest_turn_id: thread.turns.at(-1)?.id ?? null, latest_turn_status: thread.turns.at(-1)?.status ?? null };
  }
  async openResult(projectId, taskRef) {
    const detail = await this.store.detail(projectId, taskRef);
    const executor = detail.claims.find((claim) => claim.phase === "EXECUTION");
    const verifier = detail.claims.find((claim) => claim.phase === "VERIFICATION");
    if (!detail.completion_verified || !executor || verifier?.verification_outcome?.kind !== "VERIFICATION_PASSED") {
      throw formalWorkError("CONTROL_RESULT_NOT_VERIFIED", "工作尚未完成實際驗收，暫時無法開啟成果。");
    }
    const configuration = await this.configurationLoader();
    const root = path.join(configuration.environment.LATTICE_DELIVERY_ROOT, "control-results", taskRef, verifier.turn_id);
    const receipt = async (reference, kind) => {
      if (!/^evidence:sha256:[a-f0-9]{64}$/u.test(reference ?? "")) throw formalWorkError("CONTROL_RESULT_CHANGED", "成果證據無法核對。");
      const hash = reference.slice(-64), bytes = await readFile(path.join(root, `${hash}.json`));
      if (sha(bytes) !== hash) throw formalWorkError("CONTROL_RESULT_CHANGED", "成果證據已變更，請重新核對。");
      const value = JSON.parse(bytes);
      if (value.schema !== `lattice.local-result.${kind}.v1` || value.task_ref !== taskRef) throw formalWorkError("CONTROL_RESULT_CHANGED", "成果不屬於這項工作。");
      return value;
    };
    const acceptance = await receipt(verifier.verification_outcome.evidence_ref, "acceptance");
    const artifactReceipt = await receipt(acceptance.artifact_ref, "artifact");
    const artifact = await existingFile(executor.worktree_path, path.relative(executor.worktree_path, artifactReceipt.path));
    if (artifact.sha256 !== artifactReceipt.sha256) throw formalWorkError("CONTROL_RESULT_CHANGED", "驗收後成果已變更，請重新核對。");
    let preview = this.previews.get(taskRef);
    if (preview && !await isOwnedResultPreview(preview)) {
      await closeResultPreview(preview);
      this.previews.delete(taskRef); preview = null;
    }
    if (!preview) { preview = { ...await startResultPreview(artifact), projectId, taskRef, resultDigest: detail.result_digest }; this.previews.set(taskRef, preview); }
    return { task_ref: taskRef, result_digest: detail.result_digest, url: preview.url };
  }
  async resultPreviewIdentity(url) {
    const preview = [...this.previews.values()].find((row) => `${row.url}/` === url
      && row.child.exitCode === null && row.child.signalCode === null);
    if (!preview || !await isOwnedResultPreview(preview)) throw formalWorkError("CONTROL_RESULT_PREVIEW_UNAVAILABLE", "這個成果入口已關閉，請從原工作重新開啟。");
    return { schema_version: "lattice.control.result-preview.v1", url,
      task_ref: preview.taskRef, result_digest: preview.resultDigest };
  }
  async reopenClaim(claim) {
    const resume = () => claim.turn_id
      ? this.codex.resumeThread(claim.thread_id, { expectedTurnId: claim.turn_id })
      : this.codex.resumeEmptyThread(claim.thread_id);
    try { await resume(); }
    catch (error) {
      if (error.code !== "CODEX_THREAD_ARCHIVED" || error.threadId !== claim.thread_id) throw error;
      await this.codex.unarchiveThread(claim.thread_id);
      await resume();
    }
    await this.observe(claim, "REOPENED", { summary: "已重開原對話，沒有重送執行回合。" });
  }
  action(projectId, taskRef, action, input = {}) {
    return this.serial(taskRef, async () => {
      if (action === "preview") return this.openResult(projectId, taskRef);
      if (action === "reconcile" || action === "verify") return this.reconcile(projectId, taskRef, { advance: action === "verify", resumePrepared: true });
      if (action === "answer") return this.answer(projectId, taskRef, input);
      const detail = await this.store.detail(projectId, taskRef);
      if (action === "archive" || action === "reopen") {
        for (const claim of detail.claims) {
          if (!claim.thread_id || claim.archived === (action === "archive")) continue;
          if (action === "archive") await this.codex.archiveThread(claim.thread_id);
          else { await this.reopenClaim(claim); continue; }
          await this.observe(claim, "ARCHIVED", { summary: "工作對話已封存，成果與身份保留。" });
        }
      } else if (action === "interrupt") {
        for (const claim of detail.claims) if (claim.turn_id && !terminal.has(claim.turn_status)) await this.codex.interruptTurn(claim.thread_id, claim.turn_id);
      } else if (action === "continue") {
        if (detail.completion_verified) throw formalWorkError("CONTROL_WORK_COMPLETED", "此成果已正式完成；新增需求請建立新工作。");
        const claim = detail.claims.find((row) => row.phase === (input.phase ?? "EXECUTION"));
        if (!claim?.thread_id || claim.archived || !terminal.has(claim.turn_status)) throw formalWorkError("CONTROL_WORK_NOT_READY", "請先核對並重開原工作回合。");
        if (typeof input.text !== "string" || !input.text.trim()) throw new TypeError("請描述需要補正的內容。");
        this.owners.set(claim.thread_id, { projectId, taskRef, claimId: claim.claim_id });
        try { await this.codex.resumeThread(claim.thread_id, { expectedTurnId: claim.turn_id }); }
        catch (error) {
          if (error.code === "CODEX_THREAD_ARCHIVED") await this.observe(claim, "ARCHIVED", { summary: "原對話已封存，補充內容尚未派送。" });
          throw error;
        }
        const queued = claim.pending_inputs?.find((row) => row.input_id === input.inputId);
        if (queued && queued.summary !== input.text) throw formalWorkError("CONTROL_INPUT_CONFLICT", "此補充訊息身份已用於不同內容。");
        if (!queued) await this.observe(claim, "INPUT_QUEUED", { turn_id: null, input_id: input.inputId, summary: input.text });
        await this.dispatch(claim, input.inputId, input.text);
      } else throw new TypeError("不支援的工作操作。");
      return this.store.detail(projectId, taskRef);
    });
  }
  async close() {
    this.closed = true;
    this.codex.off("notification", this.onNotification);
    this.codex.off("serverRequest", this.onRequest);
    await Promise.allSettled([...this.operations.values()]);
    await Promise.all([...this.previews.values()].map(closeResultPreview));
    this.previews.clear();
    this.deniedTurns.clear();
    await this.codex.close();
  }
}
