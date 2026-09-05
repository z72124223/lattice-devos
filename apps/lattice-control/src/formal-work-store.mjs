import { createHash } from "node:crypto";
import { projectControlWorkSnapshot } from "./store.mjs";
import { LatticeRuntimeClient } from "./lattice-runtime-client.mjs";
import { runtimeHealthFromValue } from "./lattice-runtime-health.mjs";
import { openCircuitSummary } from "./execution-recovery.mjs";

const source = Object.freeze({ kind: "POSTGRESQL_CONTROL_PRODUCT", authority: "POSTGRESQL_TASK_LEDGER" });
const priorities = ["urgent", "high", "normal", "low"];
const digest = (value) => createHash("sha256").update(JSON.stringify(value)).digest("hex");
export function formalWorkError(code, message, status = 409) {
  return Object.assign(new Error(message), { code, status });
}
function assertIdentity(snapshot, revision, expectedDigest) {
  if (revision !== snapshot.revision || expectedDigest !== snapshot.digest) {
    throw formalWorkError("CONTROL_WORK_SNAPSHOT_CHANGED", "工作資料已更新，請重新載入後操作。");
  }
}
function pendingCount(claim) {
  return claim?.pending_questions?.length ?? claim?.pending_questions_count ?? 0;
}
function taskProjection(task, facts) {
  const metadata = facts.metadata.find((row) => row.task_ref === task.task_ref);
  const claims = facts.claims.filter((row) => row.task_ref === task.task_ref);
  const execution = claims.find((row) => row.phase === "EXECUTION");
  const verifier = claims.find((row) => row.phase === "VERIFICATION");
  const completed = task.ledger.status === "COMPLETED" && Boolean(task.ledger.result_digest);
  const latest = facts.observations.filter((row) => claims.some((claim) => claim.claim_id === row.claim_id))
    .sort((a, b) => String(a.observed_at).localeCompare(String(b.observed_at))).at(-1);
  const activeClaim = claims.find((claim) => ["DISPATCH_STARTED", "TURN_BOUND"].includes(claim.turn_status));
  const failedVerification = verifier?.verification_outcome?.kind === "VERIFICATION_FAILED"
    ? verifier.verification_outcome : null;
  let status = "draft";
  if (completed) status = "verified";
  else if (latest?.summary === openCircuitSummary) status = "failed";
  else if (claims.some((claim) => pendingCount(claim) > 0)) status = "waiting_approval";
  else if (activeClaim) status = activeClaim.turn_id ? "running" : "starting";
  else if (failedVerification) status = "failed";
  else if (claims.some((claim) => ["TURN_FAILED", "INTERRUPTED", "CLAIM_FAILED"].includes(claim.turn_status))) status = "failed";
  else if (execution?.turn_status === "TURN_COMPLETED") status = "codex_done";
  else if (execution) status = "starting";
  if (execution?.archived && (!verifier || verifier.archived)) status = "archived";
  return {
    id: task.task_ref, project_id: task.ledger.project_id,
    title: metadata?.title ?? task.objective.slice(0, 256), objective: task.objective,
    success_criteria: metadata?.success_criteria ?? null,
    priority: priorities[metadata?.priority ?? 2], status,
    completion_verified: completed, result_digest: task.ledger.result_digest,
    ledger_head_digest: task.ledger.ledger_head_digest,
    progress: completed ? "實際驗收與成果已正式保存。" : (!activeClaim && failedVerification?.summary)
      || (activeClaim && !activeClaim.turn_id ? "派送已保存；正在核對原生回合，尚未確認執行結果。" : latest?.summary?.slice(0, 4096))
      || (execution ? "已登記 Codex 執行；等待目前回合的實際狀態。" : "已正式登記；尚未執行。"),
    updated_at: latest?.observed_at ?? metadata?.updated_at ?? execution?.created_at ?? "尚未開始",
    claims, metadata: metadata ?? null,
  };
}

export function projectFormalWork(pages, { maxNodes = 256, maxEdges = 1024 } = {}) {
  const first = pages[0];
  if (!first || pages.some((page) => page.source?.authority !== source.authority
    || page.project.id !== first.project.id)) {
    throw formalWorkError("CONTROL_WORK_AUTHORITY_REJECTED", "無法核對正式工作資料來源。");
  }
  const facts = { metadata: [], claims: [], observations: [], decisions: [] };
  const tasks = [];
  for (const page of pages) {
    tasks.push(...page.tasks);
    for (const key of ["metadata", "claims", "observations"]) facts[key].push(...page.product[key]);
    facts.decisions = page.product.decisions;
  }
  if (tasks.length > maxNodes || new Set(tasks.map((task) => task.task_ref)).size !== tasks.length) {
    throw formalWorkError("CONTROL_WORK_NODE_LIMIT_EXCEEDED", "工作數量超過這次快照的範圍。");
  }
  const rows = tasks.map((task) => taskProjection(task, facts));
  const relations = rows.map((row) => ({ work_item_id: row.id,
    parent_work_item_id: row.metadata?.parent_ref ?? null, blocker_status: "clear", blocker_reason: null }));
  const dependencies = rows.flatMap((row) => (row.metadata?.dependency_refs ?? [])
    .map((dependency) => ({ work_item_id: row.id, depends_on_work_item_id: dependency })));
  if (dependencies.length + relations.filter((row) => row.parent_work_item_id).length > maxEdges) {
    throw formalWorkError("CONTROL_WORK_EDGE_LIMIT_EXCEEDED", "工作關聯超過這次快照的範圍。");
  }
  const snapshot = projectControlWorkSnapshot({ projectId: first.project.id, nodes: rows,
    relations, dependencies, source, sourceIdentity: pages.map((page) => page.revision) });
  return { project: first.project, snapshot, rows, facts,
    decisionsTruncated: pages.some((page) => page.product.truncation?.decisions === true) };
}

// This adapter has no local task database. Every durable edit goes through the
// existing Runtime; the cache only avoids repeating identical read projections.
export class FormalWorkStore {
  constructor({ runtime = new LatticeRuntimeClient(), cacheMs = 15000 } = {}) {
    this.runtime = runtime;
    this.cacheMs = cacheMs;
    this.cache = new Map();
    this.pending = new Map();
    this.generation = 0;
    this.decisionPresence = new Map();
  }
  invalidate() { this.generation += 1; this.cache.clear(); this.decisionPresence.clear(); }
  dataPresence(projectId) {
    const cached = this.cache.get(projectId);
    return { work: cached && cached.expires > Date.now() ? cached.value.rows.length > 0 : null,
      decisions: this.decisionPresence.get(projectId) ?? null };
  }
  async runtimeHealth({ signal } = {}) {
    if (signal?.aborted) return { postgresql: "STOPPED" };
    let onAbort;
    try {
      // The store owns this shared client; cancelling a health observation must
      // not close a connection also serving a task. Store shutdown drains it.
      const probe = this.runtime.call("lattice_runtime_status", {}).then(runtimeHealthFromValue)
        .catch(() => ({ postgresql: "UNREACHABLE", detail: "LATTICE_RUNTIME_UNREACHABLE" }));
      if (!signal) return await probe;
      return await Promise.race([probe, new Promise((resolve) => {
        onAbort = () => resolve({ postgresql: "STOPPED" });
        signal.addEventListener("abort", onAbort, { once: true });
      })]);
    } finally { if (onAbort) signal.removeEventListener("abort", onAbort); }
  }
  async readProject(projectId, { fresh = false } = {}) {
    const cached = this.cache.get(projectId);
    if (!fresh && cached && cached.expires > Date.now()) return cached.value;
    const generation = this.generation;
    const key = `${generation}:${projectId}`;
    if (this.pending.has(key)) return this.pending.get(key);
    const operation = (async () => {
      const pages = [];
      let cursor = null;
      do {
        const page = await this.runtime.call("lattice_control_snapshot", {
          project_id: projectId, ...(cursor ? { after_task_ref: cursor } : {}),
        });
        if (page.schema_version !== "lattice.control.product-snapshot.v1") {
          throw formalWorkError("CONTROL_WORK_SCHEMA_REJECTED", "正式工作資料版本無法核對。");
        }
        pages.push(page);
        if (page.next_task_ref && (pages.length >= 8 || page.next_task_ref === cursor)) {
          throw formalWorkError("CONTROL_WORK_NODE_LIMIT_EXCEEDED", "工作清單超過這次完整快照的範圍。");
        }
        cursor = page.next_task_ref;
      } while (cursor);
      const value = projectFormalWork(pages);
      if (this.generation === generation) this.cache.set(projectId, { value, expires: Date.now() + this.cacheMs });
      return value;
    })();
    this.pending.set(key, operation);
    try { return await operation; } finally { this.pending.delete(key); }
  }
  async detail(projectId, taskRef) {
    const page = await this.runtime.call("lattice_control_snapshot", { project_id: projectId, task_ref: taskRef });
    const task = page.tasks.find((row) => row.task_ref === taskRef);
    if (!task) throw formalWorkError("CONTROL_WORK_ITEM_NOT_FOUND", "找不到這項正式工作。", 404);
    return { ...taskProjection(task, page.product), task, product: page.product, project: page.project };
  }
  async getWorkSnapshot({ projectId, maxNodes = 256, maxEdges = 1024 }) {
    const { snapshot } = await this.readProject(projectId);
    if (snapshot.tree.nodes.length > maxNodes || snapshot.graph.nodes.reduce((sum, row) => sum + row.depends_on.length, 0)
      + snapshot.tree.nodes.filter((row) => row.parent_id).length > maxEdges) {
      throw formalWorkError("CONTROL_WORK_OUTPUT_LIMIT_EXCEEDED", "工作快照超過指定範圍。");
    }
    return snapshot;
  }
  async getWorkNode({ projectId, workItemId, expectedRevision, expectedDigest }) {
    const { snapshot } = await this.readProject(projectId);
    assertIdentity(snapshot, expectedRevision, expectedDigest);
    const treeNode = snapshot.tree.nodes.find((row) => row.id === workItemId);
    const graphNode = snapshot.graph.nodes.find((row) => row.id === workItemId);
    if (!treeNode || !graphNode) throw formalWorkError("CONTROL_WORK_ITEM_NOT_FOUND", "找不到這項正式工作。", 404);
    return { schema_version: "lattice.control.work-node.v1", source,
      project_id: projectId, revision: snapshot.revision, digest: snapshot.digest,
      tree_node: treeNode, graph_node: graphNode };
  }
  async decisions(projectId) {
    return this.getCurrentDecisionsPacket({ scope: projectId, limit: 32 });
  }
  async decisionHistory({ projectId, decisionId, expectedRevision, expectedDigest }) {
    const packet = await this.readDecision({ decisionId, maxDepth: 32, expectedRevision, expectedDigest });
    if (packet.decision.scope !== projectId) throw formalWorkError("CONTROL_DECISION_SCOPE_REJECTED", "決策不屬於目前專案。");
    return packet;
  }
  async decisionQuery(decisions, schema, maximumBytes) {
    const packet = await this.runtime.call("lattice_control_snapshot", { decisions });
    if (packet.schema_version !== `lattice.control.${schema}.v1`
      || packet.source?.authority !== source.authority || !Number.isSafeInteger(packet.revision)
      || packet.revision < 0 || !/^[a-f0-9]{64}$/u.test(packet.digest ?? "")
      || Buffer.byteLength(JSON.stringify(packet)) > maximumBytes) {
      throw formalWorkError("CONTROL_DECISION_PACKET_REJECTED", "決策資料未通過來源、版本與範圍核對。");
    }
    if (decisions.mode === "current" && !decisions.subject) this.decisionPresence.set(decisions.scope, packet.decisions.length > 0);
    return packet;
  }
  getCurrentDecisionsPacket({ scope, limit = 32, subject }) {
    return this.decisionQuery({ mode: "current", scope, limit, ...(subject ? { subject } : {}) },
      "current-decisions-packet", 262144);
  }
  readDecision({ decisionId, maxDepth = 32, expectedRevision, expectedDigest }) {
    return this.decisionQuery({ mode: "read", decision_id: decisionId, max_depth: maxDepth,
      revision: expectedRevision, digest: expectedDigest }, "decision-read", 524288);
  }
  searchDecisions({ scope, query, limit = 20, expectedRevision, expectedDigest }) {
    return this.decisionQuery({ mode: "search", scope, query, limit,
      revision: expectedRevision, digest: expectedDigest }, "decision-search", 196608);
  }
  async recordDecision({ scope, subject, content, rationale, source: decisionSource,
    clientRequestId, expectedRevision, expectedDigest, supersedesDecisionId }) {
    // Identity derives from the caller's retained key; a lost response cannot
    // create another decision. Runtime verifies exact semantics before replay.
    const packet = await this.update({ action: "DECISION", project_id: scope,
      decision_id: `decision:${digest([scope, clientRequestId])}`, client_request_id: clientRequestId,
      subject, content, reason: rationale, source: decisionSource,
      expected_revision: expectedRevision, expected_digest: expectedDigest,
      ...(supersedesDecisionId ? { supersedes_id: supersedesDecisionId } : {}) });
    if (packet.schema_version !== "lattice.control.decision-mutation.v1"
      || packet.source?.authority !== source.authority) {
      throw formalWorkError("CONTROL_DECISION_PACKET_REJECTED", "決策寫入回應無法核對；請沿用原請求身份重新查詢。");
    }
    return packet;
  }
  async questionResolution(projectId, taskRef, questionId) {
    const page = await this.runtime.call("lattice_control_snapshot", {
      project_id: projectId, task_ref: taskRef, question_id: questionId,
    });
    return page.product.question_resolution ?? null;
  }
  async update(command) {
    try { return await this.runtime.call("lattice_control_update", command); }
    finally { this.invalidate(); }
  }
  async submit(command) {
    try { return await this.runtime.call("lattice_task_submit", command); }
    finally { this.invalidate(); }
  }
  async close() { await this.runtime.close(); }
}
