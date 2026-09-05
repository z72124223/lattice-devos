import assert from "node:assert/strict";
import test from "node:test";
import { FormalWorkStore, projectFormalWork } from "../src/formal-work-store.mjs";
import { openCircuitSummary } from "../src/execution-recovery.mjs";

const taskA = "a".repeat(64), taskB = "b".repeat(64);
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
  assert.equal(first.graph.nodes[1].blocker.status, "blocked");
  input.tasks[0].ledger.status = "COMPLETED";
  input.tasks[0].ledger.result_digest = "3".repeat(64);
  const verified = projectFormalWork([input]).snapshot;
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
