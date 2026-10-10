import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, writeFile, readFile, lstat, rm, symlink, link, rename, open } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { previewControlCodeGraphPurge, validateControlCodeGraphPurge, readbackControlCodeGraphPurge } from '../src/project-purge-code-graph.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const hash = value => createHash('sha256').update(value).digest('hex');
async function fixture(t) {
  const base = await mkdtemp(path.join(here, '.project-purge-code-graph-fixture-'));
  t.after(async () => {
    assert.equal(path.dirname(path.resolve(base)), here);
    assert.ok(path.basename(base).startsWith('.project-purge-code-graph-fixture-'));
    await rm(base, { recursive: true, force: true });
  });
  const cacheDirectory = path.join(base, 'cache');
  return { base, cacheDirectory, projectId: 'synthetic-project-a' };
}
async function cache(f, { projectId = f.projectId, sourceRoot = path.join(f.base, 'checkout-a'), patch = {}, key, padding = 0 } = {}) {
  // The checkout deliberately need not exist: old derived caches remain owned.
  key ??= hash(`${projectId}\0${sourceRoot.toLocaleLowerCase()}`);
  const root = path.join(f.cacheDirectory, key);
  const graph = { schema_version: 'lattice.control.code-graph.v1', source: 'GRAPHIFY', authority: 'DERIVED',
    commit: 'a'.repeat(40), nodes: [], edges: [], project_id: projectId, source_root: sourceRoot,
    generated_at: '2026-10-10T00:00:00Z', ...patch };
  const cacheDigest = hash(JSON.stringify(graph)), filename = path.join(root, 'graph.json');
  await mkdir(path.join(root, 'runs'), { recursive: true });
  const content = JSON.stringify({ ...graph, cache_digest: cacheDigest });
  if (padding) {
    const buffer = Buffer.alloc(padding, ' '); buffer.write(content);
    await writeFile(filename, buffer);
  } else await writeFile(filename, content);
  return { root, filename, graph, binding: { root, sourceRoot, key, cacheDigest } };
}
const hasBlocker = (result, code) => result.blockers.some(blocker => blocker.code === code);

test('discovers exact target cache roots across checkouts and preserves another project', async t => {
  const f = await fixture(t), a = await cache(f), a2 = await cache(f, { sourceRoot: path.join(f.base, 'checkout-two') });
  const b = await cache(f, { projectId: 'synthetic-project-b' });
  const before = await readFile(b.filename);
  const plan = await previewControlCodeGraphPurge(f);
  assert.equal(plan.discovery, 'COMPLETE'); assert.deepEqual(plan.blockers, []);
  assert.deepEqual(plan.roots, [a.root, a2.root].sort());
  assert.deepEqual(plan.bindings, [a.binding, a2.binding].sort((x, y) => x.root < y.root ? -1 : 1));
  assert.equal((await validateControlCodeGraphPurge(plan)).valid, true);
  assert.equal((await readbackControlCodeGraphPurge(plan)).complete, false);
  await rm(a.root, { recursive: true }); await rm(a2.root, { recursive: true });
  assert.deepEqual((await validateControlCodeGraphPurge(plan)).absentRoots, plan.roots);
  assert.equal((await readbackControlCodeGraphPurge(plan)).complete, true);
  assert.deepEqual(await readFile(b.filename), before);
});

test('an absent cache and missing ancestors are observed without creating anything', async t => {
  const f = await fixture(t); f.cacheDirectory = path.join(f.base, 'missing', 'cache');
  const plan = await previewControlCodeGraphPurge(f);
  assert.equal(plan.discovery, 'COMPLETE'); assert.deepEqual(plan.roots, []);
  assert.equal((await validateControlCodeGraphPurge(plan)).valid, true);
  assert.equal((await readbackControlCodeGraphPurge(plan)).complete, true);
  await assert.rejects(lstat(path.join(f.base, 'missing')), { code: 'ENOENT' });
  await cache(f);
  await assert.rejects(validateControlCodeGraphPurge(plan), { code: 'PURGE_CODE_GRAPH_SCOPE_CHANGED' });
  assert.equal((await readbackControlCodeGraphPurge(plan)).complete, false);
});

for (const [name, patch] of [
  ['schema', { schema_version: 'unrelated.v1' }], ['source', { source: 'OTHER' }],
  ['authority', { authority: 'AUTHORITATIVE' }], ['graph shape', { edges: [{ source: 'missing' }] }],
  ['relative source root', { source_root: 'relative/path' }], ['invalid project', { project_id: '' }],
]) {
  test(`rejects ${name} even with a matching content digest`, async t => {
    const f = await fixture(t); await cache(f, { patch });
    const result = await previewControlCodeGraphPurge(f);
    assert.equal(result.discovery, 'BLOCKED'); assert.deepEqual(result.roots, []);
    assert.ok(hasBlocker(result, 'PURGE_CODE_GRAPH_MANIFEST_INVALID'));
  });
}

test('directory name alone cannot authorize an owner or a different checkout', async t => {
  for (const patch of [{ project_id: 'synthetic-project-b' }, { source_root: path.join(here, 'another-checkout') }]) {
    const f = await fixture(t); await cache(f, { patch });
    const result = await previewControlCodeGraphPurge(f);
    assert.equal(result.discovery, 'BLOCKED'); assert.deepEqual(result.roots, []);
    assert.ok(hasBlocker(result, 'PURGE_CODE_GRAPH_OWNER_MISMATCH'));
  }
});

test('content tampering and unknown entries block discovery rather than imply no target', async t => {
  const f = await fixture(t), a = await cache(f);
  const parsed = JSON.parse(await readFile(a.filename, 'utf8')); parsed.generated_at = 'tampered';
  await writeFile(a.filename, JSON.stringify(parsed));
  await writeFile(path.join(f.cacheDirectory, 'unrecognized.txt'), 'synthetic');
  const result = await previewControlCodeGraphPurge(f);
  assert.equal(result.discovery, 'BLOCKED'); assert.deepEqual(result.roots, []);
  assert.ok(hasBlocker(result, 'PURGE_CODE_GRAPH_DIGEST_MISMATCH'));
  assert.ok(hasBlocker(result, 'PURGE_CODE_GRAPH_UNKNOWN_ENTRY'));
  await assert.rejects(validateControlCodeGraphPurge(result), { code: 'PURGE_CODE_GRAPH_PLAN_INVALID' });
});

test('failed analysis without graph.json remains unknown even when the folder matches the target key', async t => {
  const f = await fixture(t), a = await cache(f); await rm(a.filename);
  const result = await previewControlCodeGraphPurge(f);
  assert.equal(result.discovery, 'BLOCKED'); assert.deepEqual(result.roots, []);
  assert.ok(hasBlocker(result, 'PURGE_CODE_GRAPH_MANIFEST_MISSING'));
});

test('validate rejects new target caches and changed planned graph bindings', async t => {
  const f = await fixture(t); await cache(f);
  const plan = await previewControlCodeGraphPurge(f);
  const extra = await cache(f, { sourceRoot: path.join(f.base, 'new-checkout') });
  for (const allowPlannedPartial of [false, true]) {
    await assert.rejects(validateControlCodeGraphPurge(plan, { allowPlannedPartial }), { code: 'PURGE_CODE_GRAPH_SCOPE_CHANGED' });
  }
  await rm(extra.root, { recursive: true });
  await cache(f, { patch: { generated_at: '2026-10-11T00:00:00Z' } });
  await assert.rejects(validateControlCodeGraphPurge(plan), { code: 'PURGE_CODE_GRAPH_SCOPE_CHANGED' });
});

test('partial resume skips only planned headers; readback requires every planned root to disappear', async t => {
  const f = await fixture(t), a = await cache(f), plan = await previewControlCodeGraphPurge(f);
  await rm(a.filename);
  await assert.rejects(validateControlCodeGraphPurge(plan), { code: 'PURGE_CODE_GRAPH_BLOCKED' });
  assert.equal((await validateControlCodeGraphPurge(plan, { allowPlannedPartial: true })).valid, true);
  let readback = await readbackControlCodeGraphPurge(plan);
  assert.equal(readback.complete, false); assert.deepEqual(readback.remainingPlannedRoots, [a.root]);
  const unknown = path.join(f.cacheDirectory, 'b'.repeat(64)); await mkdir(unknown);
  await assert.rejects(validateControlCodeGraphPurge(plan, { allowPlannedPartial: true }), { code: 'PURGE_CODE_GRAPH_BLOCKED' });
  await rm(a.root, { recursive: true });
  readback = await readbackControlCodeGraphPurge(plan);
  assert.equal(readback.complete, false); assert.deepEqual(readback.remainingPlannedRoots, []);
  assert.ok(hasBlocker(readback, 'PURGE_CODE_GRAPH_MANIFEST_MISSING'));
  await rm(unknown, { recursive: true });
  assert.equal((await readbackControlCodeGraphPurge(plan)).complete, true);
});

test('planned root replacement with a junction is rejected even during partial resume', async t => {
  const f = await fixture(t), a = await cache(f), plan = await previewControlCodeGraphPurge(f);
  const saved = path.join(f.base, 'saved-cache'); await rename(a.root, saved);
  await symlink(saved, a.root, process.platform === 'win32' ? 'junction' : 'dir');
  const result = await previewControlCodeGraphPurge(f);
  assert.ok(hasBlocker(result, 'PURGE_CODE_GRAPH_UNSAFE_DIRECTORY'));
  await assert.rejects(validateControlCodeGraphPurge(plan, { allowPlannedPartial: true }), { code: 'PURGE_CODE_GRAPH_BLOCKED' });
  assert.equal((await readbackControlCodeGraphPurge(plan)).complete, false);
  assert.ok((await readFile(path.join(saved, 'graph.json'), 'utf8')).includes('GRAPHIFY'));
});

test('cache-directory links and ancestor links never establish even absent descendant scope', async t => {
  const f = await fixture(t), target = path.join(f.base, 'elsewhere'); await mkdir(target);
  const alias = path.join(f.base, 'alias'); await symlink(target, alias, process.platform === 'win32' ? 'junction' : 'dir');
  for (const cacheDirectory of [alias, path.join(alias, 'missing-cache')]) {
    const result = await previewControlCodeGraphPurge({ ...f, cacheDirectory });
    assert.equal(result.discovery, 'BLOCKED');
    assert.ok(hasBlocker(result, 'PURGE_CODE_GRAPH_UNSAFE_DIRECTORY'));
  }
  await assert.rejects(lstat(path.join(target, 'missing-cache')), { code: 'ENOENT' });
});

test('hard-linked graph headers are rejected without modifying either name', async t => {
  const f = await fixture(t), a = await cache(f), other = path.join(f.base, 'shared.json');
  await link(a.filename, other); const before = await readFile(other);
  assert.ok(hasBlocker(await previewControlCodeGraphPurge(f), 'PURGE_CODE_GRAPH_UNSAFE_MANIFEST'));
  assert.deepEqual(await readFile(other), before);
});

test('symbolic graph headers are not followed', async t => {
  const f = await fixture(t), a = await cache(f), outside = path.join(f.base, 'elsewhere.json');
  await rename(a.filename, outside);
  try { await symlink(outside, a.filename, 'file'); }
  catch (error) { if (process.platform === 'win32' && error.code === 'EPERM') { t.skip('Windows file symlink privilege unavailable'); return; } throw error; }
  assert.ok(hasBlocker(await previewControlCodeGraphPurge(f), 'PURGE_CODE_GRAPH_UNSAFE_MANIFEST'));
  assert.ok((await readFile(outside, 'utf8')).includes('GRAPHIFY'));
});

test('valid new survivor caches are classified but never added to target roots', async t => {
  const f = await fixture(t), a = await cache(f), plan = await previewControlCodeGraphPurge(f);
  const b = await cache(f, { projectId: 'synthetic-project-b' }), before = await readFile(b.filename);
  assert.equal((await validateControlCodeGraphPurge(plan)).valid, true);
  await rm(a.root, { recursive: true });
  assert.equal((await readbackControlCodeGraphPurge(plan)).complete, true);
  assert.deepEqual(await readFile(b.filename), before);
});

test('malformed plans cannot broaden the partial-resume exception', async t => {
  const f = await fixture(t); await cache(f); const plan = await previewControlCodeGraphPurge(f);
  for (const change of [
    value => { value.roots.push(path.join(f.base, 'unplanned')); },
    value => { value.bindings[0].sourceRoot = path.join(f.base, 'other'); },
    value => { value.projectId = 'synthetic-project-b'; },
    value => { value.bindings[0].cacheDigest = 'invalid'; },
  ]) {
    const bad = structuredClone(plan); change(bad);
    await assert.rejects(validateControlCodeGraphPurge(bad, { allowPlannedPartial: true }), { code: 'PURGE_CODE_GRAPH_PLAN_INVALID' });
  }
  await assert.rejects(previewControlCodeGraphPurge({ cacheDirectory: 'relative', projectId: f.projectId }), { code: 'PURGE_CODE_GRAPH_PATH_INVALID' });
});

test('oversized headers and aggregate reads above 64 MiB remain blocked', async t => {
  const f = await fixture(t), a = await cache(f), file = await open(a.filename, 'r+');
  try { await file.truncate(64 * 1024 * 1024 + 1); } finally { await file.close(); }
  assert.ok(hasBlocker(await previewControlCodeGraphPurge(f), 'PURGE_CODE_GRAPH_READ_LIMIT'));
  await cache(f, { padding: 33 * 1024 * 1024 });
  await cache(f, { projectId: 'synthetic-project-b', padding: 33 * 1024 * 1024 });
  const result = await previewControlCodeGraphPurge(f);
  assert.equal(result.discovery, 'BLOCKED'); assert.ok(hasBlocker(result, 'PURGE_CODE_GRAPH_READ_LIMIT'));
});

test('direct child enumeration is bounded, with no inferred absence after overflow', async t => {
  const f = await fixture(t); await mkdir(f.cacheDirectory);
  for (let start = 0; start < 4097; start += 128) {
    await Promise.all(Array.from({ length: Math.min(128, 4097 - start) }, (_, offset) =>
      writeFile(path.join(f.cacheDirectory, `unknown-${start + offset}`), '')));
  }
  const result = await previewControlCodeGraphPurge(f);
  assert.equal(result.discovery, 'BLOCKED'); assert.ok(hasBlocker(result, 'PURGE_CODE_GRAPH_ENTRY_LIMIT'));
});
