import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { startDefaultServer, parseServerOptions } from '../src/server.mjs';
import { LatticeStore } from '../src/store.mjs';
import { FormalTaskService } from '../src/formal-task-service.mjs';
import { LatticeRuntimeClient } from '../src/lattice-runtime-client.mjs';
import { CodexAppServer } from '../src/codex-app-server.mjs';

test('server options preserve default and accept only the explicit opt-out flag', async () => {
  assert.deepEqual(parseServerOptions([]), { autoRestore: true });
  assert.deepEqual(parseServerOptions(['--no-auto-restore']), { autoRestore: false });
  for (const args of [['--no-auto-restore=false'], ['--unknown'], ['--no-auto-restore', '--no-auto-restore']]) assert.throws(() => parseServerOptions(args));
  await assert.rejects(startDefaultServer({ autoRestore: 'false' }), /boolean/u);
});
for (const autoRestore of [true, false]) test(`startup restore policy ${autoRestore} is observable without native or Runtime activation`, async t => {
  const directory = await mkdtemp(path.join(tmpdir(), 'lattice-startup-')), database = path.join(directory, 'control.db');
  const env = { LATTICE_CONTROL_DATABASE_PATH: database, LATTICE_CONTROL_PORT: '0', LATTICE_CONTROL_DESKTOP_OWNED: '0' };
  const prior = Object.fromEntries(Object.keys(env).map(key => [key, process.env[key]]));
  const seed = new LatticeStore(database);
  const ids = [seed.createProject({ name: 'one', rootPath: path.join(directory, 'one') }).id,
    seed.createProject({ name: 'two', rootPath: path.join(directory, 'two') }).id]; seed.close();
  const restores = [], effects = [];
  t.mock.method(FormalTaskService.prototype, 'restore', async projects => { restores.push(projects); });
  t.mock.method(LatticeRuntimeClient.prototype, 'call', async () => { effects.push('runtime'); throw new Error('Unexpected Runtime'); });
  t.mock.method(CodexAppServer.prototype, 'connect', async () => { effects.push('native'); throw new Error('Unexpected native'); });
  let app;
  try {
    Object.assign(process.env, env);
    app = autoRestore ? await startDefaultServer() : await startDefaultServer(parseServerOptions(['--no-auto-restore']));
    await app.formalRestore;
    assert.equal(restores.length, autoRestore ? 1 : 0);
    if (autoRestore) assert.deepEqual([...restores[0]].sort(), [...ids].sort());
    const state = await (await fetch(`http://127.0.0.1:${app.server.address().port}/api/state`)).json();
    assert.equal(state.formal_work_enabled, true);
    assert.deepEqual(state.life_harness, { enabled: true, scope: 'owned-execution', mode: 'failure-candidates' });
    assert.deepEqual(state.formal_startup, { mode: autoRestore ? 'restore-existing' : 'explicit-only', restoreRequested: autoRestore });
    assert.deepEqual(effects, []);
  } finally {
    if (app) await app.shutdownOwned({ timeoutMs: 5000 });
    for (const [key, value] of Object.entries(prior)) { if (value === undefined) delete process.env[key]; else process.env[key] = value; }
    assert.deepEqual(effects, []); await rm(directory, { recursive: true, force: true });
  }
});
