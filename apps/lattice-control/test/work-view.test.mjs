import test from 'node:test';
import assert from 'node:assert/strict';
import { layoutGraph, layoutTree, workAppearance, exampleSnapshot, workScope, graphBranches, treeBranches } from '../public/work-view.mjs';

test('branches open progressively and remember their state when a parent closes and reopens', () => {
  const choices = new Map(), tree = exampleSnapshot.tree;
  const initial = layoutTree(tree, treeBranches(tree, choices));
  assert.equal(initial.positions.size, tree.nodes.length, 'the first two levels start visible like the approved reference');
  choices.set('website', false); choices.set('payments', false);
  const folded = layoutTree(tree, treeBranches(tree, choices));
  assert.equal(folded.positions.has('login'), false);
  assert.equal(folded.positions.has('payment'), false);
  assert.ok(folded.positions.has('acceptance'), 'an unrelated branch stays open');
  choices.set('website', true);
  const opened = layoutTree(tree, treeBranches(tree, choices));
  for (const id of ['login', 'list', 'booking']) assert.ok(opened.positions.has(id));
  assert.equal(opened.positions.has('payment'), false, 'other branches remain folded');
  for (const id of ['goal', 'website', 'payments', 'delivery']) assert.deepEqual(opened.positions.get(id), initial.positions.get(id));
  choices.set('goal', false);
  assert.equal(layoutTree(tree, treeBranches(tree, choices)).positions.size, 1);
  choices.set('goal', true);
  assert.ok(layoutTree(tree, treeBranches(tree, choices)).positions.has('booking'));
  choices.set('website', false);
  assert.equal(layoutTree(tree, treeBranches(tree, choices)).positions.has('booking'), false);
  assert.equal(tree.nodes.length, 10, 'folding never deletes work');
  assert.ok(layoutTree(tree, treeBranches(tree, new Map(), true)).positions.has('launch'), 'status search can reveal matching paths');
});

test('graph folding preserves shared downstream work reached by a different open branch', () => {
  const choices = new Map(), graph = exampleSnapshot.graph;
  assert.deepEqual([...graphBranches(graph, choices).visible].sort(), ['requirements', 'login', 'list', 'payment'].sort());
  choices.set('login', true); choices.set('list', true);
  assert.ok(graphBranches(graph, choices).visible.has('booking'));
  choices.set('login', false);
  assert.ok(graphBranches(graph, choices).visible.has('booking'));
  choices.set('list', false);
  assert.equal(graphBranches(graph, choices).visible.has('booking'), false);
  choices.set('requirements', false);
  assert.deepEqual([...graphBranches(graph, choices).visible], ['requirements']);
  assert.equal(graphBranches(graph, new Map(), true).visible.size, graph.nodes.length);
});

test('status filters retain ancestors and count only actual verified completion', () => {
  const filtered = workScope(exampleSnapshot, null, 'complete');
  assert.equal(filtered.counts.complete, 1);
  assert.equal(filtered.counts.active, 3);
  assert.deepEqual([...filtered.matches].sort(), ['login']);
  assert.deepEqual(filtered.tree.nodes.map((work) => work.id).sort(), ['goal', 'login', 'website']);
  assert.deepEqual(filtered.tree.nodes.find((work) => work.id === 'website').children, ['login']);
  const nodes = [
    { id: 'archived', status: 'archived', completion_verified: false, children: [] },
    { id: 'finished', status: 'archived', completion_verified: true, children: [] },
    { id: 'replied', status: 'codex_done', children: [] },
  ];
  const state = workScope({ graph: { nodes }, tree: { nodes, roots: nodes.map((work) => work.id) } });
  assert.equal(state.counts.complete, 1);
  assert.equal(state.counts.archived, 1);
  assert.equal(state.counts.review, 1);
});

test('next and previous levels follow stored parents and isolate sibling branches', () => {
  const branch = workScope(exampleSnapshot, 'website');
  assert.deepEqual(branch.lineage.map((work) => work.id), ['goal', 'website']);
  assert.deepEqual(branch.children.map((work) => work.id), ['login', 'list', 'booking']);
  assert.equal(branch.counts.all, 4);
  assert.equal(branch.tree.nodes.some((work) => work.id === 'payment'), false);
  assert.deepEqual(branch.tree.roots, ['website']);
  const leaf = workScope(exampleSnapshot, 'login');
  assert.equal(leaf.lineage.at(-2).id, 'website');
  assert.equal(leaf.children.length, 0);
  assert.equal(leaf.counts.all, 1);
  assert.equal(workScope(exampleSnapshot, 'removed-node').counts.all, 10);
  assert.equal(workScope(exampleSnapshot, 'delivery', 'complete').matches.size, 0);
});

function assertVisibleWithoutOverlap(layout) {
  const boxes = [...layout.positions.values()];
  for (const [index, box] of boxes.entries()) {
    assert.ok(box.x >= 0 && box.y >= 0);
    assert.ok(box.x + box.width <= layout.width);
    assert.ok(box.y + box.height <= layout.height);
    for (const other of boxes.slice(index + 1)) {
      assert.ok(box.x + box.width <= other.x || other.x + other.width <= box.x
        || box.y + box.height <= other.y || other.y + other.height <= box.y, 'work cards must not overlap');
    }
  }
}

test('work graph keeps more than six unrelated and dependent tasks individually visible', () => {
  for (const count of [0, 1, 7, 11, 100]) {
    for (const connected of [false, true]) {
      const nodes = Array.from({ length: count }, (_, i) => ({ id: `task-${i}`, depends_on: connected && i ? [`task-${i - 1}`] : [] }));
      const layout = layoutGraph(nodes);
      assert.equal(layout.positions.size, count);
      assertVisibleWithoutOverlap(layout);
      for (const node of nodes) for (const dependency of node.depends_on) {
        assert.ok(layout.positions.get(dependency).x < layout.positions.get(node.id).x);
      }
    }
  }
});

test('tree uses actual children and preserves other branches when one is collapsed', () => {
  const full = layoutTree(exampleSnapshot.tree);
  assert.equal(full.positions.size, exampleSnapshot.tree.nodes.length);
  assert.equal(full.edges.length, exampleSnapshot.tree.nodes.length - 1);
  assertVisibleWithoutOverlap(full);
  const collapsed = layoutTree(exampleSnapshot.tree, new Set(['website']));
  for (const id of ['login', 'list', 'booking']) assert.equal(collapsed.positions.has(id), false);
  for (const id of ['website', 'payment', 'acceptance', 'launch']) assert.equal(collapsed.positions.has(id), true);
  const rootOnly = layoutTree(exampleSnapshot.tree, new Set(['goal']));
  assert.equal(rootOnly.positions.size, 1);
});

test('wide and deep real work trees stay within their canvas without overlapping', () => {
  const nodes = [];
  for (let branch = 0; branch < 11; branch++) {
    for (let depth = 0; depth < 7; depth++) {
      nodes.push({ id: `${branch}-${depth}`, children: depth === 6 ? [] : [`${branch}-${depth + 1}`] });
    }
  }
  const tree = { nodes, roots: Array.from({ length: 11 }, (_, i) => `${i}-0`) };
  const layout = layoutTree(tree);
  assert.equal(layout.positions.size, nodes.length + 1);
  assert.equal(layout.edges.length, nodes.length);
  assertVisibleWithoutOverlap(layout);
});

test('completion, approval, blockers and unknown states are not conflated', () => {
  assert.equal(workAppearance({ status: 'verified' }).label, '已完成');
  assert.equal(workAppearance({ status: 'draft' }).label, '待開始');
  assert.equal(workAppearance({ status: 'waiting_approval' }).label, '等你決定');
  assert.equal(workAppearance({ status: 'running', blocker: { status: 'blocked' } }).label, '遇到阻礙');
  assert.equal(workAppearance({ status: 'archived' }).label, '已封存');
  assert.equal(workAppearance({ status: 'codex_done' }).label, '等待驗收');
  assert.equal(workAppearance({ status: 'unknown' }).label, '狀態待確認');
});

test('invalid cycles fail visibly instead of hanging the browser', () => {
  assert.throws(() => layoutGraph([{ id: 'a', depends_on: ['b'] }, { id: 'b', depends_on: ['a'] }]), /循環/u);
  assert.throws(() => layoutTree({ roots: ['a'], nodes: [{ id: 'a', children: ['b'] }, { id: 'b', children: ['a'] }] }), /循環/u);
});
