import { createHash } from 'node:crypto';
import { lstat, opendir, open, readlink, realpath, unlink, rmdir } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SCHEMA = 'lattice.project-purge-files.v1';
const DEFAULT_LIMITS = Object.freeze({ maxEntries: 10000, maxDepth: 64, maxManifestBytes: 8388608 });
const self = fileURLToPath(import.meta.url);
const compare = (a, b) => a < b ? -1 : a > b ? 1 : 0;
const key = value => process.platform === 'win32' ? value.toLowerCase() : value;
const within = (child, parent) => key(child) === key(parent) || key(child).startsWith(key(parent.endsWith(path.sep) ? parent : parent + path.sep));
const overlap = (a, b) => within(a, b) || within(b, a);
const digest = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const fail = code => { throw Object.assign(new Error(code), { code }); };

function absolute(value) {
  if (typeof value !== 'string' || !value || Buffer.byteLength(value) > 4096 || value.trim() !== value || /[\x00-\x1f\x7f]/u.test(value) || !path.isAbsolute(value)) fail('PURGE_FILE_PATH_INVALID');
  if (value.split(/[\\/]/u).some(part => part === '.' || part === '..')) fail('PURGE_FILE_PATH_TRAVERSAL');
  if (process.platform === 'win32' && (!/^[A-Za-z]:[\\/]/u.test(value) || /^[\\/]{2}/u.test(value) || value.slice(2).includes(':') || value.split(/[\\/]/u).some(part => /[. ]$/u.test(part)))) fail('PURGE_FILE_PATH_INVALID');
  return path.resolve(value);
}

function scope({ projectId, roots, protectedRoots }) {
  if (typeof projectId !== 'string' || !projectId.trim() || projectId.length > 128 || /[\x00-\x1f\x7f]/u.test(projectId)) fail('PURGE_FILE_PROJECT_INVALID');
  if (!Array.isArray(roots) || roots.length > 128 || !Array.isArray(protectedRoots) || protectedRoots.length > 4096) fail('PURGE_FILE_SCOPE_INVALID');
  const canonicalRoots = roots.map(absolute).sort(compare);
  const protectedPaths = [...new Set(protectedRoots.map(absolute))].sort(compare);
  for (const [index, root] of canonicalRoots.entries()) {
    if (within(os.homedir(), root) || within(self, root) || root === path.parse(root).root) fail('PURGE_FILE_UNSAFE_ROOT');
    if (canonicalRoots.some((other, otherIndex) => index !== otherIndex && overlap(root, other))) fail('PURGE_FILE_OVERLAPPING_ROOTS');
    if (protectedPaths.some(other => overlap(root, other))) fail('PURGE_FILE_PROTECTED_ROOT');
  }
  return { projectId, roots: canonicalRoots, protectedRoots: protectedPaths };
}

function limitsFor(limits = {}) {
  for (const name of Object.keys(limits)) if (!(name in DEFAULT_LIMITS)) fail('PURGE_FILE_LIMIT_INVALID');
  const result = { ...DEFAULT_LIMITS, ...limits };
  for (const [name, value] of Object.entries(result)) if (!Number.isSafeInteger(value) || value < 1 || value > DEFAULT_LIMITS[name]) fail('PURGE_FILE_LIMIT_INVALID');
  return result;
}

async function inspect(target) {
  try { return await lstat(target, { bigint: true }); }
  catch (error) { if (error.code === 'ENOENT') return null; throw error; }
}

function identity(stat) {
  return { dev: String(stat.dev), ino: String(stat.ino), birthtimeNs: String(stat.birthtimeNs), mode: String(stat.mode) };
}

function details(stat) {
  return { identity: identity(stat), size: String(stat.size), mtimeNs: String(stat.mtimeNs), ctimeNs: String(stat.ctimeNs), nlink: String(stat.nlink) };
}

async function ancestry(roots) {
  const result = new Map();
  for (const root of roots) {
    const parents = [];
    for (let current = path.dirname(root); ;) {
      parents.unshift(current);
      const parent = path.dirname(current);
      if (parent === current) break;
      current = parent;
    }
    for (const current of parents) {
      if (result.has(current)) continue;
      const stat = await inspect(current);
      if (!stat || !stat.isDirectory() || stat.isSymbolicLink()) fail('PURGE_FILE_UNSAFE_ANCESTOR');
      if (key(await realpath(current)) !== key(current)) fail('PURGE_FILE_UNSAFE_ANCESTOR');
      result.set(current, { path: current, identity: identity(stat) });
    }
  }
  return [...result.values()].sort((a, b) => compare(a.path, b.path));
}

async function checkProtectedAliases({ roots, protectedRoots }) {
  for (const protectedPath of protectedRoots) {
    let existing = protectedPath, suffix = [];
    while (!(await inspect(existing))) {
      const parent = path.dirname(existing);
      if (parent === existing) fail('PURGE_FILE_UNSAFE_ANCESTOR');
      suffix.unshift(path.basename(existing)); existing = parent;
    }
    // Resolving a protected path only adds protection; it never authorizes walking
    // or deleting the resolved target. Include aliases and missing descendants.
    const canonical = path.resolve(await realpath(existing), ...suffix);
    if (roots.some(root => overlap(root, canonical))) fail('PURGE_FILE_PROTECTED_ROOT');
  }
}

async function manifest(roots, limits) {
  const entries = [], absentRoots = [];
  let bytes = 0;
  async function gitDirectory(root) {
    const gitPath = path.join(root, '.git'), stat = await inspect(gitPath);
    if (!stat) return null;
    if (stat.isSymbolicLink()) fail('PURGE_FILE_EXTERNAL_GIT_METADATA');
    if (stat.isDirectory()) return gitPath;
    if (!stat.isFile() || stat.size > 4096n) fail('PURGE_FILE_GIT_METADATA_INVALID');
    // Read only Git's bounded administrative pointer, never arbitrary project
    // contents. The target is inspected by the normal bounded tree walk.
    const handle = await open(gitPath, 'r');
    let pointer;
    try {
      if (digest(details(await handle.stat({ bigint: true }))) !== digest(details(stat))) fail('PURGE_FILE_SCOPE_CHANGED');
      const buffer = Buffer.alloc(4097), result = await handle.read(buffer, 0, 4097, 0);
      if (result.bytesRead > 4096) fail('PURGE_FILE_GIT_METADATA_INVALID');
      pointer = buffer.subarray(0, result.bytesRead).toString('utf8');
    } finally { await handle.close(); }
    const match = /^gitdir: ([^\r\n\x00]+)\r?\n?$/u.exec(pointer);
    if (!match) fail('PURGE_FILE_GIT_METADATA_INVALID');
    const target = path.resolve(root, match[1]);
    if (!within(target, root)) fail('PURGE_FILE_EXTERNAL_GIT_METADATA');
    await ancestry([target]);
    const targetStat = await inspect(target);
    if (targetStat && (!targetStat.isDirectory() || targetStat.isSymbolicLink() || key(await realpath(target)) !== key(target))) fail('PURGE_FILE_GIT_METADATA_INVALID');
    return target;
  }
  const gitDirectories = new Map();
  for (const root of roots) {
    const stat = await inspect(root);
    if (stat?.isDirectory() && !stat.isSymbolicLink()) gitDirectories.set(root, await gitDirectory(root));
  }
  async function walk(target, root, depth) {
    if (depth > limits.maxDepth) fail('PURGE_FILE_MANIFEST_LIMIT');
    const stat = await inspect(target);
    if (!stat) { if (depth === 0) { absentRoots.push(root); return; } fail('PURGE_FILE_SCOPE_CHANGED'); }
    const type = stat.isSymbolicLink() ? 'link' : stat.isDirectory() ? 'directory' : stat.isFile() ? 'file' : 'unsupported';
    if (type === 'unsupported') fail('PURGE_FILE_UNSUPPORTED_TYPE');
    if (type !== 'link' && key(await realpath(target)) !== key(target)) fail('PURGE_FILE_PATH_ALIAS');
    const gitRoot = gitDirectories.get(root);
    if (gitRoot && key(target) === key(path.join(gitRoot, 'commondir'))) fail('PURGE_FILE_SHARED_GIT_METADATA');
    const entry = { root, path: target, relativePath: path.relative(root, target), type, ...details(stat) };
    if (type === 'link') {
      entry.linkTarget = await readlink(target);
      entry.targetAction = 'EXCLUDED_NOT_FOLLOWED';
    }
    bytes += Buffer.byteLength(JSON.stringify(entry));
    if (entries.length >= limits.maxEntries || bytes > limits.maxManifestBytes) fail('PURGE_FILE_MANIFEST_LIMIT');
    entries.push(entry);
    if (type === 'directory') {
      const names = [];
      for await (const child of await opendir(target)) {
        names.push(child.name);
        if (names.length + entries.length > limits.maxEntries) fail('PURGE_FILE_MANIFEST_LIMIT');
      }
      names.sort(compare);
      if (names.length && gitRoot && ['worktrees', 'modules'].some(name => key(target) === key(path.join(gitRoot, name)))) fail('PURGE_FILE_SHARED_GIT_METADATA');
      if (depth > 0 && names.some(name => key(name) === '.git')) fail('PURGE_FILE_NESTED_REPOSITORY');
      for (const name of names) {
        // Names come from the directory, but still reject ambiguous Windows aliases.
        const child = absolute(path.join(target, name));
        if (!within(child, root)) fail('PURGE_FILE_PATH_TRAVERSAL');
        await walk(child, root, depth + 1);
      }
      const after = await inspect(target);
      if (!after || digest(details(after)) !== digest(details(stat))) fail('PURGE_FILE_SCOPE_CHANGED');
    }
  }
  for (const root of roots) await walk(root, root, 0);
  return { entries, absentRoots };
}

function boundPlan(plan, options) {
  if (!plan || plan.schema !== SCHEMA || typeof options?.planDigest !== 'string' || !/^[a-f0-9]{64}$/u.test(options.planDigest)) fail('PURGE_FILE_PLAN_INVALID');
  const { planDigest, ...payload } = plan;
  if (planDigest !== options.planDigest || digest(payload) !== planDigest) fail('PURGE_FILE_PLAN_DIGEST_MISMATCH');
  const authoritative = scope(options);
  if (digest(authoritative) !== digest({ projectId: plan.projectId, roots: plan.roots, protectedRoots: plan.protectedRoots })) fail('PURGE_FILE_SCOPE_MISMATCH');
  limitsFor(plan.limits);
  return authoritative;
}

function previousRemoved(plan, previousResult) {
  if (!previousResult) return new Set();
  if (previousResult.projectId !== plan.projectId || previousResult.planDigest !== plan.planDigest || !Array.isArray(previousResult.removed)) fail('PURGE_FILE_RECEIPT_MISMATCH');
  const allowed = new Set(plan.entries.map(entry => entry.path));
  if (previousResult.removed.some(target => !allowed.has(target))) fail('PURGE_FILE_RECEIPT_MISMATCH');
  return new Set(previousResult.removed);
}

/** Only bounded Git administrative pointers are read; project contents are not.
 * Links are listed separately and their targets excluded.
 * The caller supplies fresh authoritative scopes and must keep this scope quiescent.
 * Path-based Node APIs cannot promise race-free deletion against a hostile writer.
 */
export async function previewProjectPurgeFiles(options) {
  const authoritative = scope(options), limits = limitsFor(options.limits);
  await checkProtectedAliases(authoritative);
  const ancestors = await ancestry(authoritative.roots);
  const tree = await manifest(authoritative.roots, limits);
  const after = await ancestry(authoritative.roots);
  if (digest(ancestors) !== digest(after)) fail('PURGE_FILE_SCOPE_CHANGED');
  const payload = { schema: SCHEMA, ...authoritative, limits, ancestors, ...tree };
  return { ...payload, planDigest: digest(payload) };
}

export async function validateProjectPurgeFiles(plan, options) {
  const authoritative = boundPlan(plan, options), removed = previousRemoved(plan, options.previousResult);
  await checkProtectedAliases(authoritative);
  const ancestors = await ancestry(authoritative.roots);
  if (digest(ancestors) !== digest(plan.ancestors)) fail('PURGE_FILE_SCOPE_CHANGED');
  const current = await manifest(authoritative.roots, plan.limits);
  const absent = new Set(current.absentRoots);
  const expected = plan.entries.filter(entry => !absent.has(entry.root) && !removed.has(entry.path));
  if (expected.length !== current.entries.length) fail('PURGE_FILE_SCOPE_CHANGED');
  for (const [index, entry] of expected.entries()) {
    const observed = current.entries[index];
    const changedByDeletion = entry.type === 'directory' && [...removed].some(target => within(target, entry.path));
    if (changedByDeletion) {
      if (observed.path !== entry.path || observed.type !== entry.type || digest(observed.identity) !== digest(entry.identity)) fail('PURGE_FILE_SCOPE_CHANGED');
    } else if (digest(entry) !== digest(observed)) fail('PURGE_FILE_SCOPE_CHANGED');
  }
  return { valid: true, absentRoots: current.absentRoots, remainingEntries: current.entries };
}

async function checkAncestors(plan, entry) {
  const checks = [
    ...plan.ancestors,
    ...plan.entries.filter(parent => parent.type === 'directory' && parent.path !== entry.path && within(entry.path, parent.path)),
  ];
  for (const expected of checks) {
    const observed = await inspect(expected.path);
    if (!observed?.isDirectory() || observed.isSymbolicLink() || digest(identity(observed)) !== digest(expected.identity)) fail('PURGE_FILE_SCOPE_CHANGED');
  }
}

function safeCode(error) {
  return typeof error?.code === 'string' && /^[A-Z][A-Z0-9_]{0,80}$/u.test(error.code) ? error.code : 'PURGE_FILE_IO_FAILED';
}

export async function readbackProjectPurgeFiles(plan, options) {
  boundPlan(plan, options);
  const absentRoots = [], presentRoots = [], errors = [];
  for (const root of plan.roots) {
    try {
      const relevant = plan.ancestors.filter(parent => within(root, parent.path));
      for (const parent of relevant) {
        const stat = await inspect(parent.path);
        if (!stat?.isDirectory() || stat.isSymbolicLink() || digest(identity(stat)) !== digest(parent.identity)) fail('PURGE_FILE_SCOPE_CHANGED');
      }
      if (await inspect(root)) presentRoots.push(root); else absentRoots.push(root);
    } catch (error) { errors.push({ path: root, code: safeCode(error) }); }
  }
  return { complete: presentRoots.length === 0 && errors.length === 0, absentRoots, presentRoots, errors };
}

export async function applyProjectPurgeFiles(plan, options) {
  // Whole-scope validation precedes the first mutation. Recursive rm is deliberately
  // avoided: an unexpected child makes rmdir fail rather than deleting new data.
  const checked = await validateProjectPurgeFiles(plan, options);
  const removed = [...previousRemoved(plan, options.previousResult)], failed = [];
  for (const entry of [...checked.remainingEntries].reverse()) {
    try {
      await checkAncestors(plan, entry);
      const stat = await inspect(entry.path);
      if (!stat || digest(identity(stat)) !== digest(entry.identity)) fail('PURGE_FILE_SCOPE_CHANGED');
      if (entry.type !== 'directory' && digest(details(stat)) !== digest({ identity: entry.identity, size: entry.size, mtimeNs: entry.mtimeNs, ctimeNs: entry.ctimeNs, nlink: entry.nlink })) fail('PURGE_FILE_SCOPE_CHANGED');
      if (entry.type === 'link' && await readlink(entry.path) !== entry.linkTarget) fail('PURGE_FILE_SCOPE_CHANGED');
      if (entry.type === 'directory') await rmdir(entry.path); else await unlink(entry.path);
      removed.push(entry.path);
    } catch (error) {
      failed.push({ path: entry.path, code: safeCode(error) });
      break;
    }
  }
  const readback = await readbackProjectPurgeFiles(plan, options);
  return { schema: SCHEMA, projectId: plan.projectId, planDigest: plan.planDigest,
    status: readback.complete && failed.length === 0 ? 'completed' : removed.length ? 'partial' : 'failed',
    removed, alreadyAbsent: checked.absentRoots, failed, readback };
}
