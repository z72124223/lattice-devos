import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { runProjectCli } from '../src/project-client.mjs';

test('delete and purge aliases delegate exact arguments to the offline purge CLI', async () => {
  for (const alias of ['delete', 'purge']) {
    const args = ['apply', '--plan', 'approved.json', '--confirm', 'digest', '--maintenance-offline'];
    const expected = { output: 'PURGE_EXACT_CONFIRMATION_REQUIRED', exitCode: 1 };
    const result = await runProjectCli([alias, ...args], {
      purgeCli: async actual => { assert.deepEqual(actual, args); return expected; },
      projectCommand: () => assert.fail('a purge must never use the HTTP project command'),
    });
    assert.equal(result, expected);
  }
});

test('purge aliases do not invent missing confirmations or discard unknown arguments', async () => {
  for (const args of [[], ['apply'], ['apply', '--unknown'], ['--origin', 'http://127.0.0.1:4317']]) {
    const result = await runProjectCli(['delete', ...args], {
      purgeCli: async actual => {
        assert.deepEqual(actual, args);
        return { output: 'PURGE_ARGUMENTS_REJECTED', exitCode: 1 };
      },
      projectCommand: () => assert.fail('HTTP must not run'),
    });
    assert.equal(result.exitCode, 1);
  }
});

test('ordinary project help advertises the single purge entry without loading it', async () => {
  const result = await runProjectCli(['--help'], {
    purgeCli: () => assert.fail('no purge invocation'),
    projectCommand: () => assert.fail('no HTTP invocation'),
  });
  assert.equal(result.exitCode, 0);
  assert.match(result.output, /project:purge/);
});

test('real alias CLI rejects missing confirmation and unknown flags before accessing a plan', () => {
  const client = fileURLToPath(new URL('../src/project-client.mjs', import.meta.url));
  for (const alias of ['delete', 'purge']) {
    for (const args of [['apply', '--plan', 'does-not-exist.json'], ['status', '--plan', 'does-not-exist.json', '--unknown']]) {
      const result = spawnSync(process.execPath, [client, alias, ...args], { encoding: 'utf8', windowsHide: true });
      assert.equal(result.status, 1, result.stderr);
      assert.doesNotMatch(result.stderr, /ENOENT|ENOTFOUND|ECONNREFUSED|not a function/);
      assert.match(result.stderr, /PURGE_/);
    }
  }
});
