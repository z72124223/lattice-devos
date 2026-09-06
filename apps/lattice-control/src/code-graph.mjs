import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { createHash, randomUUID } from 'node:crypto';
import { readFile, writeFile, mkdir, rename, realpath, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadLatticeRuntimeConfiguration } from './lattice-runtime-health.mjs';
import { codeIndex, codeBranches, searchCode } from '../public/code-graph-model.mjs';

const exec = promisify(execFile), maxBytes = 64 * 1024 * 1024;
const digest = value => createHash('sha256').update(value).digest('hex');
const fail = code => Object.assign(new Error(code), { code });
const schema = 'lattice.control.code-graph.v1';
export function validateCodeGraph(value, commit) {
  if (value?.schema_version !== schema || value.source !== 'GRAPHIFY' || value.authority !== 'DERIVED'
    || value.commit !== commit || !/^[a-f0-9]{40,64}$/.test(commit)
    || !Array.isArray(value.nodes) || !Array.isArray(value.edges)
    || value.nodes.length > 100000 || value.edges.length > 250000) throw fail('CODE_GRAPH_DATA_REJECTED');
  const ids = new Set();
  const text = v => typeof v === 'string' && v.length <= 1024 && !/[\u0000-\u001f]/.test(v);
  const source = v => text(v) && !path.isAbsolute(v) && !v.split(/[\\/]/).some(p => p === '..') && !v.includes(':');
  for (const n of value.nodes) {
    if (!text(n.id) || !n.id || ids.has(n.id) || !text(n.label) || !source(n.file)
      || !text(n.location) || !text(n.kind)) throw fail('CODE_GRAPH_DATA_REJECTED');
    ids.add(n.id);
  }
  for (const e of value.edges) if (!ids.has(e.source) || !ids.has(e.target) || !text(e.relation)
    || !['EXTRACTED','INFERRED','AMBIGUOUS'].includes(e.confidence) || !source(e.file) || !text(e.location))
    throw fail('CODE_GRAPH_DATA_REJECTED');
  return value;
}

// Rebuildable code observations live in a separate cache, never the task ledger.
export class CodeGraphStore {
  constructor({ cacheDirectory = path.join(process.env.LOCALAPPDATA || process.cwd(), 'LATTICE/control/code-graphs'),
    configurationLoader = loadLatticeRuntimeConfiguration, run = exec,
    executable = fileURLToPath(new URL('../bin/lattice-code-graph.exe', import.meta.url)) } = {}) {
    Object.assign(this, { cacheDirectory, configurationLoader, run, executable });
    this.jobs = new Map(); this.cache = new Map(); this.active = null;
    this.abortController = null; this.closed = false;
  }
  async inspect(project, checkout = null) {
    const { environment: env } = await this.configurationLoader();
    const git = env.LATTICE_DELIVERY_GIT_EXE;
    if (!git || !path.isAbsolute(git)) throw fail('CODE_GRAPH_GIT_UNAVAILABLE');
    let root = await realpath(project.root_path);
    const options = { cwd: root, windowsHide: true, timeout: 15000, maxBuffer: 4 * 1024 * 1024 };
    const gitArgs = ['-c', 'core.fsmonitor=false'];
    const listing = (await this.run(git, [...gitArgs, 'worktree', 'list', '--porcelain'], options)).stdout;
    const checkouts = listing.split(/\r?\n\r?\n/).map(block => {
      const root = block.match(/^worktree (.+)$/m)?.[1]?.trim();
      const commit = block.match(/^HEAD ([a-f0-9]+)$/m)?.[1];
      if (!root || !commit || /^(?:prunable|bare)/m.test(block)) return null;
      return { id:digest(path.resolve(root).toLocaleLowerCase()), root:path.resolve(root), commit,
        label:block.match(/^branch refs\/heads\/(.+)$/m)?.[1]?.trim() || `保存版本 ${commit.slice(0, 8)}` };
    }).filter(Boolean);
    const developmentRoot = fileURLToPath(new URL('../../../', import.meta.url));
    let installedCommit = null;
    try { installedCommit = JSON.parse(await readFile(path.join(process.env.LOCALAPPDATA || '',
      'Programs/LATTICE/active-install.json'), 'utf8')).source_commit; } catch { /* No installed product. */ }
    const choice = checkout ? checkouts.find(c => c.id === checkout)
      : checkouts.find(c => path.resolve(c.root).toLocaleLowerCase() === path.resolve(developmentRoot).toLocaleLowerCase())
        || checkouts.find(c => c.commit === installedCommit) || checkouts.find(c => c.root.toLocaleLowerCase() === root.toLocaleLowerCase());
    if (!choice) throw fail('CODE_GRAPH_CHECKOUT_NOT_FOUND');
    root = await realpath(choice.root); options.cwd = root;
    const commit = (await this.run(git, [...gitArgs, 'rev-parse', '--verify', 'HEAD'], options)).stdout.trim();
    if (!/^[a-f0-9]{40,64}$/.test(commit)) throw fail('CODE_GRAPH_COMMIT_REJECTED');
    const changes = (await this.run(git, [...gitArgs, 'status', '--porcelain', '-uno'], options)).stdout.trim();
    const key = digest(`${project.id}\0${root.toLocaleLowerCase()}`);
    return { project_id: project.id, project_name: project.name, root, commit, dirty: Boolean(changes), key,
      checkout:choice.id, source_label:choice.label, checkouts,
      git, runtime: env.LATTICE_GRAPHIFY_RUNTIME_ROOT, wsl: env.LATTICE_GRAPHIFY_WSL_EXE };
  }
  async read(project, query = {}) {
    let source;
    try { source = await this.inspect(project, query.checkout); } catch { return { status:'unavailable', project_id:project.id,
      message:'目前無法讀取這個專案的已保存程式版本。工作樹仍可使用。' }; }
    const job = this.jobs.get(source.key);
    let graph = this.cache.get(source.key);
    if (!graph) {
      try {
        const filename = path.join(this.cacheDirectory, source.key, 'graph.json');
        if ((await stat(filename)).size > maxBytes) throw fail('CODE_GRAPH_DATA_REJECTED');
        const { cache_digest, ...cached } = JSON.parse(await readFile(filename, 'utf8'));
        if (cache_digest !== digest(JSON.stringify(cached))) throw fail('CODE_GRAPH_CACHE_CHANGED');
        if (cached.project_id !== source.project_id || cached.source_root !== source.root) throw fail('CODE_GRAPH_SOURCE_REJECTED');
        graph = validateCodeGraph(cached, cached.commit); this.cache.set(source.key, graph);
      } catch { /* Missing or invalid derived cache is rebuilt explicitly. */ }
    }
    const packet = { project_id:project.id, project_name:project.name, current_commit:source.commit,
      checkout:source.checkout, checkouts:source.checkouts, source_label:source.source_label, source_root:source.root,
      working_changes:source.dirty, status:job?.status || (graph ? 'ready' : 'not_analyzed'),
      error:job?.error || null, graph:graph || null,
      stale:Boolean(graph && (graph.commit !== source.commit || source.dirty)),
      message:'圖譜分析已保存的程式版本；呼叫關係是檢查線索，仍須測試確認。' };
    if (graph && (query.node || query.q)) {
      const index = codeIndex(graph);
      const q = String(query.q || '');
      const direction = query.direction || 'incoming';
      const depth = Number(query.depth || 1);
      if (q.length > 160 || !['incoming','outgoing','both'].includes(direction)
        || !Number.isInteger(depth) || depth < 1 || depth > 6) throw fail('CODE_GRAPH_QUERY_REJECTED');
      const matches = searchCode(index, q);
      const root = query.node || matches[0]?.id;
      if (query.node && !index.nodes.has(query.node)) throw fail('CODE_GRAPH_NODE_NOT_FOUND');
      let selected = codeBranches(index, root, new Set([root]), direction);
      for (let i = 1; i < depth; i++) selected = codeBranches(index, root,
        new Set(selected.nodes.map(n => n.id)), direction);
      packet.graph = { ...graph, nodes:selected.nodes, edges:selected.edges };
      packet.query = { matches, direction, depth, truncated:selected.truncated };
    }
    return packet;
  }
  async start(project, checkout = null) {
    if (this.closed) throw fail('CODE_GRAPH_CLOSED');
    const source = await this.inspect(project, checkout);
    if (this.jobs.get(source.key)?.status === 'analyzing') return { status:'analyzing', project_id:project.id };
    if (this.active || this.starting) throw fail('CODE_GRAPH_ANALYZER_BUSY');
    if (!source.runtime || !source.wsl) throw fail('CODE_GRAPH_RUNTIME_UNAVAILABLE');
    this.starting = true;
    try { await stat(this.executable); }
    catch (error) { this.starting = false; throw error; }
    if (this.closed) { this.starting = false; throw fail('CODE_GRAPH_CLOSED'); }
    const job = { status:'analyzing', error:null }; this.jobs.set(source.key, job);
    this.abortController = new AbortController();
    this.active = this.analyze(source).then(() => { job.status = 'ready'; }, error => {
      job.status = 'failed';
      const detail = String(error.stderr || error.code || 'CODE_GRAPH_ANALYSIS_FAILED').trim();
      job.error = detail.match(/\b(?:GRAPHIFY|CODE_GRAPH)_[A-Z0-9_]{1,100}\b/)?.[0] || 'CODE_GRAPH_ANALYSIS_FAILED';
    }).finally(() => { this.active = null; this.abortController = null; });
    this.starting = false;
    return { status:'analyzing', project_id:project.id };
  }
  async analyze(source) {
    const dir = path.join(this.cacheDirectory, source.key); await mkdir(dir, { recursive:true });
    const work = path.join(dir, 'runs', randomUUID());
    const { stdout } = await this.run(this.executable,
      [source.root, source.commit, source.git, source.runtime, source.wsl, work],
      { windowsHide:true, timeout:390000, maxBuffer:maxBytes, signal:this.abortController?.signal,
        env:Object.fromEntries(['SystemRoot','WINDIR','TEMP','TMP','PATH','LOCALAPPDATA','USERPROFILE']
          .filter(k => process.env[k]).map(k => [k, process.env[k]])) });
    const graph = { ...validateCodeGraph(JSON.parse(stdout), source.commit),
      project_id:source.project_id, source_root:source.root, generated_at:new Date().toISOString() };
    graph.coverage_status = graph.coverage_warnings?.length ? 'partial' : 'observed';
    const temporary = path.join(dir, `${randomUUID()}.json`);
    await writeFile(temporary, JSON.stringify({ ...graph, cache_digest:digest(JSON.stringify(graph)) }));
    await rename(temporary, path.join(dir, 'graph.json'));
    this.cache.set(source.key, graph);
  }
  async close() {
    this.closed = true; this.abortController?.abort();
    await this.active;
  }
}
