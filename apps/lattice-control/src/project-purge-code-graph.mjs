import { createHash } from 'node:crypto';
import { constants } from 'node:fs';
import { lstat, open, opendir, realpath } from 'node:fs/promises';
import path from 'node:path';
import { validateCodeGraph } from './code-graph.mjs';

const SCHEMA = 'lattice.project-purge-control-code-graph.v1';
const MAX_CHILDREN = 4096, MAX_BYTES = 64 * 1024 * 1024;
const hash = value => createHash('sha256').update(value).digest('hex');
const compare = (a, b) => a < b ? -1 : a > b ? 1 : 0;
const pathKey = value => process.platform === 'win32' ? value.toLowerCase() : value;
const fail = (code, extra = {}) => { throw Object.assign(new Error(code), { code, ...extra }); };
const hex = value => typeof value === 'string' && /^[a-f0-9]{64}$/u.test(value);

function projectIdentity(value) {
  if (typeof value !== 'string' || !value.trim() || value.length > 128 || /[\x00-\x1f\x7f]/u.test(value)) fail('PURGE_CODE_GRAPH_PROJECT_INVALID');
  return value;
}
function absolute(value) {
  if (typeof value !== 'string' || !value || Buffer.byteLength(value) > 4096 || value.trim() !== value
    || /[\x00-\x1f\x7f]/u.test(value) || !path.isAbsolute(value)
    || value.split(/[\\/]/u).some(part => part === '.' || part === '..')) fail('PURGE_CODE_GRAPH_PATH_INVALID');
  if (process.platform === 'win32' && (!/^[A-Za-z]:[\\/]/u.test(value) || value.slice(2).includes(':')
    || value.split(/[\\/]/u).some(part => /[. ]$/u.test(part)))) fail('PURGE_CODE_GRAPH_PATH_INVALID');
  return path.resolve(value);
}
async function inspect(target) {
  try { return await lstat(target, { bigint: true }); }
  catch (error) { if (error.code === 'ENOENT') return null; throw error; }
}
function identity(stat) {
  return [stat.dev, stat.ino, stat.birthtimeNs, stat.mode].map(String).join(':');
}
function version(stat) {
  return [identity(stat), stat.size, stat.mtimeNs, stat.ctimeNs, stat.nlink].map(String).join(':');
}
async function directory(target, stat) {
  if (!stat || stat.isSymbolicLink() || !stat.isDirectory()) fail('PURGE_CODE_GRAPH_UNSAFE_DIRECTORY');
  if (pathKey(await realpath(target)) !== pathKey(target)) fail('PURGE_CODE_GRAPH_PATH_ALIAS');
}
// Check every existing ancestor before resolving descendants. Missing caches are
// observations only: this module never creates directories or follows a link.
async function ancestry(target) {
  const root = path.parse(target).root, parts = path.relative(root, target).split(path.sep).filter(Boolean);
  const entries = [];
  let current = root, stat;
  for (let index = 0; index <= parts.length; index++) {
    if (index) current = path.join(current, parts[index - 1]);
    stat = await inspect(current);
    if (!stat) return { entries, stat: null };
    await directory(current, stat);
    entries.push([current, identity(stat)]);
  }
  return { entries, stat };
}
async function graphHeader(root, budget) {
  const filename = path.join(root, 'graph.json'), before = await inspect(filename);
  if (!before) fail('PURGE_CODE_GRAPH_MANIFEST_MISSING');
  if (before.isSymbolicLink() || !before.isFile() || before.nlink !== 1n) fail('PURGE_CODE_GRAPH_UNSAFE_MANIFEST');
  if (pathKey(await realpath(filename)) !== pathKey(filename)) fail('PURGE_CODE_GRAPH_PATH_ALIAS');
  if (before.size > BigInt(budget.remaining)) fail('PURGE_CODE_GRAPH_READ_LIMIT');
  const handle = await open(filename, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  let content;
  try {
    if (version(await handle.stat({ bigint: true })) !== version(before)) fail('PURGE_CODE_GRAPH_SCOPE_CHANGED');
    const buffer = Buffer.alloc(Number(before.size));
    let count = 0;
    while (count < buffer.length) {
      const { bytesRead } = await handle.read(buffer, count, buffer.length - count, count);
      if (!bytesRead) break;
      count += bytesRead;
      budget.remaining -= bytesRead;
    }
    if (count !== Number(before.size) || budget.remaining < 0
      || version(await handle.stat({ bigint: true })) !== version(before)) fail('PURGE_CODE_GRAPH_SCOPE_CHANGED');
    content = buffer.subarray(0, count).toString('utf8');
  } finally { await handle.close(); }
  const after = await inspect(filename);
  if (!after || version(after) !== version(before)) fail('PURGE_CODE_GRAPH_SCOPE_CHANGED');
  let parsed;
  try { parsed = JSON.parse(content); } catch { fail('PURGE_CODE_GRAPH_MANIFEST_INVALID'); }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) fail('PURGE_CODE_GRAPH_MANIFEST_INVALID');
  const { cache_digest: cacheDigest, ...graph } = parsed;
  if (!hex(cacheDigest) || cacheDigest !== hash(JSON.stringify(graph))) fail('PURGE_CODE_GRAPH_DIGEST_MISMATCH');
  try {
    validateCodeGraph(graph, graph.commit);
    projectIdentity(graph.project_id);
    if (absolute(graph.source_root) !== graph.source_root) fail('PURGE_CODE_GRAPH_PATH_INVALID');
  } catch { fail('PURGE_CODE_GRAPH_MANIFEST_INVALID'); }
  return { projectId: graph.project_id, sourceRoot: graph.source_root, cacheDigest };
}

async function inventory({ cacheDirectory, projectId }, skipRoots = new Set()) {
  cacheDirectory = absolute(cacheDirectory); projectId = projectIdentity(projectId);
  const result = { schema: SCHEMA, projectId, cacheDirectory, roots: [], bindings: [], blockers: [], discovery: 'COMPLETE' };
  const presentRoots = [], budget = { remaining: MAX_BYTES };
  const blocked = (error, target) => result.blockers.push({ code: /^PURGE_CODE_GRAPH_[A-Z_]+$/u.test(error.code ?? '')
    ? error.code : 'PURGE_CODE_GRAPH_READ_FAILED', path: target });
  try {
    const before = await ancestry(cacheDirectory);
    if (before.stat) {
      const names = [];
      for await (const child of await opendir(cacheDirectory)) {
        if (names.length >= MAX_CHILDREN) fail('PURGE_CODE_GRAPH_ENTRY_LIMIT');
        names.push(child.name);
      }
      names.sort(compare);
      for (const name of names) {
        const root = path.join(cacheDirectory, name);
        presentRoots.push(root);
        try {
          if (!hex(name) || absolute(root) !== root) fail('PURGE_CODE_GRAPH_UNKNOWN_ENTRY');
          const stat = await inspect(root);
          await directory(root, stat);
          // Only the coordinator's independently validated file manifest permits
          // partially removed planned roots to skip their now-missing header.
          if (skipRoots.has(root)) continue;
          const graph = await graphHeader(root, budget);
          if (name !== hash(`${graph.projectId}\0${graph.sourceRoot.toLocaleLowerCase()}`)) fail('PURGE_CODE_GRAPH_OWNER_MISMATCH');
          const after = await inspect(root);
          if (!after || version(after) !== version(stat)) fail('PURGE_CODE_GRAPH_SCOPE_CHANGED');
          if (graph.projectId === projectId) {
            result.roots.push(root);
            result.bindings.push({ root, sourceRoot: graph.sourceRoot, key: name, cacheDigest: graph.cacheDigest });
          }
        } catch (error) { blocked(error, root); }
      }
      const after = await inspect(cacheDirectory);
      if (!after || version(after) !== version(before.stat)) fail('PURGE_CODE_GRAPH_SCOPE_CHANGED');
    }
    const after = await ancestry(cacheDirectory);
    if (JSON.stringify(before.entries) !== JSON.stringify(after.entries)) fail('PURGE_CODE_GRAPH_SCOPE_CHANGED');
  } catch (error) { blocked(error, cacheDirectory); }
  result.discovery = result.blockers.length ? 'BLOCKED' : 'COMPLETE';
  return { result, presentRoots };
}

function boundPlan(plan) {
  if (!plan || plan.schema !== SCHEMA || plan.discovery !== 'COMPLETE' || !Array.isArray(plan.blockers) || plan.blockers.length
    || !Array.isArray(plan.roots) || !Array.isArray(plan.bindings) || plan.roots.length > MAX_CHILDREN
    || plan.roots.length !== plan.bindings.length) fail('PURGE_CODE_GRAPH_PLAN_INVALID');
  const cacheDirectory = absolute(plan.cacheDirectory), projectId = projectIdentity(plan.projectId);
  if (cacheDirectory !== plan.cacheDirectory || new Set(plan.roots).size !== plan.roots.length) fail('PURGE_CODE_GRAPH_PLAN_INVALID');
  for (const [index, root] of plan.roots.entries()) {
    const binding = plan.bindings[index];
    if (absolute(root) !== root || path.dirname(root) !== cacheDirectory || !binding || binding.root !== root
      || !hex(binding.key) || path.basename(root) !== binding.key || !hex(binding.cacheDigest)
      || absolute(binding.sourceRoot) !== binding.sourceRoot
      || hash(`${projectId}\0${binding.sourceRoot.toLocaleLowerCase()}`) !== binding.key) fail('PURGE_CODE_GRAPH_PLAN_INVALID');
  }
  return { cacheDirectory, projectId };
}

/** Read-only ownership discovery. The caller binds projectId to its authoritative
 * Registry identity and keeps Control/analyzers offline throughout validation and
 * file removal. The workflow digest protects this plan; Node path operations do
 * not provide a race-free boundary against a hostile concurrent writer. */
export async function previewControlCodeGraphPurge(options) {
  return (await inventory(options)).result;
}

export async function validateControlCodeGraphPurge(plan, { allowPlannedPartial = false } = {}) {
  const scope = boundPlan(plan);
  if (typeof allowPlannedPartial !== 'boolean') fail('PURGE_CODE_GRAPH_PLAN_INVALID');
  const { result, presentRoots } = await inventory(scope, allowPlannedPartial ? new Set(plan.roots) : new Set());
  if (result.blockers.length) fail('PURGE_CODE_GRAPH_BLOCKED', { blockers: result.blockers });
  const expected = new Map(plan.bindings.map(binding => [binding.root, binding]));
  for (const binding of result.bindings) {
    if (JSON.stringify(binding) !== JSON.stringify(expected.get(binding.root))) fail('PURGE_CODE_GRAPH_SCOPE_CHANGED');
  }
  // A formerly owned directory reassigned to another valid project is also drift.
  if (!allowPlannedPartial && plan.roots.some(root => presentRoots.includes(root) && !result.roots.includes(root))) fail('PURGE_CODE_GRAPH_SCOPE_CHANGED');
  return { valid: true, absentRoots: plan.roots.filter(root => !presentRoots.includes(root)) };
}

export async function readbackControlCodeGraphPurge(plan) {
  const { result, presentRoots } = await inventory(boundPlan(plan));
  const remainingPlannedRoots = plan.roots.filter(root => presentRoots.includes(root));
  return { ...result, remainingPlannedRoots,
    complete: result.discovery === 'COMPLETE' && !result.roots.length && !remainingPlannedRoots.length };
}
