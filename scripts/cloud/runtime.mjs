#!/usr/bin/env node
// One foreground Control process. Codex owns the execution loop and process lifetime.
import { createHash, randomBytes } from 'node:crypto';
import { execFile, spawn } from 'node:child_process';
import { access, lstat, mkdir, readFile, realpath, rmdir, stat, unlink, writeFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { isDeepStrictEqual, promisify } from 'node:util';
import { CodexAppServer } from '../../apps/lattice-control/src/codex-app-server.mjs';
import { LatticeRuntimeClient } from '../../apps/lattice-control/src/lattice-runtime-client.mjs';
import { loadLatticeRuntimeConfiguration, runtimeHealthFromValue } from '../../apps/lattice-control/src/lattice-runtime-health.mjs';
import { controlDataScopeDescriptor } from '../../apps/lattice-control/src/database-path.mjs';

const exec = promisify(execFile);
const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const schema = 'lattice.cloud-installation/1';
const fail = (code) => { throw Object.assign(new Error(code), { code }); };
const digest = async (file) => createHash('sha256').update(await readFile(file)).digest('hex');
const exists = async (file) => { try { await lstat(file); return true; } catch (error) { if (error.code === 'ENOENT') return false; throw error; } };
async function privateWrite(file, value, exclusive = false) {
  if (await exists(file)) await privateFile(file);
  await writeFile(file, value, { mode: 0o600, flag: exclusive ? 'wx' : 'w' });
}

export function versionAtLeast(value, major, minor) {
  const match = /(?:^|\s|v)(\d+)\.(\d+)\.(\d+)(?:\s|$|[-+])/u.exec(value);
  return Boolean(match && (Number(match[1]) > major || (Number(match[1]) === major && Number(match[2]) >= minor)));
}

export function isOutsideRepository(candidate, repo, paths = path) {
  const relation = paths.relative(repo, candidate);
  const reverse = paths.relative(candidate, repo);
  return Boolean(relation && (relation === '..' || relation.startsWith(`..${paths.sep}`) || paths.isAbsolute(relation))
    && reverse && (reverse === '..' || reverse.startsWith(`..${paths.sep}`) || paths.isAbsolute(reverse)));
}

export function checkedPort(value, fallback) {
  const port = value === undefined ? fallback : Number(value);
  if (!Number.isInteger(port) || port < 1024 || port > 65535 || port === 5432) fail('CLOUD_PORT_REJECTED');
  return port;
}

export function cleanEnvironment(input = process.env) {
  return Object.fromEntries(Object.entries(input).filter(([name]) => !name.startsWith('LATTICE_')
    && !name.startsWith('JEV_') && !name.startsWith('PG')));
}

async function command(executable, args, options = {}) {
  try {
    return await exec(executable, args, { timeout: 120_000, maxBuffer: 4 * 1024 * 1024,
      env: cleanEnvironment(), windowsHide: true, ...options });
  } catch (error) {
    // Never echo environment, command arguments, SQL errors, or account payloads.
    const result = Object.assign(new Error('CLOUD_COMMAND_FAILED'), { code: 'CLOUD_COMMAND_FAILED', exitCode: error.code });
    result.safeCode = String(error.stderr ?? '').split(/\r?\n/u).findLast((line) => /^(LATTICE|GRAPHIFY|GRAPH_USAGE)_[A-Z0-9_]{1,150}$/u.test(line));
    throw result;
  }
}

async function executable(name, explicit) {
  const candidates = explicit ? [explicit] : (process.env.PATH ?? '').split(path.delimiter).map((entry) => path.join(entry, name));
  for (const file of candidates) {
    if (!path.isAbsolute(file)) continue;
    try { await access(file, constants.X_OK); if ((await stat(file)).isFile()) return await realpath(file); } catch { /* try next */ }
  }
  fail(`CLOUD_${name.toUpperCase().replaceAll('-', '_')}_MISSING`);
}

async function stateRoot({ create = false } = {}) {
  const supplied = process.env.LATTICE_CLOUD_STATE_ROOT;
  if (!supplied || !path.isAbsolute(supplied)) fail('CLOUD_STATE_ROOT_ABSOLUTE_REQUIRED');
  const repo = await realpath(repository);
  const lexical = path.resolve(supplied);
  if (!isOutsideRepository(lexical, repo)) fail('CLOUD_STATE_ROOT_MUST_BE_OUTSIDE_REPOSITORY');
  if (create && !(await exists(lexical))) await mkdir(lexical, { mode: 0o700 });
  const info = await lstat(lexical);
  const canonical = await realpath(lexical);
  if (!info.isDirectory() || info.isSymbolicLink() || !isOutsideRepository(canonical, repo)
    || info.uid !== process.getuid() || (info.mode & 0o077) !== 0) fail('CLOUD_STATE_ROOT_NOT_PRIVATE');
  return canonical;
}

async function privateFile(file) {
  const info = await lstat(file);
  if (!info.isFile() || info.isSymbolicLink() || info.uid !== process.getuid() || (info.mode & 0o077) !== 0) fail('CLOUD_PRIVATE_FILE_REJECTED');
}

export function validateManagedPathEntry({ root, target, canonical, info, directory, ownerId }) {
  const relative = path.relative(root, canonical);
  if (info.isSymbolicLink() || !(directory ? info.isDirectory() : info.isFile())
    || info.uid !== ownerId || (info.mode & 0o077) !== 0 || canonical !== target
    || !relative || relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
    fail('CLOUD_MANAGED_PATH_REJECTED');
  }
}

export async function validateManagedPaths(data, { ownerId = process.getuid(), inspect = lstat, canonicalize = realpath } = {}) {
  const directories = ['delivery', 'graph', 'dependencies', 'postgres'];
  const files = ['control.sqlite', 'control.sqlite-wal', 'control.sqlite-shm', 'postgres.log',
    'runtime.toml', 'installation.json', 'graphify-refresh.json',
    ...['postgresql.conf', 'postgresql.auto.conf', 'pg_hba.conf', 'pg_ident.conf'].map((name) => path.join('postgres', name))];
  for (const [name, directory] of [...directories.map((name) => [name, true]), ...files.map((name) => [name, false])]) {
    const target = path.join(data.root, name);
    let info;
    try { info = await inspect(target); } catch (error) { if (error.code === 'ENOENT') continue; throw error; }
    validateManagedPathEntry({ root: data.root, target, canonical: await canonicalize(target), info, directory, ownerId });
  }
}

export async function validateCodexPath(codex, { canonicalize = realpath, checkAccess = access, inspect = stat } = {}) {
  const entry = path.join(path.dirname(codex), 'codex');
  try {
    await checkAccess(entry, constants.X_OK);
    if (!(await inspect(entry)).isFile() || await canonicalize(entry) !== codex) fail('CLOUD_CODEX_PATH_BINDING_REJECTED');
  } catch { fail('CLOUD_CODEX_PATH_BINDING_REJECTED'); }
}

async function load(root) {
  const file = path.join(root, 'installation.json');
  await privateFile(file);
  const data = JSON.parse(await readFile(file, 'utf8'));
  if (data.schema !== schema || data.root !== root || data.repository !== await realpath(repository)
    || !/^[a-f0-9]{32}$/u.test(data.runId) || !/^[a-f0-9]{64}$/u.test(data.password)
    || data.pgPort === data.controlPort) fail('CLOUD_INSTALLATION_REJECTED');
  checkedPort(data.pgPort); checkedPort(data.controlPort);
  await validateManagedPaths(data);
  await validateCodexPath(data.codex);
  for (const [filePath, expected] of Object.entries(data.files)) {
    if (!path.isAbsolute(filePath) || await digest(filePath) !== expected) fail('CLOUD_COMPONENT_CHANGED');
  }
  await privateFile(path.join(root, 'runtime.toml'));
  return data;
}

export function runtimeEnvironment(data) {
  const env = {
    LATTICE_TASK019_HOST: '127.0.0.1', LATTICE_TASK019_PORT: String(data.pgPort),
    LATTICE_TASK019_RUN_ID: data.runId, LATTICE_TASK019_PASSWORD: data.password,
    LATTICE_DELIVERY_CODEX_MODE: 'OFFICIAL_CODEX_APP_SERVER',
    LATTICE_FULL_CHAIN_RUN_MODE: 'RESUME_EXISTING', LATTICE_RUNTIME_INTEGRATION: 'GRAPHIFY',
    LATTICE_DELIVERY_ROOT: path.join(data.root, 'delivery'),
    LATTICE_DELIVERY_SCHEMA_DIR: path.join(data.root, 'delivery', 'schema'),
    LATTICE_DELIVERY_CODEX_HOME: data.codexHome, LATTICE_DELIVERY_LAUNCHER: data.codex,
    LATTICE_DELIVERY_LAUNCHER_VERSION: data.codexVersion, LATTICE_DELIVERY_LAUNCHER_SHA256: data.files[data.codex],
    LATTICE_DELIVERY_GIT_EXE: data.git, LATTICE_DELIVERY_TIMEOUT_SECONDS: '120',
    LATTICE_GRAPHIFY_SOURCE_ROOT: data.repository, LATTICE_GRAPHIFY_WORK_ROOT: path.join(data.root, 'graph'),
    LATTICE_DEPENDENCY_WORKTREE_ROOT: path.join(data.root, 'dependencies'),
    LATTICE_CONTROL_ORIGIN: `http://127.0.0.1:${data.controlPort}`,
  };
  if (data.graphifyRoot) env.LATTICE_GRAPHIFY_RUNTIME_ROOT = data.graphifyRoot;
  return env;
}

export function childEnvironment(data, input = process.env) {
  return { ...cleanEnvironment(input), CODEX_HOME: data.codexHome,
    PATH: `${path.dirname(data.codex)}${path.delimiter}${input.PATH ?? ''}`,
    LATTICE_RUNTIME_CONFIG_PATH: path.join(data.root, 'runtime.toml'),
    LATTICE_CONTROL_DATABASE_PATH: path.join(data.root, 'control.sqlite'),
    LATTICE_CONTROL_PORT: String(data.controlPort) };
}

export function runtimeChildEnvironment(data, input = process.env) {
  return { ...cleanEnvironment(input), ...runtimeEnvironment(data), CODEX_HOME: data.codexHome };
}

async function setup(build) {
  const root = await stateRoot({ create: true });
  if (await exists(path.join(root, 'installation.json'))) {
    await load(root);
    return { status: 'CONFIGURED', root, reused: true, serviceStarted: false, overallReady: false };
  }
  const rustc = await executable('rustc');
  const rustVersion = (await command(rustc, ['--version'])).stdout.trim();
  if (!/^rustc 1\.97\.\d+(?:\s|$)/u.test(rustVersion)) fail('CLOUD_RUST_1_97_REQUIRED');
  const pgBin = process.env.LATTICE_PG_BIN;
  if (!pgBin || !path.isAbsolute(pgBin)) fail('CLOUD_PG_BIN_ABSOLUTE_REQUIRED');
  const pg = {};
  for (const name of ['initdb', 'pg_ctl', 'pg_controldata', 'postgres', 'psql']) pg[name] = await executable(name, path.join(pgBin, name));
  const pgVersion = (await command(pg.postgres, ['--version'])).stdout.trim();
  const git = await executable('git');
  const codex = await executable('codex', process.env.LATTICE_CODEX_BIN);
  await validateCodexPath(codex);
  const codexVersion = (await command(codex, ['--version'])).stdout.trim();
  const codexHome = await realpath(process.env.CODEX_HOME ?? path.join(homedir(), '.codex'));
  if (build) {
    const cargo = await executable('cargo');
    await command(cargo, ['build', '--locked', '--release', '-p', 'lattice-runtime', '--bin', 'latticed'], {
      cwd: repository, env: { ...cleanEnvironment(), CARGO_TARGET_DIR: path.join(root, 'target') }, timeout: 20 * 60_000,
    });
  }
  const runtime = await executable('latticed', process.env.LATTICE_LATTICED ?? path.join(root, 'target', 'release', 'latticed'));
  const pgPort = checkedPort(process.env.LATTICE_CLOUD_PG_PORT, 55432);
  const controlPort = checkedPort(process.env.LATTICE_CONTROL_PORT, 4317);
  if (pgPort === controlPort) fail('CLOUD_PORT_COLLISION');
  const cluster = path.join(root, 'postgres');
  if (await exists(cluster)) fail('CLOUD_UNOWNED_CLUSTER_PRESENT');
  const data = { schema, root, repository: await realpath(repository), runtime, pg, git, codex, codexHome,
    pgPort, controlPort, runId: randomBytes(16).toString('hex'), password: randomBytes(32).toString('hex'),
    codexVersion, rustVersion, pgVersion, nodeVersion: process.version, graphifyRoot: null, files: {} };
  await validateManagedPaths(data);
  if (process.env.LATTICE_GRAPHIFY_RUNTIME_ROOT) {
    if (!path.isAbsolute(process.env.LATTICE_GRAPHIFY_RUNTIME_ROOT)) fail('CLOUD_GRAPHIFY_ROOT_ABSOLUTE_REQUIRED');
    data.graphifyRoot = await realpath(process.env.LATTICE_GRAPHIFY_RUNTIME_ROOT).catch((error) => {
      if (error.code === 'ENOENT') return path.resolve(process.env.LATTICE_GRAPHIFY_RUNTIME_ROOT);
      throw error;
    });
  }
  for (const directory of ['delivery', 'graph', 'dependencies']) await mkdir(path.join(root, directory), { recursive: true, mode: 0o700 });
  const passwordFile = path.join(root, 'initdb-password');
  await privateWrite(passwordFile, `${data.password}\n`, true);
  try {
    await command(pg.initdb, ['-D', cluster, '-U', 'runtime_bootstrap', '--auth-host=scram-sha-256',
      '--auth-local=reject', '--encoding=UTF8', '--locale=C', '--data-checksums', `--pwfile=${passwordFile}`]);
  } finally { await unlink(passwordFile); }
  // Override defaults, then inspect effective values before any listener starts.
  const pgConfig = path.join(cluster, 'postgresql.conf');
  await privateWrite(pgConfig, `${await readFile(pgConfig, 'utf8')}\nlisten_addresses = '127.0.0.1'\nport = ${pgPort}\nunix_socket_directories = ''\n`);
  const control = (await command(pg.pg_controldata, [cluster], { env: { ...cleanEnvironment(), LC_ALL: 'C' } })).stdout;
  data.systemId = /^Database system identifier:\s+(\d+)\s*$/mu.exec(control)?.[1];
  if (!data.systemId) fail('CLOUD_CLUSTER_IDENTITY_MISSING');
  const files = [runtime, codex, git, ...Object.values(pg), ...['postgresql.conf', 'postgresql.auto.conf', 'pg_hba.conf', 'pg_ident.conf'].map((name) => path.join(cluster, name))];
  for (const file of files) data.files[file] = await digest(file);
  const toml = `[mcp_servers.lattice]\ncommand = ${JSON.stringify(runtime)}\nenabled = true\nrequired = true\nstartup_timeout_sec = 120\ntool_timeout_sec = 120\n\n[mcp_servers.lattice.env]\n`
    + Object.entries(runtimeEnvironment(data)).map(([name, value]) => `${name} = ${JSON.stringify(value)}\n`).join('');
  await privateWrite(path.join(root, 'runtime.toml'), toml, true);
  data.files[path.join(root, 'runtime.toml')] = await digest(path.join(root, 'runtime.toml'));
  await privateWrite(path.join(root, 'installation.json'), `${JSON.stringify(data, null, 2)}\n`, true);
  return { status: 'CONFIGURED', root, reused: false, serviceStarted: false, overallReady: false,
    versions: { node: process.version, rust: rustVersion, postgres: pgVersion, codex: codexVersion }, graphify: 'NOT_VERIFIED', jev: 'EXCLUDED' };
}

async function postgresReadback(data) {
  const sql = "SELECT json_build_object('id',system_identifier::text,'data',current_setting('data_directory'),'port',current_setting('port')::int,'listen',current_setting('listen_addresses'),'sockets',current_setting('unix_socket_directories')) FROM pg_control_system();";
  const { stdout } = await command(data.pg.psql, ['-X', '-h', '127.0.0.1', '-p', String(data.pgPort), '-U', 'runtime_bootstrap',
    '-d', 'postgres', '-A', '-t', '-v', 'ON_ERROR_STOP=1', '-c', sql], {
    env: { ...cleanEnvironment(), PGPASSWORD: data.password, PGCONNECT_TIMEOUT: '5' },
  });
  const value = JSON.parse(stdout);
  if (value.id !== data.systemId || value.data !== path.join(data.root, 'postgres') || value.port !== data.pgPort
    || value.listen !== '127.0.0.1' || value.sockets !== '') fail('CLOUD_CLUSTER_IDENTITY_REJECTED');
  return { status: 'VERIFIED', host: '127.0.0.1', port: data.pgPort, systemId: data.systemId };
}

async function startPostgres(data) {
  const cluster = path.join(data.root, 'postgres');
  for (const [name, expected] of Object.entries({ listen_addresses: '127.0.0.1', port: String(data.pgPort), data_directory: cluster, unix_socket_directories: '' })) {
    if ((await command(data.pg.postgres, ['-C', name, '-D', cluster])).stdout.trim() !== expected) fail('CLOUD_POSTGRES_EFFECTIVE_CONFIG_REJECTED');
  }
  let running = false;
  try { await command(data.pg.pg_ctl, ['-D', cluster, 'status']); running = true; }
  catch (error) { if (error.exitCode !== 3) throw error; }
  if (!running) await command(data.pg.pg_ctl, ['-D', cluster, '-l', path.join(data.root, 'postgres.log'), '-w', '-t', '30', 'start']);
  return postgresReadback(data);
}

async function codexReadback(data) {
  await validateCodexPath(data.codex);
  const client = new CodexAppServer({ codexBin: data.codex,
    spawnProcess: (file, args, options) => spawn(file, args, { ...options, env: childEnvironment(data) }) });
  try { const auth = await client.readAuthReadiness(); return { ready: auth.ready, authMode: auth.authMode }; }
  finally { await client.close(); }
}

async function graphifyPreflight(data) {
  if (!data.graphifyRoot) return { status: 'DEGRADED', code: 'GRAPHIFY_RUNTIME_NOT_PROVISIONED' };
  try {
    await command(data.runtime, ['--graphify-runtime-preflight'], { env: runtimeChildEnvironment(data) });
    return { status: 'IDENTITY_VERIFIED' };
  } catch (error) { return { status: 'DEGRADED', code: error.safeCode ?? 'GRAPHIFY_PREFLIGHT_REJECTED' }; }
}

export function classifyReadiness({ postgres, control, runtime, codex, graphify }) {
  const coreReady = postgres?.status === 'VERIFIED' && control?.identityVerified === true && control?.formal_work_enabled === true
    && control?.life_harness?.enabled === true && control?.formal_startup?.mode === 'explicit-only'
    && runtime?.postgresql === 'HEALTHY' && runtime?.integration === 'GRAPHIFY' && codex?.ready === true;
  const overallReady = coreReady && graphify?.status === 'VERIFIED';
  return { coreReady, overallReady, status: overallReady ? 'READY' : coreReady ? 'DEGRADED' : 'UNAVAILABLE',
    formalTaskAcceptance: 'NOT_RUN', lifeHarnessAcceptance: 'NOT_RUN', jev: 'EXCLUDED' };
}

export function validGraphifyReceipt(receipt, sourceHead) {
  return receipt?.component === 'graphify' && receipt.status === 'PERSISTED' && receipt.commit === sourceHead
    && receipt.usage_outcome === 'ANALYZED' && Number.isInteger(receipt.analysis_calls) && receipt.analysis_calls > 0
    && typeof receipt.receipt_digest === 'string' && receipt.receipt_digest.length > 0;
}

async function attempt(operation) {
  try { return await operation(); } catch (error) { return { status: 'UNAVAILABLE', code: /^[A-Z][A-Z0-9_]+$/u.test(error.code ?? '') ? error.code : 'CLOUD_CHECK_FAILED' }; }
}

export async function controlReadback(data, fetchResponse = fetch) {
  const origin = `http://127.0.0.1:${data.controlPort}`;
  const get = async (route) => {
    const url = `${origin}${route}`;
    const response = await fetchResponse(url, { signal: AbortSignal.timeout(10_000), redirect: 'error' });
    if (!response.ok || response.url !== url) fail('CLOUD_CONTROL_HTTP_REJECTED');
    return response.json();
  };
  const expected = controlDataScopeDescriptor(path.join(data.root, 'control.sqlite'));
  const surface = await get('/api/runtime');
  if (surface.schema_version !== 'lattice.control.runtime-surface.v2'
    || !isDeepStrictEqual(surface.data_scope, expected)) fail('CLOUD_CONTROL_SCOPE_REJECTED');
  const state = await get('/api/state');
  // Recheck after capabilities so a normal replacement at this port is not silently accepted.
  const after = await get('/api/runtime');
  if (after.schema_version !== surface.schema_version || !isDeepStrictEqual(after.data_scope, expected)) fail('CLOUD_CONTROL_SCOPE_REJECTED');
  return { identityVerified: true, origin, data_scope: expected,
    formal_work_enabled: state.formal_work_enabled, life_harness: state.life_harness, formal_startup: state.formal_startup };
}

async function ready(data) {
  const postgres = await attempt(() => postgresReadback(data));
  const control = await attempt(() => controlReadback(data));
  const runtime = await attempt(async () => {
    const client = new LatticeRuntimeClient({ configurationLoader: () => loadLatticeRuntimeConfiguration({ configPath: path.join(data.root, 'runtime.toml') }) });
    try {
      const value = await client.call('lattice_runtime_status', {});
      return { ...runtimeHealthFromValue(value), integration: value.runtime_integration, graphifyConfiguration: value.graphify_runtime_status };
    } finally { await client.close(); }
  });
  const codex = await attempt(() => codexReadback(data));
  let graphify = await graphifyPreflight(data);
  if (graphify.status === 'IDENTITY_VERIFIED') {
    const receipt = await attempt(async () => JSON.parse(await readFile(path.join(data.root, 'graphify-refresh.json'), 'utf8')));
    const head = (await command(data.git, ['-C', data.repository, 'rev-parse', 'HEAD'])).stdout.trim();
    graphify = receipt.runtimeSha256 === data.files[data.runtime] && receipt.sourceHead === head
      && receipt.graphifyRoot === data.graphifyRoot && receipt.status === 'VERIFIED' && validGraphifyReceipt(receipt.receipt, head)
      && runtime.integration === 'GRAPHIFY' && ['READY', 'PREPARED'].includes(runtime.graphifyConfiguration)
      ? { status: 'VERIFIED', observedAt: receipt.observedAt, receipt: path.join(data.root, 'graphify-refresh.json') }
      : { status: 'DEGRADED', code: 'GRAPHIFY_EXECUTION_RECEIPT_REQUIRED' };
  }
  return { ...classifyReadiness({ postgres, control, runtime, codex, graphify }), observedAt: new Date().toISOString(),
    root: data.root, postgres, control, runtime, codex, graphify };
}

async function start(data) {
  await validateManagedPaths(data);
  await validateCodexPath(data.codex);
  await startPostgres(data);
  const env = runtimeChildEnvironment(data);
  for (const action of ['--postgres-initialize', '--postgres-bootstrap']) await command(data.runtime, [action], { env });
  const codex = await attempt(() => codexReadback(data));
  let graphify = await graphifyPreflight(data);
  if (graphify.status === 'IDENTITY_VERIFIED') {
    graphify = await attempt(async () => {
      const sourceHead = (await command(data.git, ['-C', data.repository, 'rev-parse', 'HEAD'])).stdout.trim();
      const { stdout } = await command(data.runtime, ['--graphify-refresh'], { env });
      const receipt = JSON.parse(stdout);
      if (!validGraphifyReceipt(receipt, sourceHead)) return { status: 'DEGRADED', code: 'GRAPHIFY_FRESH_ANALYSIS_NOT_OBSERVED' };
      await privateWrite(path.join(data.root, 'graphify-refresh.json'), JSON.stringify({ status: 'VERIFIED', observedAt: new Date().toISOString(),
        runtimeSha256: data.files[data.runtime], graphifyRoot: data.graphifyRoot, sourceHead, receipt }, null, 2));
      return { status: 'VERIFIED' };
    });
  }
  await validateManagedPaths(data);
  await validateCodexPath(data.codex);
  console.log(JSON.stringify({ status: 'CONTROL_STARTING', codex, graphify, overallReady: false, jev: 'EXCLUDED' }));
  const child = spawn(process.execPath, [path.join(repository, 'apps/lattice-control/src/server.mjs'), '--no-auto-restore'], { env: childEnvironment(data), stdio: 'inherit' });
  for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => child.kill(signal));
  return new Promise((resolve, reject) => {
    child.once('error', () => reject(Object.assign(new Error('CLOUD_CONTROL_SPAWN_FAILED'), { code: 'CLOUD_CONTROL_SPAWN_FAILED' })));
    child.once('exit', (code) => { process.exitCode = code ?? 1; resolve(null); });
  });
}

async function main() {
  if (process.platform !== 'linux') fail('CLOUD_LINUX_REQUIRED');
  if (process.getuid() === 0) fail('CLOUD_NONROOT_USER_REQUIRED');
  if (!versionAtLeast(process.version, 24, 15)) fail('CLOUD_NODE_24_15_REQUIRED');
  const [action, flag, ...extra] = process.argv.slice(2);
  if (!['setup', 'start', 'ready'].includes(action) || extra.length || (flag && !(action === 'setup' && flag === '--build'))) fail('CLOUD_ARGUMENTS_REJECTED');
  process.umask(0o077);
  const root = await stateRoot({ create: action === 'setup' });
  if (action === 'ready') {
    const result = await ready(await load(root));
    if (!result.overallReady) process.exitCode = 2;
    return result;
  }
  const lock = path.join(root, '.operation');
  try { await mkdir(lock, { mode: 0o700 }); } catch { fail('CLOUD_OPERATION_ALREADY_ACTIVE'); }
  try { return await (action === 'setup' ? setup(flag === '--build') : start(await load(root))); }
  finally { await rmdir(lock); }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().then((value) => { if (value) console.log(JSON.stringify(value, null, 2)); }).catch((error) => {
    console.error(JSON.stringify({ status: 'REJECTED', code: /^[A-Z][A-Z0-9_]+$/u.test(error.code ?? '') ? error.code : 'CLOUD_OPERATION_FAILED', detail: error.safeCode ?? null }));
    process.exitCode = 1;
  });
}
