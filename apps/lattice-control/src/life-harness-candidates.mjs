import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { isExecutionDenied, openCircuitSummary } from './execution-recovery.mjs';
import { elicitationDenied } from './mcp-tool-elicitation.mjs';

const prefix = 'LIFE_HARNESS_CANDIDATE_V1 ';
const id = value => typeof value === 'string' && /^[A-Za-z0-9._:-]{1,256}$/u.test(value);
const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const loopbackOrigin = origin => {
  let url;
  try { url = new URL(origin); } catch { throw new TypeError('Life-Harness requires a loopback HTTP origin'); }
  if (url.protocol !== 'http:' || url.hostname !== '127.0.0.1' || url.username || url.password
    || url.pathname !== '/' || url.search || url.hash) throw new TypeError('Life-Harness requires a loopback HTTP origin');
  return url.origin;
};
const stop = reason => { throw Object.assign(new Error(reason), { candidateReason: reason }); };
const quote = value => {
  if (typeof value !== 'string' || /['"\u0000-\u001f\u007f\u2018-\u201f$`!^]/u.test(value)) stop('COMMAND_UNREPRESENTABLE');
  return `'${value}'`;
};
const inside = (root, value) => {
  const relative = path.relative(root, value);
  return relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
};
export const failedCommand = item => item?.type === 'commandExecution' && item.status === 'failed'
  && Number.isSafeInteger(item.exitCode) && item.exitCode !== 0;

// This is only an event classifier. Static dependency, path and receipt checks
// remain in the existing helper; log text never becomes an executable command.
function classify(item) {
  if (!id(item.id) || typeof item.aggregatedOutput !== 'string' || Buffer.byteLength(item.aggregatedOutput) > 65536) return { reason: 'INCOMPLETE_FAILURE' };
  if (isExecutionDenied(item)) return { reason: 'DENIED' };
  const command = Array.isArray(item.commandActions) && item.commandActions.length === 1 ? item.commandActions[0]?.command : null;
  const node = typeof command === 'string' && command.length <= 8192
    && /^(?:&\s+)?(?:["'][^"'\r\n]*[\\/]node(?:\.exe)?["']|node(?:\.exe)?)\s/iu.test(command);
  if (!node) return { reason: 'UNSUPPORTED_COMMAND' };
  if (/Cannot find package\b/u.test(item.aggregatedOutput)) return { reason: 'THIRD_PARTY_PACKAGE' };
  const matches = [...item.aggregatedOutput.matchAll(/^Error \[ERR_MODULE_NOT_FOUND\]: Cannot find module '([^'\r\n]+)' imported from ([^\r\n]+)$/gmu)];
  if (matches.length !== 1 || !path.isAbsolute(matches[0][1]) || !path.isAbsolute(matches[0][2])) return { reason: 'UNSUPPORTED_FAILURE' };
  return { reason: 'NODE_RELATIVE_MODULE_CANDIDATE', target: matches[0][1], importer: matches[0][2], eligible: true };
}

export class LifeHarnessCandidates {
  constructor(service, { enabled = false, origin } = {}) {
    this.service = service;
    this.enabled = enabled === true;
    this.turns = new Map();
    this.inflight = new Set();
    if (!this.enabled) return;
    // Only trusted server composition supplies this function (listen(0) needs
    // its bound port). Resolve it once per event; HTTP input cannot replace it.
    this.origin = typeof origin === 'function' ? origin : loopbackOrigin(origin);
    this.client = fileURLToPath(new URL('./life-harness-client.mjs', import.meta.url));
  }

  handle(owner, { threadId, turnId, item }) {
    if (!this.enabled || this.service.closed || !failedCommand(item) || !id(item.id) || !id(threadId) || !id(turnId)) return;
    let origin;
    try { origin = loopbackOrigin(typeof this.origin === 'function' ? this.origin() : this.origin); } catch { return; }
    const binding = { projectId: owner.projectId, taskRef: owner.taskRef, claimId: owner.claimId, threadId, turnId, failureItemId: item.id };
    const key = hash([owner.projectId, owner.taskRef, owner.claimId, threadId, turnId]);
    let state = this.turns.get(key);
    if (!state) {
      for (const [oldKey, old] of this.turns) if (!this.service.codex.isTurnActive(old.threadId, old.turnId)) this.turns.delete(oldKey);
      // Never evict a live turn and accidentally reopen its notification budget.
      if (this.turns.size >= 256) return;
      state = { threadId, turnId, ids: new Set(), skipped: 0, candidate: false };
      this.turns.set(key, state);
    }
    if (state.ids.has(item.id)) return;
    const candidate = classify(item);
    if (candidate.eligible ? state.candidate : state.skipped >= 3) return;
    // Reserve before the first await: duplicate notifications and helper failures
    // cannot enqueue another candidate while source reads or native ACKs wait.
    state.ids.add(item.id);
    if (candidate.eligible) state.candidate = true; else state.skipped++;
    const codex = this.service.codex;
    const identity = { generation: codex.connectionGeneration, session: codex.appServerSessionId, requestSequence: codex.serverRequestSequence, started: Date.now() };
    const pending = this.run(binding, identity, candidate, origin).catch(() => {});
    this.inflight.add(pending);
    void pending.finally(() => this.inflight.delete(pending));
  }

  current(binding, identity, deadline) {
    const service = this.service, codex = service.codex, owner = service.owners.get(binding.threadId);
    if (Date.now() < identity.started || Date.now() >= deadline) stop('EXPIRED');
    if (service.closed || !codex.connected || codex.closePromise || codex.connectPromise
      || !Number.isSafeInteger(identity.generation) || identity.generation < 1
      || !/^app-server-session:sha256:[a-f0-9]{64}$/u.test(identity.session ?? '')
      || codex.connectionGeneration !== identity.generation || codex.appServerSessionId !== identity.session
      || owner?.projectId !== binding.projectId || owner.taskRef !== binding.taskRef || owner.claimId !== binding.claimId
      || !codex.isTurnActive(binding.threadId, binding.turnId)) stop('CURRENTNESS_CHANGED');
    if (codex.pendingServerRequestCount !== 0 || !Number.isSafeInteger(identity.requestSequence) || identity.requestSequence < 0
      || codex.serverRequestSequence !== identity.requestSequence) stop('PERMISSION_PENDING_OR_CHANGED');
    const notices = codex.notificationSnapshot?.({ threadId: binding.threadId, turnId: binding.turnId });
    if (!Array.isArray(notices) || (service.deniedTurns.get(`${binding.threadId}:${binding.turnId}`)?.size ?? 0) > 0
      || notices.some(row => row.message?.method === 'item/completed' && isExecutionDenied(row.message.params?.item))) stop('DENIED_OR_UNKNOWN');
  }

  claim(detail, binding, source) {
    const matches = detail?.claims?.filter(row => row.claim_id === binding.claimId), claim = matches?.length === 1 ? matches[0] : null;
    if (detail?.source?.kind !== 'POSTGRESQL_CONTROL_PRODUCT' || detail.source.authority !== 'POSTGRESQL_TASK_LEDGER'
      || detail.id !== binding.taskRef || detail.project_id !== binding.projectId || detail.project?.id !== binding.projectId
      || detail.project.active !== true || !id(detail.project.project_snapshot_id) || detail.status !== 'running' || detail.completion_verified !== false
      || !['SUBMITTED', 'RUNNING'].includes(detail.task?.ledger?.status) || detail.task.ledger.task_ref !== binding.taskRef
      || detail.task.ledger.project_id !== binding.projectId || detail.task.ledger.project_snapshot_id !== detail.project.project_snapshot_id
      || detail.task.ledger.blocker != null || detail.task.ledger.failure_code != null || !claim || claim.phase !== 'EXECUTION'
      || claim.task_ref !== binding.taskRef || claim.project_id !== binding.projectId || claim.thread_id !== binding.threadId
      || claim.turn_id !== binding.turnId || !id(claim.input_id) || claim.archived !== false || claim.dispatch_started !== true
      || claim.turn_status !== 'TURN_BOUND' || !Number.isSafeInteger(claim.dispatch_sequence) || claim.dispatch_sequence < 1
      || !Number.isSafeInteger(claim.last_sequence) || claim.last_sequence <= claim.dispatch_sequence
      || !Array.isArray(claim.pending_inputs) || claim.pending_inputs.length || !Array.isArray(claim.pending_questions) || claim.pending_questions.length
      || !Array.isArray(detail.product?.observations) || elicitationDenied(detail, claim)
      || detail.product.observations.some(row => row.claim_id === binding.claimId && row.turn_id === binding.turnId && row.summary === openCircuitSummary)
      || source && (source.binding.inputId !== claim.input_id || source.projectRoot !== claim.worktree_path)) stop('CLAIM_CHANGED_OR_INELIGIBLE');
    return claim;
  }

  async run(binding, identity, candidate, origin) {
    let source, reason = candidate.reason;
    if (candidate.eligible) {
      try {
        this.current(binding, identity, Date.now() + 5000);
        // diagnosticSource already owns the task queue. Never nest it inside serial.
        source = await this.service.diagnosticSource(binding.projectId, binding.taskRef, {
          claimId: binding.claimId, threadId: binding.threadId, turnId: binding.turnId, failureItemId: binding.failureItemId });
        const verified = classify(JSON.parse(source.failureJson));
        if (!verified.eligible || verified.importer !== candidate.importer || verified.target !== candidate.target) stop('SOURCE_CHANGED');
        if (!inside(source.projectRoot, candidate.importer) || !inside(source.projectRoot, candidate.target)) { source = null; reason = 'PATH_OUTSIDE_PROJECT'; }
      } catch (error) {
        source = null;
        reason = error.candidateReason ?? (/^CONTROL_DIAGNOSTIC_SOURCE_[A-Z_]{1,50}$/u.test(error.code ?? '')
          ? error.code.replace('CONTROL_DIAGNOSTIC_SOURCE_', 'SOURCE_') : 'SOURCE_REJECTED');
      }
    }
    const deadline = Math.min(Date.now() + 5000, source ? Date.parse(source.context.validUntil) : Infinity);
    let timer;
    const expired = new Promise((_, reject) => { timer = setTimeout(() => reject(Object.assign(new Error('EXPIRED'), { candidateReason: 'EXPIRED' })), Math.max(1, deadline - Date.now())); });
    const read = async action => { this.current(binding, identity, deadline); const result = await Promise.race([action(), expired]); this.current(binding, identity, deadline); return result; };
    try {
      await Promise.race([this.service.serial(binding.taskRef, async () => {
        const detail = await read(() => this.service.store.detail(binding.projectId, binding.taskRef));
        const claim = this.claim(detail, binding, source);
        const state = source ? 'steer_intent' : 'skipped';
        const requestId = `life-harness:${hash([binding.taskRef, binding.claimId, binding.threadId, binding.turnId, source ? 'candidate' : binding.failureItemId])}`;
        if (detail.product.observations.some(row => row.request_id === requestId)) return;
        const previousSequence = claim.last_sequence;
        // Runtime snapshots clip summaries to 256 characters. Keep a complete
        // typed projection; exact IDs remain in the native hint/source binding.
        const summary = prefix + JSON.stringify({ state, reason, failureIdHash: hash(binding.failureItemId), advisoryOnly: true, adopted: false });
        const observation = await read(() => this.service.observe(claim, 'PROGRESS', { request_id: requestId, summary }));
        if (!source || observation.sequence <= previousSequence) return;
        // Persisted intent is never replayed, including an unknown/lost native ACK.
        // This second fresh read also sees permissions or inputs changed by another caller.
        this.claim(await read(() => this.service.store.detail(binding.projectId, binding.taskRef)), binding, source);
        const args = ['--origin', origin, '--project', binding.projectId, '--task', binding.taskRef, '--claim', binding.claimId,
          '--thread', binding.threadId, '--turn', binding.turnId, '--failure', binding.failureItemId];
        const command = `node --experimental-vm-modules --disable-warning=ExperimentalWarning ${quote(this.client)} prepare `
          + args.map((value, index) => index % 2 ? quote(value) : value).join(' ');
        const text = `Life-Harness 已核對本回合的一個 Node 相對模組失敗候選。這是唯讀診斷提示，沒有授予新的修復、檔案寫入或權限繞過許可；仍遵守原任務與原生核准，拒絕或待核准時停止。若原任務允許保存本機診斷資料，請在目前原生回合用獨立 exec_command 精確執行下列命令，不改寫路徑或參數：\n${command}\nprepare 會重新核對來源並回傳 command 與 readbackCommand。先以另一個獨立 exec_command 原樣執行 command，成功後再以另一個獨立 exec_command 原樣執行 readbackCommand；不能包在子程序、eval、管線或重新造收據。讀回仍只是建議，依原任務決定是否採用。任何拒絕或失敗保留原證據並停止這條診斷，不重試、不重新啟動回合，不啟用 Jev。`;
        this.current(binding, identity, deadline);
        await this.service.codex.request('turn/steer', { threadId: binding.threadId, expectedTurnId: binding.turnId, input: [{ type: 'text', text }] },
          { expectedGeneration: identity.generation, expectedSessionId: identity.session, timeoutMs: Math.max(1, deadline - Date.now()) });
      }), expired]);
    } finally { clearTimeout(timer); }
  }
}
