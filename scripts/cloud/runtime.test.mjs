import assert from 'node:assert/strict';
import test from 'node:test';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtemp, realpath, rmdir, unlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { promisify } from 'node:util';
import { controlDataScopeDescriptor } from '../../apps/lattice-control/src/database-path.mjs';
import { checkedPort, childEnvironment, classifyReadiness, cleanEnvironment, controlReadback, isOutsideRepository,
  runtimeChildEnvironment, runtimeEnvironment, validateCodexPath, validateManagedPaths, validGraphifyReceipt, versionAtLeast } from './runtime.mjs';

test('Node lower bound includes current patch releases and rejects older runtime', () => {
  for (const version of ['v24.15.0', 'v24.19.0', 'v25.0.0']) assert.equal(versionAtLeast(version, 24, 15), true);
  for (const version of ['v24.14.9', 'v22.99.0', 'not-a-version']) assert.equal(versionAtLeast(version, 24, 15), false);
});

test('private root must be neither repository descendant nor ancestor', () => {
  for (const root of ['/workspace/repo', '/workspace/repo/state', '/workspace']) assert.equal(isOutsideRepository(root, '/workspace/repo', path.posix), false);
  assert.equal(isOutsideRepository('/workspace/state', '/workspace/repo', path.posix), true);
});

test('listener ports reject public defaults, ambiguous values and collisions at caller', () => {
  assert.equal(checkedPort(undefined, 55432), 55432);
  assert.equal(checkedPort('4317'), 4317);
  for (const value of ['5432', '0', '443', '65536', '1.5', 'NaN', '']) assert.throws(() => checkedPort(value));
});

test('cloud child does not inherit JEV, old LATTICE bindings or external PG options', () => {
  assert.deepEqual(cleanEnvironment({ PATH: '/bin', CODEX_HOME: '/workspace/auth', JEV_ENABLED: 'true',
    LATTICE_JEV_ENABLED: '1', LATTICE_TASK019_PASSWORD: 'old', PGHOST: 'remote' }),
  { PATH: '/bin', CODEX_HOME: '/workspace/auth' });
  const environment = runtimeEnvironment({ root: '/state', repository: '/repo', codexHome: '/auth', codex: '/bin/codex',
    codexVersion: 'codex 1', files: { '/bin/codex': 'hash' }, git: '/bin/git', pgPort: 55432, controlPort: 4317, runId: 'run', password: 'secret' });
  assert.equal(environment.LATTICE_TASK019_HOST, '127.0.0.1');
  assert.equal(environment.LATTICE_RUNTIME_INTEGRATION, 'GRAPHIFY');
  assert.equal(Object.keys(environment).some((key) => /JEV|WSL|HERMES/u.test(key)), false);
});

function readyEvidence() {
  return { postgres: { status: 'VERIFIED' }, control: { identityVerified: true, formal_work_enabled: true, life_harness: { enabled: true }, formal_startup: { mode: 'explicit-only' } },
    runtime: { postgresql: 'HEALTHY', integration: 'GRAPHIFY' }, codex: { ready: true }, graphify: { status: 'VERIFIED' } };
}

test('core readiness does not imply Graphify or task acceptance', () => {
  const evidence = readyEvidence();
  evidence.graphify = { status: 'DEGRADED' };
  assert.deepEqual(classifyReadiness(evidence), { coreReady: true, overallReady: false, status: 'DEGRADED',
    formalTaskAcceptance: 'NOT_RUN', lifeHarnessAcceptance: 'NOT_RUN', jev: 'EXCLUDED' });
  evidence.graphify = { status: 'PREPARED' };
  assert.equal(classifyReadiness(evidence).overallReady, false);
});

test('every actual prerequisite is required; HTTP health and login text are insufficient', () => {
  assert.equal(classifyReadiness(readyEvidence()).overallReady, true);
  for (const key of ['postgres', 'control', 'runtime', 'codex']) {
    const evidence = readyEvidence(); evidence[key] = {};
    assert.equal(classifyReadiness(evidence).coreReady, false, key);
  }
  const restored = readyEvidence(); restored.control.formal_startup.mode = 'restore-existing';
  assert.equal(classifyReadiness(restored).coreReady, false);
  const wrongMode = readyEvidence(); wrongMode.runtime.integration = 'CORE_ONLY';
  assert.equal(classifyReadiness(wrongMode).coreReady, false);
  const foreignControl = readyEvidence(); foreignControl.control.identityVerified = false;
  assert.equal(classifyReadiness(foreignControl).coreReady, false);
});

function fixtureInstallation() {
  return { root: path.resolve('test-state'), repository: path.resolve('test-repo'), codexHome: path.resolve('test-auth'), codex: process.execPath,
    codexVersion: 'test-only', files: { [process.execPath]: 'fixture' }, git: path.resolve('git'), pgPort: 55432,
    controlPort: 4317, runId: 'test-only', password: 'fixture-private-password' };
}

test('Control and its ordinary descendants receive no PostgreSQL password or runtime bindings', async () => {
  const data = fixtureInstallation();
  const contaminated = { ...process.env, LATTICE_TASK019_PASSWORD: 'inherited-password', PGPASSWORD: 'inherited-pg', LATTICE_GRAPHIFY_RUNTIME_ROOT: '/old' };
  const env = childEnvironment(data, contaminated);
  const childProgram = `console.log(JSON.stringify({password:process.env.LATTICE_TASK019_PASSWORD??null,pg:process.env.PGPASSWORD??null,runId:process.env.LATTICE_TASK019_RUN_ID??null,config:process.env.LATTICE_RUNTIME_CONFIG_PATH}))`;
  const program = `require('node:child_process').execFile(process.execPath,['-e',${JSON.stringify(childProgram)}],(error,stdout)=>{if(error)process.exit(1);process.stdout.write(stdout);})`;
  const { stdout } = await promisify(execFile)(process.execPath, ['-e', program], { env });
  assert.deepEqual(JSON.parse(stdout), { password: null, pg: null, runId: null, config: path.join(data.root, 'runtime.toml') });
  assert.equal(Object.values(env).includes(data.password), false);
  assert.equal(runtimeChildEnvironment(data, contaminated).LATTICE_TASK019_PASSWORD, data.password);
});

test('readiness accepts only Control whose real HTTP data scope matches this installation', async (context) => {
  const data = fixtureInstallation();
  let observedRoot = data.root;
  let changeAfterState = false;
  const requests = [];
  const server = createServer((request, response) => {
    requests.push(request.url);
    response.setHeader('Content-Type', 'application/json');
    if (request.url === '/api/runtime') response.end(JSON.stringify({ schema_version: 'lattice.control.runtime-surface.v2',
      data_scope: controlDataScopeDescriptor(path.join(observedRoot, 'control.sqlite')) }));
    else {
      response.end(JSON.stringify(readyEvidence().control));
      if (changeAfterState) observedRoot = path.join(data.root, 'different');
    }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  context.after(async () => { server.closeAllConnections(); await new Promise((resolve) => server.close(resolve)); });
  data.controlPort = server.address().port;
  const result = await controlReadback(data);
  assert.equal(result.identityVerified, true);
  assert.equal(result.origin, `http://127.0.0.1:${data.controlPort}`);
  assert.equal(result.data_scope.digest, controlDataScopeDescriptor(path.join(data.root, 'control.sqlite')).digest);
  assert.deepEqual(requests, ['/api/runtime', '/api/state', '/api/runtime']);
  observedRoot = path.join(data.root, 'foreign');
  await assert.rejects(() => controlReadback(data), { code: 'CLOUD_CONTROL_SCOPE_REJECTED' });
  observedRoot = data.root; changeAfterState = true;
  await assert.rejects(() => controlReadback(data), { code: 'CLOUD_CONTROL_SCOPE_REJECTED' });
});

test('native Codex lookup rejects a missing or different executable at the first PATH entry', async (context) => {
  const directory = await mkdtemp(path.join(tmpdir(), 'lattice-cloud-codex-path-'));
  const codex = path.join(directory, 'codex');
  const selected = path.join(directory, 'selected');
  context.after(async () => {
    for (const file of [codex, selected]) await unlink(file).catch((error) => { if (error.code !== 'ENOENT') throw error; });
    await rmdir(directory);
  });
  await writeFile(codex, '', { mode: 0o700, flag: 'wx' });
  await writeFile(selected, '', { mode: 0o700, flag: 'wx' });
  await validateCodexPath(await realpath(codex));
  await assert.rejects(async () => validateCodexPath(await realpath(selected)), { code: 'CLOUD_CODEX_PATH_BINDING_REJECTED' });
  await unlink(codex);
  await assert.rejects(() => validateCodexPath(codex), { code: 'CLOUD_CODEX_PATH_BINDING_REJECTED' });
});

test('managed paths reject symlinks, public permissions, wrong owners and canonical escapes before startup', async () => {
  const data = fixtureInstallation();
  const directories = new Set(['delivery', 'graph', 'dependencies', 'postgres']);
  const visited = [];
  const info = (target) => ({ uid: 42, mode: directories.has(path.relative(data.root, target)) ? 0o700 : 0o600,
    isSymbolicLink: () => false, isDirectory: () => directories.has(path.relative(data.root, target)),
    isFile: () => !directories.has(path.relative(data.root, target)) });
  await validateManagedPaths(data, { ownerId: 42, inspect: async (target) => { visited.push(path.relative(data.root, target)); return info(target); }, canonicalize: async (target) => target });
  for (const required of ['control.sqlite', 'control.sqlite-wal', 'control.sqlite-shm', 'postgres.log', 'delivery', 'graph', 'dependencies']) assert.ok(visited.includes(required));
  for (const leaf of ['control.sqlite', 'control.sqlite-wal', 'control.sqlite-shm', 'postgres.log', 'delivery', 'graph', 'dependencies']) {
    await assert.rejects(() => validateManagedPaths(data, { ownerId: 42,
      inspect: async (target) => ({ ...info(target), isSymbolicLink: () => path.relative(data.root, target) === leaf }),
      canonicalize: async (target) => target }), { code: 'CLOUD_MANAGED_PATH_REJECTED' }, leaf);
  }
  for (const change of [{ uid: 7 }, { mode: 0o755 }]) {
    await assert.rejects(() => validateManagedPaths(data, { ownerId: 42,
      inspect: async (target) => ({ ...info(target), ...change }), canonicalize: async (target) => target }), { code: 'CLOUD_MANAGED_PATH_REJECTED' });
  }
  await assert.rejects(() => validateManagedPaths(data, { ownerId: 42, inspect: async (target) => info(target),
    canonicalize: async () => path.resolve(data.root, '..', 'escaped') }), { code: 'CLOUD_MANAGED_PATH_REJECTED' });
});

test('fresh Graphify analysis requires native persisted receipt for the current commit', () => {
  const receipt = { component: 'graphify', status: 'PERSISTED', commit: 'abc', usage_outcome: 'ANALYZED', analysis_calls: 1, receipt_digest: 'sha256:abc' };
  assert.equal(validGraphifyReceipt(receipt, 'abc'), true);
  for (const patch of [{ usage_outcome: 'REUSED' }, { analysis_calls: 0 }, { status: 'READY' }, { commit: 'old' }, { receipt_digest: '' }]) {
    assert.equal(validGraphifyReceipt({ ...receipt, ...patch }, 'abc'), false);
  }
});
