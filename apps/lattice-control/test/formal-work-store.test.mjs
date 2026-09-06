import assert from "node:assert/strict";
import test from "node:test";
import { FormalWorkStore, projectFormalWork } from "../src/formal-work-store.mjs";
import { openCircuitSummary } from "../src/execution-recovery.mjs";
import { workScope } from "../public/work-view.mjs";

const taskA = "a".repeat(64), taskB = "b".repeat(64);
test("a response for another project cannot populate the selected project's graph or details", async () => {
  const store = new FormalWorkStore({ runtime: { call: async () => page() } });
  await assert.rejects(store.readProject('project-b'), { code: 'CONTROL_WORK_PROJECT_MISMATCH' });
  await assert.rejects(store.detail('project-b', taskA), { code: 'CONTROL_WORK_PROJECT_MISMATCH' });
  assert.equal(store.cache.size, 0);
});
function page() {
  return { schema_version: "lattice.control.product-snapshot.v1", source: { authority: "POSTGRESQL_TASK_LEDGER" },
    project: { id: "project-a" }, revision: "1".repeat(64), next_task_ref: null,
    tasks: [taskA, taskB].map((task_ref) => ({ task_ref, objective: "可操作的清單",
      ledger: { project_id: "project-a", status: "SUBMITTED", result_digest: null, ledger_head_digest: "2".repeat(64) } })),
    product: { metadata: [{ task_ref: taskB, dependency_refs: [taskA], priority: 2 }], claims: [], observations: [], decisions: [] } };
}

test("a completed native reply with an open circuit stays visibly failed and does not satisfy dependencies", () => {
  const input = page();
  input.product.claims.push({ claim_id: "claim-a", task_ref: taskA, phase: "EXECUTION", turn_status: "TURN_COMPLETED" });
  input.product.observations.push({ claim_id: "claim-a", kind: "TURN_COMPLETED", summary: openCircuitSummary,
    observed_at: "2026-09-06T00:00:00Z" });
  const { rows, snapshot } = projectFormalWork([input]);
  assert.equal(rows[0].status, "failed");
  assert.equal(rows[0].progress, openCircuitSummary);
  assert.equal(rows[0].completion_verified, false);
  assert.equal(snapshot.graph.nodes[1].blocker.status, "blocked");
});

test("archive and model completion cannot satisfy a formal dependency", () => {
  const input = page();
  input.product.claims.push({ claim_id: "claim-a", task_ref: taskA, phase: "EXECUTION", archived: true, turn_status: "TURN_COMPLETED" });
  const first = projectFormalWork([input]).snapshot;
  assert.equal(first.tree.nodes[0].status, "archived");
  assert.equal(first.tree.nodes[0].completion_verified, false);
  assert.equal(workScope(first, null, "complete").counts.complete, 0);
  assert.equal(first.graph.nodes[1].blocker.status, "blocked");
  input.tasks[0].ledger.status = "COMPLETED";
  input.tasks[0].ledger.result_digest = "3".repeat(64);
  const verified = projectFormalWork([input]).snapshot;
  assert.equal(verified.tree.nodes[0].status, "archived");
  assert.equal(verified.tree.nodes[0].completion_verified, true);
  assert.equal(verified.graph.nodes[0].completion_verified, true);
  const completedView = workScope(verified, null, "complete");
  assert.equal(completedView.counts.complete, 1);
  assert.equal(completedView.tree.nodes[0].id, taskA);
  assert.equal(verified.graph.nodes[1].blocker.status, "clear");
  assert.equal(verified.tree.revision, verified.graph.revision);
  assert.equal(verified.tree.digest, verified.graph.digest);
  assert.notEqual(first.revision, verified.revision);
});

test("Runtime failure after invalidation cannot fall back to old local facts", async () => {
  let calls = 0;
  const store = new FormalWorkStore({ runtime: { call: async () => {
    if (++calls > 1) throw Object.assign(new Error("unavailable"), { code: "RUNTIME_UNAVAILABLE" });
    return page();
  } } });
  await store.getWorkSnapshot({ projectId: "project-a" });
  store.invalidate();
  await assert.rejects(store.getWorkSnapshot({ projectId: "project-a" }), { code: "RUNTIME_UNAVAILABLE" });
});

test("native multiline and long Unicode progress cannot break the work tree or imply completion", () => {
  for (const summary of [JSON.stringify({ summary: "檢查通過\n等待獨立驗收", artifact_path: "acceptance.mjs" }, null, 2),
    "檢查\r\n" + "進度✅".repeat(1400), "\u001b\u0000\u007f"]) {
    const input = page();
    input.product.claims.push({ claim_id: "claim-a", task_ref: taskA, phase: "EXECUTION", turn_status: "TURN_COMPLETED" });
    input.product.observations.push({ claim_id: "claim-a", kind: "TURN_COMPLETED", summary,
      observed_at: "2026-09-06T05:11:00Z" });
    const { snapshot, rows, facts } = projectFormalWork([input]);
    const work = snapshot.tree.nodes.find((node) => node.id === taskA);
    assert.equal(work.status, "codex_done");
    assert.equal(work.completion_verified, false);
    assert.ok(work.progress.length > 0 && Buffer.byteLength(work.progress) <= 4096);
    assert.doesNotMatch(work.progress, /[\u0000-\u001f\u007f-\u009f\ufffd]/u);
    assert.equal(rows[0].progress, summary.slice(0, 4096));
    assert.equal(facts.observations[0].summary, summary);
  }
});

test("node selection is bound to the same tree and graph identity", async () => {
  const store = new FormalWorkStore({ runtime: { call: async () => page() } });
  const snapshot = await store.getWorkSnapshot({ projectId: "project-a" });
  const node = await store.getWorkNode({ projectId: "project-a", workItemId: taskA,
    expectedRevision: snapshot.revision, expectedDigest: snapshot.digest });
  assert.equal(node.tree_node.id, node.graph_node.id);
  await assert.rejects(store.getWorkNode({ projectId: "project-a", workItemId: taskA,
    expectedRevision: "4".repeat(64), expectedDigest: snapshot.digest }), { code: "CONTROL_WORK_SNAPSHOT_CHANGED" });
});

test("project pages must have one authority and no duplicate task identities", () => {
  const first = page();
  assert.throws(() => projectFormalWork([first, first]), { code: "CONTROL_WORK_NODE_LIMIT_EXCEEDED" });
  const foreign = page(); foreign.project.id = "project-b";
  assert.throws(() => projectFormalWork([first, foreign]), { code: "CONTROL_WORK_AUTHORITY_REJECTED" });
});
