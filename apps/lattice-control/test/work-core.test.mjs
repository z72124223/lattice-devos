import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { constants as sqliteConstants, DatabaseSync } from "node:sqlite";
import test from "node:test";
import { LatticeStore } from "../src/store.mjs";
import { ControlWorkService } from "../src/work-core-service.mjs";

test("Control schema v4 migrates work-core tables without losing work items or events", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-work-core-migration-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  try {
    store = new LatticeStore(databasePath);
    const project = store.createProject({ name: "Migration", rootPath: directory });
    const item = store.createWorkItem({
      projectId: project.id,
      title: "Preserve me",
      objective: "Retain the existing work item and its event.",
    });
    store.close();
    store = null;

    const legacy = new DatabaseSync(databasePath);
    legacy.exec(`
      DROP INDEX IF EXISTS work_items_project_id_id;
      DROP TABLE IF EXISTS work_item_dependencies;
      DROP TABLE IF EXISTS work_item_relations;
      PRAGMA user_version = 4;
    `);
    legacy.close();

    store = new LatticeStore(databasePath);
    assert.equal(store.database.prepare("PRAGMA user_version").get().user_version, 5);
    assert.equal(store.getWorkItem(item.id).title, "Preserve me");
    assert.deepEqual(store.listEvents(item.id).map(({ kind }) => kind), ["created"]);
    assert.deepEqual(
      store.database.prepare(`
        SELECT name FROM sqlite_master
        WHERE type = 'table' AND name IN ('work_item_dependencies', 'work_item_relations')
        ORDER BY name
      `).all().map(({ name }) => name),
      ["work_item_dependencies", "work_item_relations"],
    );
  } finally {
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("Control schema v2 migrates directly to v5 and preserves existing work", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-work-core-v2-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  try {
    store = new LatticeStore(databasePath);
    const project = store.createProject({ name: "V2 migration", rootPath: directory });
    const item = store.createWorkItem({
      projectId: project.id,
      title: "Existing V2 work",
      objective: "Survive the direct migration.",
    });
    store.close();
    store = null;

    const legacy = new DatabaseSync(databasePath);
    legacy.exec(`
      DROP INDEX IF EXISTS work_items_project_id_id;
      DROP TABLE work_item_dependencies;
      DROP TABLE work_item_relations;
      DROP TABLE conversation_writer_leases;
      PRAGMA user_version = 2;
    `);
    legacy.close();

    store = new LatticeStore(databasePath);
    assert.equal(store.database.prepare("PRAGMA user_version").get().user_version, 5);
    assert.equal(store.getWorkItem(item.id).objective, "Survive the direct migration.");
    assert.deepEqual(store.listEvents(item.id).map(({ kind }) => kind), ["created"]);
  } finally {
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("one Control SQLite snapshot derives tree and graph with one revision and digest", () => {
  const store = new LatticeStore();
  const service = new ControlWorkService({ store });
  try {
    const project = store.createProject({ name: "Work core", rootPath: process.cwd() });
    const goal = store.createWorkItem({
      projectId: project.id,
      title: "Ship the work core",
      objective: "Deliver one bounded read model.",
      priority: "high",
    });
    const prerequisite = store.createWorkItem({
      projectId: project.id,
      title: "Verify migration",
      objective: "Prove old rows survive.",
    });
    const child = store.createWorkItem({
      projectId: project.id,
      title: "Expose MCP reads",
      objective: "Read the same work state through MCP.",
    });

    const initial = service.workSnapshot({
      projectId: project.id,
      maxNodes: 10,
      maxEdges: 20,
    });
    service.setWorkRelations({
      projectId: project.id,
      workItemId: child.id,
      parentId: goal.id,
      dependsOn: [prerequisite.id],
      blocker: { status: "blocked", reason: "Waiting for independent review" },
      expectedRevision: initial.revision,
      expectedDigest: initial.digest,
    });

    const snapshot = service.workSnapshot({
      projectId: project.id,
      maxNodes: 10,
      maxEdges: 20,
    });
    assert.equal(snapshot.schema_version, "lattice.control.work-snapshot.v1");
    assert.deepEqual(snapshot.source, {
      kind: "CONTROL_SQLITE_WORK_ITEMS",
      authority: "CONTROL_LOCAL_PRODUCT_STATE",
    });
    assert.match(snapshot.revision, /^[a-f0-9]{64}$/u);
    assert.match(snapshot.digest, /^[a-f0-9]{64}$/u);
    assert.equal(snapshot.tree.revision, snapshot.revision);
    assert.equal(snapshot.tree.digest, snapshot.digest);
    assert.equal(snapshot.graph.revision, snapshot.revision);
    assert.equal(snapshot.graph.digest, snapshot.digest);

    const treeChild = snapshot.tree.nodes.find(({ id }) => id === child.id);
    const treeGoal = snapshot.tree.nodes.find(({ id }) => id === goal.id);
    assert.equal(treeChild.parent_id, goal.id);
    assert.deepEqual(treeGoal.children, [child.id]);
    assert.ok(snapshot.tree.roots.includes(goal.id));

    const graphChild = snapshot.graph.nodes.find(({ id }) => id === child.id);
    const graphPrerequisite = snapshot.graph.nodes.find(({ id }) => id === prerequisite.id);
    assert.deepEqual(graphChild.depends_on, [prerequisite.id]);
    assert.deepEqual(graphPrerequisite.reverse_dependents, [child.id]);
    assert.deepEqual(graphChild.blocker, {
      status: "blocked",
      reasons: [
        { kind: "explicit", reason: "Waiting for independent review" },
        { kind: "dependency", work_item_id: prerequisite.id, status: "draft" },
      ],
    });

    const node = service.workNode({
      projectId: project.id,
      workItemId: child.id,
      expectedRevision: snapshot.revision,
      expectedDigest: snapshot.digest,
      maxNodes: 10,
      maxEdges: 20,
    });
    assert.deepEqual(node.tree_node, treeChild);
    assert.deepEqual(node.graph_node, graphChild);
  } finally {
    store.close();
  }
});

test("work relation mutation rejects invalid, stale, cyclic, duplicate, and unbounded state", () => {
  const store = new LatticeStore();
  const service = new ControlWorkService({ store });
  try {
    const project = store.createProject({ name: "Guarded", rootPath: process.cwd() });
    const otherProject = store.createProject({ name: "Other", rootPath: process.cwd() });
    const first = store.createWorkItem({
      projectId: project.id,
      title: "First",
      objective: "First guarded node.",
    });
    const second = store.createWorkItem({
      projectId: project.id,
      title: "Second",
      objective: "Second guarded node.",
    });
    const third = store.createWorkItem({
      projectId: project.id,
      title: "Third",
      objective: "Third guarded node.",
    });
    const foreign = store.createWorkItem({
      projectId: otherProject.id,
      title: "Foreign",
      objective: "Must never cross the project boundary.",
    });
    const current = () => service.workSnapshot({
      projectId: project.id,
      maxNodes: 10,
      maxEdges: 20,
    });
    const set = (input, identity = current()) => service.setWorkRelations({
      projectId: project.id,
      workItemId: input.workItemId,
      parentId: input.parentId ?? null,
      dependsOn: input.dependsOn ?? [],
      blocker: input.blocker ?? { status: "clear" },
      expectedRevision: identity.revision,
      expectedDigest: identity.digest,
    });

    assert.throws(
      () => set({ workItemId: first.id, parentId: first.id }),
      (error) => error.code === "CONTROL_WORK_SELF_PARENT_REJECTED",
    );
    assert.throws(
      () => set({ workItemId: first.id, dependsOn: [first.id] }),
      (error) => error.code === "CONTROL_WORK_SELF_DEPENDENCY_REJECTED",
    );
    assert.throws(
      () => set({ workItemId: first.id, dependsOn: [second.id, second.id] }),
      (error) => error.code === "CONTROL_WORK_DUPLICATE_DEPENDENCY_REJECTED",
    );
    assert.throws(
      () => set({ workItemId: first.id, parentId: "missing-node" }),
      (error) => error.code === "CONTROL_WORK_NODE_NOT_FOUND",
    );
    assert.throws(
      () => set({ workItemId: "missing-node" }),
      (error) => error.code === "CONTROL_WORK_NODE_NOT_FOUND",
    );
    assert.throws(
      () => set({ workItemId: first.id, parentId: foreign.id }),
      (error) => error.code === "CONTROL_WORK_CROSS_PROJECT_REJECTED",
    );
    assert.throws(
      () => set({ workItemId: foreign.id }),
      (error) => error.code === "CONTROL_WORK_CROSS_PROJECT_REJECTED",
    );

    set({ workItemId: second.id, parentId: first.id });
    assert.throws(
      () => set({ workItemId: first.id, parentId: second.id }),
      (error) => error.code === "CONTROL_WORK_HIERARCHY_CYCLE_REJECTED",
    );
    assert.equal(current().tree.nodes.find(({ id }) => id === first.id).parent_id, null);
    set({ workItemId: second.id });

    set({ workItemId: second.id, dependsOn: [first.id] });
    assert.throws(
      () => set({ workItemId: first.id, dependsOn: [second.id] }),
      (error) => error.code === "CONTROL_WORK_DEPENDENCY_CYCLE_REJECTED",
    );
    assert.deepEqual(
      current().graph.nodes.find(({ id }) => id === first.id).depends_on,
      [],
    );

    const beforeThird = current();
    store.updateWorkItem(third.id, { progress: "A concurrent change" });
    assert.throws(
      () => set({ workItemId: third.id, parentId: first.id }, beforeThird),
      (error) => error.code === "CONTROL_WORK_REVISION_MISMATCH",
    );

    const identity = current();
    const applied = set({
      workItemId: third.id,
      blocker: { status: "blocked", reason: "Needs an approval" },
    }, identity);
    assert.equal(applied.changed, true);
    const eventCount = store.listEvents(third.id).length;
    assert.throws(
      () => set({
        workItemId: third.id,
        blocker: { status: "blocked", reason: "Needs an approval" },
      }, identity),
      (error) => error.code === "CONTROL_WORK_REVISION_MISMATCH",
    );
    const replayed = set({
      workItemId: third.id,
      blocker: { status: "blocked", reason: "Needs an approval" },
    });
    assert.equal(replayed.changed, false);
    assert.equal(store.listEvents(third.id).length, eventCount);

    set({
      workItemId: third.id,
      parentId: first.id,
      dependsOn: [second.id],
      blocker: { status: "blocked", reason: "Needs an approval" },
    });

    assert.throws(
      () => service.workSnapshot({ projectId: project.id, maxNodes: 2, maxEdges: 20 }),
      (error) => error.code === "CONTROL_WORK_NODE_LIMIT_EXCEEDED",
    );
    assert.throws(
      () => service.workSnapshot({ projectId: project.id, maxNodes: 10, maxEdges: 1 }),
      (error) => error.code === "CONTROL_WORK_EDGE_LIMIT_EXCEEDED",
    );
    assert.throws(
      () => service.workNode({
        projectId: project.id,
        workItemId: third.id,
        expectedRevision: "0".repeat(64),
        expectedDigest: current().digest,
        maxNodes: 10,
        maxEdges: 20,
      }),
      (error) => error.code === "CONTROL_WORK_REVISION_MISMATCH",
    );

    assert.throws(
      () => store.database.prepare(`
        INSERT INTO work_item_dependencies (
          work_item_id, depends_on_work_item_id, created_at
        ) VALUES (?, ?, ?)
      `).run(first.id, "missing-node", new Date().toISOString()),
      /FOREIGN KEY constraint failed/u,
    );
    assert.throws(
      () => store.database.prepare(`
        INSERT INTO work_item_dependencies (
          work_item_id, depends_on_work_item_id, created_at
        ) VALUES (?, ?, ?)
      `).run(second.id, first.id, new Date().toISOString()),
      /UNIQUE constraint failed/u,
    );

    store.database.exec("PRAGMA ignore_check_constraints = ON;");
    store.database.prepare(`
      INSERT INTO work_item_relations (
        work_item_id, parent_work_item_id, blocker_status, blocker_reason
      ) VALUES (?, NULL, 'clear', 'SHOULD_HAVE_FAILED_CLOSED')
    `).run(first.id);
    store.database.exec("PRAGMA ignore_check_constraints = OFF;");
    assert.throws(
      () => current(),
      (error) => error.code === "CONTROL_WORK_BLOCKER_REJECTED",
    );
    store.database.prepare(
      "DELETE FROM work_item_relations WHERE work_item_id = ?",
    ).run(first.id);

    store.database.prepare(`
      INSERT INTO work_item_dependencies (
        work_item_id, depends_on_work_item_id, created_at
      ) VALUES (?, ?, ?)
    `).run(foreign.id, first.id, new Date().toISOString());
    assert.throws(
      () => current(),
      (error) => error.code === "CONTROL_WORK_CROSS_PROJECT_REJECTED",
    );
    store.database.prepare(`
      DELETE FROM work_item_dependencies
      WHERE work_item_id = ? AND depends_on_work_item_id = ?
    `).run(foreign.id, first.id);

    store.ensurePrimaryConversation(project.id);
    store.database.prepare(`
      INSERT INTO work_item_dependencies (
        work_item_id, depends_on_work_item_id, created_at
      ) VALUES ('primary', ?, ?)
    `).run(first.id, new Date().toISOString());
    assert.throws(
      () => current(),
      (error) => error.code === "CONTROL_WORK_PRIMARY_CONVERSATION_REJECTED",
    );
    store.database.prepare(
      "DELETE FROM work_item_dependencies WHERE work_item_id = 'primary'",
    ).run();
    store.database.prepare(`
      INSERT INTO work_item_relations (
        work_item_id, parent_work_item_id, blocker_status, blocker_reason
      ) VALUES ('primary', NULL, 'blocked', 'NOT_A_WORK_CORE_NODE')
    `).run();
    assert.throws(
      () => current(),
      (error) => error.code === "CONTROL_WORK_PRIMARY_CONVERSATION_REJECTED",
    );
    store.database.prepare(
      "DELETE FROM work_item_relations WHERE work_item_id = 'primary'",
    ).run();
  } finally {
    store.close();
  }
});

test("work snapshot stays on one SQLite revision while another Control connection writes", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-work-core-snapshot-"));
  const databasePath = path.join(directory, "control.db");
  let reader;
  let writer;
  try {
    reader = new LatticeStore(databasePath);
    const readerService = new ControlWorkService({ store: reader });
    const project = reader.createProject({ name: "Snapshot", rootPath: directory });
    const prerequisite = reader.createWorkItem({
      projectId: project.id,
      title: "Prerequisite",
      objective: "Starts as draft.",
    });
    const dependent = reader.createWorkItem({
      projectId: project.id,
      title: "Dependent",
      objective: "Must see one dependency status.",
    });
    const initial = readerService.workSnapshot({ projectId: project.id });
    readerService.setWorkRelations({
      projectId: project.id,
      workItemId: dependent.id,
      dependsOn: [prerequisite.id],
      expectedRevision: initial.revision,
      expectedDigest: initial.digest,
    });
    writer = new LatticeStore(databasePath);
    let wroteDuringRead = false;
    reader.database.setAuthorizer((actionCode, tableName) => {
      if (
        !wroteDuringRead
        && actionCode === sqliteConstants.SQLITE_READ
        && tableName === "work_item_dependencies"
      ) {
        wroteDuringRead = true;
        writer.updateWorkItem(prerequisite.id, { status: "verified" });
      }
      return sqliteConstants.SQLITE_OK;
    });

    const oldSnapshot = readerService.workSnapshot({ projectId: project.id });
    reader.database.setAuthorizer(null);
    assert.equal(wroteDuringRead, true);
    assert.deepEqual(
      oldSnapshot.graph.nodes.find(({ id }) => id === dependent.id).blocker.reasons,
      [{ kind: "dependency", work_item_id: prerequisite.id, status: "draft" }],
    );
    const newSnapshot = readerService.workSnapshot({ projectId: project.id });
    assert.notEqual(newSnapshot.revision, oldSnapshot.revision);
    assert.deepEqual(
      newSnapshot.graph.nodes.find(({ id }) => id === dependent.id).blocker,
      { status: "clear", reasons: [] },
    );
  } finally {
    reader?.database.setAuthorizer(null);
    writer?.close();
    reader?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("bounded work reads use project and endpoint indexes despite a large unrelated graph", () => {
  const store = new LatticeStore();
  const service = new ControlWorkService({ store });
  try {
    const project = store.createProject({ name: "Bounded", rootPath: process.cwd() });
    const item = store.createWorkItem({
      projectId: project.id,
      title: "Small project node",
      objective: "Stay independent of unrelated Control work.",
    });
    const before = service.workSnapshot({ projectId: project.id, maxNodes: 1, maxEdges: 1 });

    const unrelatedProject = store.createProject({
      name: "Unrelated graph",
      rootPath: process.cwd(),
    });
    const unrelated = [];
    for (let index = 0; index < 300; index += 1) {
      unrelated.push(store.createWorkItem({
        projectId: unrelatedProject.id,
        title: `Unrelated ${index}`,
        objective: "This row must not expand another project's read work.",
      }));
    }
    const insertDependency = store.database.prepare(`
      INSERT INTO work_item_dependencies (
        work_item_id, depends_on_work_item_id, created_at
      ) VALUES (?, ?, ?)
    `);
    for (let index = 1; index < unrelated.length; index += 1) {
      insertDependency.run(
        unrelated[index].id,
        unrelated[index - 1].id,
        new Date().toISOString(),
      );
    }

    const after = service.workSnapshot({ projectId: project.id, maxNodes: 1, maxEdges: 1 });
    assert.equal(after.revision, before.revision);
    assert.equal(after.digest, before.digest);
    assert.deepEqual(after.graph.nodes.map(({ id }) => id), [item.id]);

    const assertIndexed = (sql, args, indexName) => {
      const details = store.database.prepare(`EXPLAIN QUERY PLAN ${sql}`)
        .all(...args)
        .map(({ detail }) => detail)
        .join("\n");
      assert.match(details, new RegExp(indexName, "u"));
      assert.doesNotMatch(details, /\bSCAN\b/u);
    };
    assertIndexed(`
      SELECT id FROM work_items
      WHERE project_id = ? AND id <> 'primary'
      ORDER BY id ASC LIMIT ?
    `, [project.id, 2], "work_items_project_id_id");
    assertIndexed(`
      SELECT work_item_id FROM work_item_relations
      WHERE work_item_id IN (?) ORDER BY work_item_id LIMIT ?
    `, [item.id, 2], "sqlite_autoindex_work_item_relations_1");
    assertIndexed(`
      SELECT child.id
      FROM work_item_relations AS relations
      LEFT JOIN work_items AS child ON child.id = relations.work_item_id
      WHERE relations.parent_work_item_id IN (?)
        AND (child.id IS NULL OR child.project_id <> ?)
      LIMIT 1
    `, [item.id, project.id], "work_item_relations_parent");
    assertIndexed(`
      SELECT work_item_id FROM work_item_dependencies
      WHERE work_item_id IN (?)
      ORDER BY work_item_id, depends_on_work_item_id LIMIT ?
    `, [item.id, 2], "sqlite_autoindex_work_item_dependencies_1");
    assertIndexed(`
      SELECT child.id
      FROM work_item_dependencies AS dependencies
      LEFT JOIN work_items AS child ON child.id = dependencies.work_item_id
      WHERE dependencies.depends_on_work_item_id IN (?)
        AND (child.id IS NULL OR child.project_id <> ?)
      LIMIT 1
    `, [item.id, project.id], "work_item_dependencies_reverse");
  } finally {
    store.close();
  }
});
