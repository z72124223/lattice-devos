import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, lstat, rename, rm, rmdir, symlink, link, readdir } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { previewProjectPurgeFiles, validateProjectPurgeFiles, applyProjectPurgeFiles, readbackProjectPurgeFiles } from '../src/project-purge-files.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
async function fixture(t) {
  const base = await mkdtemp(path.join(here, '.project-purge-files-fixture-'));
  t.after(async () => {
    // Only this newly-created synthetic tree is eligible for test cleanup.
    assert.equal(path.dirname(path.resolve(base)), here);
    assert.ok(path.basename(base).startsWith('.project-purge-files-fixture-'));
    await rm(base, { recursive: true, force: true });
    await rm(base + '-old', { recursive: true, force: true });
  });
  const root = path.join(base, 'project-a'), other = path.join(base, 'project-b');
  await mkdir(path.join(root, 'nested'), { recursive: true });
  await mkdir(other);
  await writeFile(path.join(root, 'nested', 'data.txt'), 'synthetic project data');
  await writeFile(path.join(other, 'keep.txt'), 'other project unchanged');
  return { base, root, other, options: { projectId: 'synthetic-project-a', roots: [root], protectedRoots: [other] } };
}
const bound = (plan, options) => ({ ...options, planDigest: plan.planDigest });

for (const destination of ['inside', 'outside']) {
  test(`hard links ${destination} the project are rejected during inventory without removing any path`, async t => {
    const f = await fixture(t);
    const original = path.join(f.root, 'nested', 'data.txt');
    const alias = path.join(destination === 'inside' ? f.root : f.other, 'hardlink.txt');
    await link(original, alias);
    const originalStat = await lstat(original, { bigint: true }), aliasStat = await lstat(alias, { bigint: true });
    assert.equal(originalStat.nlink, 2n);
    assert.equal(aliasStat.nlink, 2n);
    assert.equal(originalStat.dev, aliasStat.dev);
    assert.equal(originalStat.ino, aliasStat.ino);
    t.diagnostic(`${process.platform}: both hard-link names report nlink=2 and the same device/inode`);
    await assert.rejects(previewProjectPurgeFiles(f.options), { code: 'PURGE_FILE_HARDLINK_UNSUPPORTED' });
    assert.equal(await readFile(original, 'utf8'), 'synthetic project data');
    assert.equal(await readFile(alias, 'utf8'), 'synthetic project data');
    assert.equal(await readFile(path.join(f.other, 'keep.txt'), 'utf8'), 'other project unchanged');
  });
}

test('durable per-entry intent resumes after unlink but before the next progress save', async t => {
  const f = await fixture(t), plan = await previewProjectPurgeFiles(f.options);
  const pending = path.join(f.root, 'nested', 'data.txt');
  const progress = { projectId: plan.projectId, planDigest: plan.planDigest, removed: [], pendingRemoval: pending };
  await rm(pending); // Reproduce the exact crash window in this synthetic fixture.
  const events = [];
  const result = await applyProjectPurgeFiles(plan, { ...bound(plan, f.options), previousResult: progress,
    onProgress: async state => { events.push(structuredClone(state)); } });
  assert.equal(result.status, 'completed');
  assert.ok(result.removed.includes(pending));
  assert.ok(events.some(event => event.pendingRemoval !== null));
  assert.equal(events.at(-1).pendingRemoval, null);
  assert.equal(await readFile(path.join(f.other, 'keep.txt'), 'utf8'), 'other project unchanged');
});

test('a pending removal neither permits replacement bytes nor deletion of another entry', async t => {
  const f = await fixture(t), plan = await previewProjectPurgeFiles(f.options);
  const pending = path.join(f.root, 'nested', 'data.txt');
  const receipt = { projectId: plan.projectId, planDigest: plan.planDigest, removed: [], pendingRemoval: pending };
  await writeFile(pending, 'unapproved replacement content');
  await assert.rejects(applyProjectPurgeFiles(plan, { ...bound(plan, f.options), previousResult: receipt }), /PURGE_FILE_SCOPE_CHANGED/);
  receipt.pendingRemoval = path.join(f.other, 'keep.txt');
  await assert.rejects(applyProjectPurgeFiles(plan, { ...bound(plan, f.options), previousResult: receipt }), /PURGE_FILE_RECEIPT_MISMATCH/);
});

test('failure to persist removal intent prevents filesystem mutation', async t => {
  const f = await fixture(t), plan = await previewProjectPurgeFiles(f.options);
  const result = await applyProjectPurgeFiles(plan, { ...bound(plan, f.options), onProgress: async state => {
    if (state.pendingRemoval) throw Object.assign(new Error('disk full'), { code: 'ENOSPC' });
  } });
  assert.equal(result.status, 'failed');
  assert.deepEqual(result.removed, []);
  assert.equal(result.failed[0].code, 'ENOSPC');
  assert.equal(await readFile(path.join(f.root, 'nested', 'data.txt'), 'utf8'), 'synthetic project data');
});

test('deletes the exact synthetic project and preserves the protected project; repeated apply is a no-op', async t => {
  const { root, other, options } = await fixture(t);
  const plan = await previewProjectPurgeFiles(options);
  assert.equal(plan.entries.length, 3);
  assert.equal((await validateProjectPurgeFiles(plan, bound(plan, options))).valid, true);
  const result = await applyProjectPurgeFiles(plan, bound(plan, options));
  assert.equal(result.status, 'completed');
  assert.equal(result.removed.length, 3);
  await assert.rejects(lstat(root), { code: 'ENOENT' });
  assert.equal(await readFile(path.join(other, 'keep.txt'), 'utf8'), 'other project unchanged');
  assert.equal((await readbackProjectPurgeFiles(plan, bound(plan, options))).complete, true);
  const repeated = await applyProjectPurgeFiles(plan, bound(plan, options));
  assert.equal(repeated.status, 'completed');
  assert.deepEqual(repeated.removed, []);
  assert.deepEqual(repeated.alreadyAbsent, [root]);
});

test('refuses changed contents, added entries, replaced roots, and replaced ancestors before deleting', async t => {
  for (const change of ['content', 'entry', 'root', 'ancestor']) {
    const f = await fixture(t), plan = await previewProjectPurgeFiles(f.options);
    if (change === 'content') await writeFile(path.join(f.root, 'nested', 'data.txt'), 'changed');
    if (change === 'entry') await writeFile(path.join(f.root, 'new.txt'), 'new');
    if (change === 'root') { await rename(f.root, f.root + '-old'); await mkdir(f.root); }
    if (change === 'ancestor') {
      await rename(f.base, f.base + '-old'); await mkdir(f.base);
      await rename(path.join(f.base + '-old', 'project-a'), f.root);
      await rename(path.join(f.base + '-old', 'project-b'), f.other);
      await rmdir(f.base + '-old');
    }
    await assert.rejects(applyProjectPurgeFiles(plan, bound(plan, f.options)), { code: 'PURGE_FILE_SCOPE_CHANGED' });
    assert.ok((await lstat(f.root)).isDirectory());
  }
});

test('binds approval to project identity, exact authoritative roots, protected scope, and digest', async t => {
  const { root, other, options } = await fixture(t), plan = await previewProjectPurgeFiles(options);
  for (const altered of [
    { ...bound(plan, options), projectId: 'different-project' },
    { ...bound(plan, options), roots: [other], protectedRoots: [] },
    { ...bound(plan, options), protectedRoots: [] },
  ]) await assert.rejects(applyProjectPurgeFiles(plan, altered), { code: 'PURGE_FILE_SCOPE_MISMATCH' });
  await assert.rejects(applyProjectPurgeFiles(plan, { ...bound(plan, options), planDigest: '0'.repeat(64) }), { code: 'PURGE_FILE_PLAN_DIGEST_MISMATCH' });
  const tampered = structuredClone(plan); tampered.entries = [];
  await assert.rejects(applyProjectPurgeFiles(tampered, bound(plan, options)), { code: 'PURGE_FILE_PLAN_DIGEST_MISMATCH' });
  assert.ok((await lstat(root)).isDirectory());
});

test('rejects system, home, traversal, shared, overlapping, and nested repository roots', async t => {
  const { base, root, other, options } = await fixture(t);
  for (const unsafe of [path.parse(root).root, os.homedir(), path.dirname(os.homedir())]) {
    await assert.rejects(previewProjectPurgeFiles({ ...options, roots: [unsafe] }), { code: 'PURGE_FILE_UNSAFE_ROOT' });
  }
  await assert.rejects(previewProjectPurgeFiles({ ...options, roots: [root + path.sep + '..' + path.sep + 'project-b'] }), { code: 'PURGE_FILE_PATH_TRAVERSAL' });
  for (const shared of [root, base, path.join(root, 'nested')]) {
    await assert.rejects(previewProjectPurgeFiles({ ...options, protectedRoots: [shared] }), { code: 'PURGE_FILE_PROTECTED_ROOT' });
  }
  await assert.rejects(previewProjectPurgeFiles({ ...options, roots: [root, path.join(root, 'nested')] }), { code: 'PURGE_FILE_OVERLAPPING_ROOTS' });
  await mkdir(path.join(root, 'nested', '.git'));
  await assert.rejects(previewProjectPurgeFiles(options), { code: 'PURGE_FILE_NESTED_REPOSITORY' });
  assert.deepEqual(await readdir(other), ['keep.txt']);
});

test('Windows refuses POSIX and drive-relative roots instead of resolving them onto the current drive', { skip: process.platform !== 'win32' }, async t => {
  const { root, options } = await fixture(t);
  for (const invalid of ['/home/synthetic-project', '\\synthetic-project']) {
    assert.equal(path.isAbsolute(invalid), true);
    await assert.rejects(previewProjectPurgeFiles({ ...options, roots: [invalid] }), { code: 'PURGE_FILE_PATH_INVALID' });
  }
  assert.ok((await lstat(root)).isDirectory());
});

test('junction or symlink is deleted as a link; its protected target is neither traversed nor deleted', async t => {
  const { root, other, options } = await fixture(t), link = path.join(root, 'shared-link');
  await symlink(other, link, process.platform === 'win32' ? 'junction' : 'dir');
  const plan = await previewProjectPurgeFiles(options);
  const entry = plan.entries.find(item => item.path === link);
  assert.equal(entry.type, 'link');
  assert.equal(entry.targetAction, 'EXCLUDED_NOT_FOLLOWED');
  assert.equal(plan.entries.some(item => item.path === path.join(link, 'keep.txt')), false);
  assert.equal((await applyProjectPurgeFiles(plan, bound(plan, options))).status, 'completed');
  assert.equal(await readFile(path.join(other, 'keep.txt'), 'utf8'), 'other project unchanged');
});

test('rejects a target reached through a junction or symlink ancestor', async t => {
  const { base, root, options } = await fixture(t), alias = path.join(base, 'alias');
  await symlink(root, alias, process.platform === 'win32' ? 'junction' : 'dir');
  await assert.rejects(previewProjectPurgeFiles({ ...options, roots: [path.join(alias, 'nested')] }), { code: 'PURGE_FILE_UNSAFE_ANCESTOR' });
  assert.equal(await readFile(path.join(root, 'nested', 'data.txt'), 'utf8'), 'synthetic project data');
});

test('a protected alias or its missing descendant still protects the real project root', async t => {
  const { base, root, options } = await fixture(t), alias = path.join(base, 'protected-alias');
  await symlink(root, alias, process.platform === 'win32' ? 'junction' : 'dir');
  for (const protectedPath of [alias, path.join(alias, 'missing-shared-data')]) {
    await assert.rejects(previewProjectPurgeFiles({ ...options, protectedRoots: [protectedPath] }), { code: 'PURGE_FILE_PROTECTED_ROOT' });
  }
});

test('shared Git worktree or submodule metadata refuses deletion and preserves external worktrees', async t => {
  for (const kind of ['worktrees', 'modules']) {
    const { root, other, options } = await fixture(t);
    await mkdir(path.join(root, '.git', kind, 'shared-entry'), { recursive: true });
    await writeFile(path.join(root, '.git', kind, 'shared-entry', 'gitdir'), path.join(other, '.git'));
    await writeFile(path.join(other, '.git'), `gitdir: ${path.join(root, '.git', kind, 'shared-entry')}\n`);
    const externalBefore = await readFile(path.join(other, '.git'), 'utf8');
    await assert.rejects(previewProjectPurgeFiles(options), { code: 'PURGE_FILE_SHARED_GIT_METADATA' });
    assert.equal(await readFile(path.join(other, '.git'), 'utf8'), externalBefore);
    assert.equal(await readFile(path.join(other, 'keep.txt'), 'utf8'), 'other project unchanged');
  }
});

test('Git pointer outside the root and shared common directory fail closed', async t => {
  const { root, other, options } = await fixture(t);
  await writeFile(path.join(root, '.git'), `gitdir: ${other}\n`);
  await assert.rejects(previewProjectPurgeFiles(options), { code: 'PURGE_FILE_EXTERNAL_GIT_METADATA' });
  assert.equal(await readFile(path.join(other, 'keep.txt'), 'utf8'), 'other project unchanged');
  await mkdir(path.join(root, 'own-metadata'));
  await writeFile(path.join(root, '.git'), 'gitdir: own-metadata\n');
  await writeFile(path.join(root, 'own-metadata', 'commondir'), other);
  await assert.rejects(previewProjectPurgeFiles(options), { code: 'PURGE_FILE_SHARED_GIT_METADATA' });
});

test('Git metadata reached through a root .git junction is not silently abandoned', async t => {
  const { root, other, options } = await fixture(t);
  await symlink(other, path.join(root, '.git'), process.platform === 'win32' ? 'junction' : 'dir');
  await assert.rejects(previewProjectPurgeFiles(options), { code: 'PURGE_FILE_EXTERNAL_GIT_METADATA' });
  assert.equal(await readFile(path.join(other, 'keep.txt'), 'utf8'), 'other project unchanged');
});

test('an internal Git pointer is included while new shared metadata invalidates a prior preview', async t => {
  const { root, options } = await fixture(t);
  await mkdir(path.join(root, 'own-metadata'));
  await writeFile(path.join(root, '.git'), 'gitdir: own-metadata\n');
  const plan = await previewProjectPurgeFiles(options);
  await mkdir(path.join(root, 'own-metadata', 'worktrees', 'new-shared'), { recursive: true });
  await assert.rejects(applyProjectPurgeFiles(plan, bound(plan, options)), { code: 'PURGE_FILE_SHARED_GIT_METADATA' });
  assert.ok((await lstat(root)).isDirectory());
});

test('retargeting a planned link fails closed and readback does not mistake a replaced ancestor for completion', async t => {
  const { base, root, other, options } = await fixture(t), link = path.join(root, 'link');
  await symlink(other, link, process.platform === 'win32' ? 'junction' : 'dir');
  const plan = await previewProjectPurgeFiles(options);
  await rm(link); await symlink(base, link, process.platform === 'win32' ? 'junction' : 'dir');
  await assert.rejects(applyProjectPurgeFiles(plan, bound(plan, options)), { code: 'PURGE_FILE_SCOPE_CHANGED' });
  await rename(base, base + '-old'); await mkdir(base);
  const readback = await readbackProjectPurgeFiles(plan, bound(plan, options));
  assert.equal(readback.complete, false);
  assert.equal(readback.errors[0].code, 'PURGE_FILE_SCOPE_CHANGED');
  await rmdir(base); await rename(base + '-old', base);
});

test('bounded inventory refuses oversized or too-deep trees and does not read file contents', async t => {
  const { options } = await fixture(t);
  await assert.rejects(previewProjectPurgeFiles({ ...options, limits: { maxEntries: 2 } }), { code: 'PURGE_FILE_MANIFEST_LIMIT' });
  await assert.rejects(previewProjectPurgeFiles({ ...options, limits: { maxDepth: 1 } }), { code: 'PURGE_FILE_MANIFEST_LIMIT' });
  const plan = await previewProjectPurgeFiles(options);
  assert.equal(JSON.stringify(plan).includes('synthetic project data'), false);
  await assert.rejects(previewProjectPurgeFiles({ ...options, limits: { maxEntries: 10001 } }), { code: 'PURGE_FILE_LIMIT_INVALID' });
});

test('an absent authoritative root is an explicit no-op; a newly created root is not silently approved', async t => {
  const { base, options } = await fixture(t), absent = path.join(base, 'not-created');
  const scoped = { ...options, roots: [absent] }, plan = await previewProjectPurgeFiles(scoped);
  assert.equal((await applyProjectPurgeFiles(plan, bound(plan, scoped))).status, 'completed');
  await mkdir(absent);
  await assert.rejects(applyProjectPurgeFiles(plan, bound(plan, scoped)), { code: 'PURGE_FILE_SCOPE_CHANGED' });
});

test('Windows locked synthetic file produces an honest partial receipt, and an exact retry completes', { skip: process.platform !== 'win32' }, async t => {
  const { root, other, options } = await fixture(t);
  const locked = path.join(root, 'a-locked.txt'), unlocked = path.join(root, 'z-removable.txt');
  await writeFile(locked, 'synthetic locked file'); await writeFile(unlocked, 'synthetic removable file');
  const quoted = locked.replaceAll("'", "''");
  const holder = spawn('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command',
    `$stream = [System.IO.File]::Open('${quoted}', [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::Read); try { [Console]::WriteLine('LOCKED'); [Console]::ReadLine() | Out-Null } finally { $stream.Dispose() }`],
  { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  const exited = once(holder, 'exit');
  t.after(async () => { holder.stdin.end('\n'); await exited; });
  await new Promise((resolve, reject) => {
    let output = '';
    const timer = setTimeout(() => reject(new Error('synthetic lock setup timed out')), 10000);
    holder.on('error', error => { clearTimeout(timer); reject(error); });
    holder.on('exit', () => { if (!output.includes('LOCKED')) { clearTimeout(timer); reject(new Error('synthetic lock setup failed')); } });
    holder.stdout.on('data', chunk => { output += chunk; if (output.includes('LOCKED')) { clearTimeout(timer); resolve(); } });
  });
  const plan = await previewProjectPurgeFiles(options);
  const result = await applyProjectPurgeFiles(plan, bound(plan, options));
  assert.equal(result.status, 'partial');
  assert.equal(result.readback.complete, false);
  assert.ok(result.removed.includes(unlocked));
  assert.equal(result.failed.length, 1);
  assert.equal(result.failed[0].path, locked);
  assert.ok(['EPERM', 'EACCES', 'EBUSY'].includes(result.failed[0].code));
  assert.equal(await readFile(path.join(other, 'keep.txt'), 'utf8'), 'other project unchanged');
  holder.stdin.end('\n'); await exited;
  const retried = await applyProjectPurgeFiles(plan, { ...bound(plan, options), previousResult: result });
  assert.equal(retried.status, 'completed');
  assert.equal(retried.readback.complete, true);
});
