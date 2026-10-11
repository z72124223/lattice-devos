import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import { mkdtemp, mkdir, writeFile, readFile, symlink, link, unlink, rename, rm, lstat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { LatticeStore } from '../src/store.mjs';
import { previewProjectPurge, applyProjectPurge, statusProjectPurge } from '../src/project-purge.mjs';
import { runProjectPurgeCli } from '../src/project-purge-client.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const directoryLink = process.platform === 'win32' ? 'junction' : 'dir';
const hashFile = async file => createHash('sha256').update(await readFile(file)).digest('hex');
async function fixture(t) {
  const base = await mkdtemp(path.join(here, '.project-purge-directory-targets-'));
  t.after(async () => {
    assert.equal(path.dirname(path.resolve(base)), here);
    assert.ok(path.basename(base).startsWith('.project-purge-directory-targets-'));
    await rm(base, { recursive: true, force: true });
  });
  const target = path.join(base, 'a-target'), source = path.join(base, 'z-owned-link'), survivor = path.join(base, 'survivor');
  await mkdir(target); await mkdir(survivor);
  await writeFile(path.join(target, 'data.txt'), 'synthetic target');
  await writeFile(path.join(survivor, 'keep.txt'), 'synthetic survivor');
  await symlink(target, source, directoryLink);
  const databasePath = path.join(base, 'control.db');
  const store = new LatticeStore(databasePath);
  const project = store.createProject({ name: 'synthetic-target', rootPath: source });
  store.createProject({ name: 'synthetic-survivor', rootPath: survivor });
  store.database.close();
  let receipt, applied = 0, botApplied = 0, authority = [{ path: source, source: 'POSTGRES_REGISTRY_OBSERVATION' }], protectedRoots = [survivor];
  const native = async (_binary, request) => {
    if (request.action === 'preview-bot') return { schema: 'lattice.project-purge.bot.v1', status: 'READY', scopeDigest: 'b'.repeat(64), blockers: [] };
    if (request.action === 'status-bot') return { status: 'NOT_FOUND' };
    if (request.action === 'apply-bot') { botApplied++; return { status: 'PURGED', scopeDigest: 'b'.repeat(64) }; }
    if (request.action === 'preview') return { status: 'READY', project: { id: project.id, canonicalPath: source },
      filesystemRoots: structuredClone(authority), protectedRoots: [...protectedRoots], blockers: [], counts: { projects: 1 }, scopeDigest: 'a'.repeat(64) };
    if (request.action === 'status') return receipt ?? { status: 'UNKNOWN_OPERATION' };
    assert.equal(request.action, 'apply');
    applied++;
    receipt = { status: 'PURGED', phase: 'POSTGRES_ONLY', operationId: request.operationId, scopeDigest: 'a'.repeat(64) };
    return receipt;
  };
  const selection = { sourceRoot: source, targetPath: target };
  const options = { projectId: project.id, databasePath, statePath: path.join(base, 'progress.json'),
    nativeBinary: path.join(base, 'synthetic-native.exe'), botService: null,
    codeGraphCacheDirectory: path.join(base, 'code-graphs'), directoryTargets: [selection] };
  return { base, target, source, survivor, project, databasePath, options, selection, native,
    preview: changes => previewProjectPurge({ ...options, ...changes }, { native }),
    apply: plan => applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, native }),
    get applied() { return applied; }, get botApplied() { return botApplied; },
    setAuthority(value) { authority = value; }, setProtected(value) { protectedRoots = value; } };
}

async function unchanged(f, before) {
  assert.equal(f.applied, 0);
  assert.equal(await hashFile(f.databasePath), before);
  assert.equal(await readFile(path.join(f.target, 'data.txt'), 'utf8'), 'synthetic target');
  assert.equal(await readFile(path.join(f.survivor, 'keep.txt'), 'utf8'), 'synthetic survivor');
}

test('default link-only scope stays unchanged; explicit verified selection includes the physical target', async t => {
  const f = await fixture(t), original = await f.preview({ directoryTargets: undefined });
  assert.equal(original.status, 'READY'); assert.deepEqual(original.files.roots, [f.source]);
  assert.equal(original.files.entries[0].targetAction, 'EXCLUDED_NOT_FOLLOWED');
  const plan = await f.preview();
  assert.equal(plan.status, 'READY'); assert.deepEqual(plan.files.roots, [f.target, f.source]);
  assert.equal(plan.directoryTargets.proofs[0].sources[0].source, 'POSTGRES_REGISTRY_OBSERVATION');
  assert.notEqual(plan.digest, original.digest);
  assert.equal((await f.apply(plan)).status, 'SCOPED_PURGED');
  assert.equal(f.applied, 1);
  await assert.rejects(lstat(f.target), { code: 'ENOENT' });
  await assert.rejects(lstat(f.source), { code: 'ENOENT' });
  assert.equal(await readFile(path.join(f.survivor, 'keep.txt'), 'utf8'), 'synthetic survivor');
});

test('claim roots support explicit selection and preserve an external .venv link target', async t => {
  const f = await fixture(t);
  f.setAuthority([{ path: f.source, source: 'POSTGRES_CONVERSATION_CLAIM' }]);
  await symlink(f.survivor, path.join(f.target, '.venv'), directoryLink);
  const plan = await f.preview();
  assert.equal(plan.status, 'READY');
  assert.equal(plan.files.entries.find(entry => entry.path === path.join(f.target, '.venv')).targetAction, 'EXCLUDED_NOT_FOLLOWED');
  assert.equal(plan.files.entries.some(entry => entry.path === path.join(f.survivor, 'keep.txt')), false);
  await f.apply(plan);
  assert.equal(await readFile(path.join(f.survivor, 'keep.txt'), 'utf8'), 'synthetic survivor');
});

for (const scenario of ['unknown-source', 'untrusted-source-kind', 'wrong-target', 'ordinary-source', 'link-chain', 'parent-target', 'protected-alias', 'overlapping-roots', 'shared-git', 'external-git', 'shared-hardlink']) {
  test(`${scenario} remains blocked with no database or shared-data mutation`, async t => {
    const f = await fixture(t);
    let directoryTargets = [f.selection];
    if (scenario === 'unknown-source') directoryTargets = [{ sourceRoot: path.join(f.base, 'unknown'), targetPath: f.target }];
    if (scenario === 'untrusted-source-kind') f.setAuthority([{ path: f.source, source: 'USER_ASSERTION' }]);
    if (scenario === 'wrong-target') directoryTargets = [{ sourceRoot: f.source, targetPath: f.survivor }];
    if (scenario === 'ordinary-source') { await unlink(f.source); await mkdir(f.source); }
    if (scenario === 'link-chain') {
      const next = path.join(f.base, 'next-link'); await symlink(f.target, next, directoryLink);
      await unlink(f.source); await symlink(next, f.source, directoryLink); directoryTargets = [{ sourceRoot: f.source, targetPath: next }];
    }
    if (scenario === 'parent-target') { await unlink(f.source); await symlink(f.base, f.source, directoryLink); directoryTargets = [{ sourceRoot: f.source, targetPath: f.base }]; }
    if (scenario === 'protected-alias') {
      const alias = path.join(f.base, 'other-project-alias'); await symlink(f.target, alias, directoryLink); f.setProtected([f.survivor, alias]);
    }
    if (scenario === 'overlapping-roots') {
      const nested = path.join(f.target, 'nested'); await mkdir(nested);
      f.setAuthority([{ path: f.source, source: 'POSTGRES_REGISTRY_OBSERVATION' }, { path: nested, source: 'POSTGRES_CONVERSATION_CLAIM' }]);
    }
    if (scenario === 'shared-git') { await mkdir(path.join(f.target, '.git', 'worktrees', 'other'), { recursive: true }); await writeFile(path.join(f.target, '.git', 'worktrees', 'other', 'gitdir'), path.join(f.survivor, '.git')); }
    if (scenario === 'external-git') { await mkdir(path.join(f.survivor, 'git')); await writeFile(path.join(f.target, '.git'), `gitdir: ${path.join(f.survivor, 'git')}\n`); }
    if (scenario === 'shared-hardlink') await link(path.join(f.survivor, 'keep.txt'), path.join(f.target, 'shared.txt'));
    const before = await hashFile(f.databasePath), plan = await f.preview({ directoryTargets });
    assert.equal(plan.status, 'BLOCKED');
    assert.ok(plan.blockers.length);
    await assert.rejects(f.apply(plan), /PURGE_BLOCKED|PURGE_PLAN_INVALID/);
    await unchanged(f, before);
  });
}

for (const drift of ['authority', 'protected-roots', 'link-target', 'physical-identity', 'new-file']) {
  test(`preview/apply rejects ${drift} drift before native mutation`, async t => {
    const f = await fixture(t), plan = await f.preview(), before = await hashFile(f.databasePath);
    if (drift === 'authority') f.setAuthority([{ path: f.source, source: 'POSTGRES_CONVERSATION_CLAIM' }]);
    if (drift === 'protected-roots') f.setProtected([f.survivor, path.join(f.base, 'future-survivor')]);
    if (drift === 'link-target') { await unlink(f.source); await symlink(f.survivor, f.source, directoryLink); }
    if (drift === 'physical-identity') { await rename(f.target, path.join(f.base, 'old-target')); await mkdir(f.target); await writeFile(path.join(f.target, 'data.txt'), 'synthetic target'); }
    if (drift === 'new-file') await writeFile(path.join(f.target, 'new.txt'), 'not in approved manifest');
    await assert.rejects(f.apply(plan), /PURGE_FILE_(?:DIRECTORY_SCOPE_CHANGED|SCOPE_CHANGED)/);
    await unchanged(f, before);
  });
}

test('resume after committed PG and removed source link follows the exact saved intent without rediscovery', async t => {
  const f = await fixture(t), plan = await f.preview();
  const originalUnlink = fs.promises.unlink, originalRename = fs.promises.rename;
  let injected = false, failedSaves = 0;
  fs.promises.unlink = async target => {
    await originalUnlink(target);
    if (target === f.source) injected = true;
  };
  fs.promises.rename = async (from, to) => {
    // Fail the post-unlink progress save, returned receipt save and catch save.
    // The durable record must therefore still contain the pre-unlink intent.
    if (injected && from === `${f.options.statePath}.tmp` && to === f.options.statePath && failedSaves < 3) {
      failedSaves++; throw Object.assign(new Error('synthetic progress I/O failure'), { code: 'EIO' });
    }
    return originalRename(from, to);
  };
  syncBuiltinESMExports();
  const restore = () => { fs.promises.unlink = originalUnlink; fs.promises.rename = originalRename; syncBuiltinESMExports(); };
  t.after(restore);
  await assert.rejects(f.apply(plan), { code: 'EIO' });
  restore();
  assert.equal(injected, true); assert.equal(f.applied, 1); assert.equal(failedSaves, 3);
  const durable = JSON.parse(await readFile(f.options.statePath, 'utf8'));
  assert.equal(durable.files.pendingRemoval, f.source);
  assert.equal(durable.files.removed.includes(f.source), false);
  await assert.rejects(lstat(f.source), { code: 'ENOENT' });
  // Deleted native rows cannot be used to re-authorize or discover new roots.
  f.setAuthority([]);
  const result = await f.apply(plan);
  assert.equal(result.status, 'SCOPED_PURGED'); assert.equal(f.applied, 1);
  await assert.rejects(lstat(f.target), { code: 'ENOENT' });
  assert.equal(await readFile(path.join(f.survivor, 'keep.txt'), 'utf8'), 'synthetic survivor');
});

test('file drift rejects before survivor rebuild changes SQLite or leaves a maintenance marker', async t => {
  const f = await fixture(t), store = new LatticeStore(f.databasePath);
  try {
    store.createInstallationReceipt({ projectId: f.project.id, component: 'synthetic', sourceCommitSha: 'a'.repeat(40),
      artifactPath: path.join(f.source, 'synthetic.exe'), artifactSha256: 'b'.repeat(64) });
  } finally { store.close(); }
  const plan = await f.preview(), before = await hashFile(f.databasePath);
  assert.equal(plan.sqlite.strategy, 'REBUILD_SURVIVORS_V1');
  await writeFile(path.join(f.target, 'unapproved.txt'), 'scope drift');
  await assert.rejects(f.apply(plan), /PURGE_FILE_SCOPE_CHANGED/);
  await unchanged(f, before);
  // A new preview must remain possible; an abandoned swap marker rejects it.
  assert.equal((await f.preview({ statePath: path.join(f.base, 'fresh-progress.json') })).status, 'READY');
});

for (const drift of ['authority', 'protected-roots']) {
  test(`${drift} drift also prevents READY Bot mutation`, async t => {
    const f = await fixture(t), before = await hashFile(f.databasePath);
    const plan = await f.preview({ botService: { port: 15499, runId: 'b'.repeat(32), systemIdentifier: '123' } });
    assert.equal(plan.bot.status, 'READY');
    if (drift === 'authority') f.setAuthority([]);
    else f.setProtected([f.survivor, path.join(f.base, 'new-protected-root')]);
    await assert.rejects(f.apply(plan), /PURGE_FILE_DIRECTORY_SCOPE_CHANGED/);
    assert.equal(f.botApplied, 0); await unchanged(f, before);
  });
}

test('CLI inventory records admitted and unknown selections and never treats a path assertion as authority', async t => {
  const f = await fixture(t), config = path.join(f.base, 'input.json'), planPath = path.join(f.base, 'plan.json');
  await writeFile(config, JSON.stringify({ ...f.options, directoryTargets: [f.selection,
    { sourceRoot: path.join(f.base, 'unregistered-conversation-cwd'), targetPath: path.join(f.base, 'unregistered-output') }] }));
  const result = await runProjectPurgeCli(['inventory', '--input', config, '--plan', planPath], {
    preview: options => previewProjectPurge(options, { native: f.native }),
  });
  assert.equal(result.exitCode, 1);
  const plan = JSON.parse(await readFile(planPath, 'utf8'));
  assert.equal(plan.status, 'BLOCKED'); assert.equal(plan.directoryTargets.requested.length, 2);
  assert.deepEqual(plan.directoryTargets.roots, [f.target]);
  assert.ok(plan.blockers.some(item => item.code === 'PURGE_FILE_DIRECTORY_OWNERSHIP_NOT_PROVEN'));
  const output = JSON.parse(result.output);
  assert.equal(output.report.stages.find(stage => stage.kind === 'files').directoryTargets.requested.length, 2);
  assert.equal(output.report.complete, false); assert.equal(f.applied, 0);
  const status = await runProjectPurgeCli(['status', '--plan', planPath], {
    status: value => statusProjectPurge(value, { native: f.native }),
  });
  assert.equal(status.exitCode, 1);
  assert.equal(JSON.parse(status.output).status, 'BLOCKED');
  assert.ok(JSON.parse(status.output).report.blockers.some(item => item.code === 'PURGE_FILE_DIRECTORY_OWNERSHIP_NOT_PROVEN'));
});

test('two registered links plus three physical directories remain fully visible when Git is shared and one owner is unknown', async t => {
  const f = await fixture(t), worktree = path.join(f.base, 'b-worktree'), worktreeLink = path.join(f.base, 'y-claim-link');
  const unknown = path.join(f.base, 'unregistered-delivery');
  await mkdir(worktree); await mkdir(unknown);
  await writeFile(path.join(unknown, 'keep.txt'), 'unknown owner is retained');
  await mkdir(path.join(f.target, '.git', 'worktrees', 'synthetic-claim'), { recursive: true });
  await writeFile(path.join(f.target, '.git', 'worktrees', 'synthetic-claim', 'commondir'), '../..\n');
  await writeFile(path.join(worktree, '.git'), `gitdir: ${path.join(f.target, '.git', 'worktrees', 'synthetic-claim')}\n`);
  await symlink(worktree, worktreeLink, directoryLink);
  f.setAuthority([{ path: f.source, source: 'POSTGRES_REGISTRY_OBSERVATION' },
    { path: worktreeLink, source: 'POSTGRES_CONVERSATION_CLAIM' }]);
  const before = await hashFile(f.databasePath);
  const plan = await f.preview({ directoryTargets: [f.selection,
    { sourceRoot: worktreeLink, targetPath: worktree }, { sourceRoot: unknown, targetPath: unknown }] });
  assert.equal(plan.status, 'BLOCKED'); assert.equal(plan.directoryTargets.requested.length, 3);
  assert.deepEqual(new Set(plan.files.roots), new Set([f.source, worktreeLink, f.target, worktree]));
  assert.ok(plan.blockers.some(item => item.code === 'PURGE_FILE_DIRECTORY_OWNERSHIP_NOT_PROVEN' && item.targetPath === unknown));
  assert.ok(plan.blockers.some(item => ['PURGE_FILE_SHARED_GIT_METADATA', 'PURGE_FILE_EXTERNAL_GIT_METADATA'].includes(item)));
  await assert.rejects(f.apply(plan), /PURGE_PLAN_INVALID|PURGE_BLOCKED/);
  await unchanged(f, before);
  assert.equal(await readFile(path.join(unknown, 'keep.txt'), 'utf8'), 'unknown owner is retained');
});
