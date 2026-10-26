import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

const cli = fileURLToPath(new URL('./cli.mjs', import.meta.url));

function run(args) {
  const result = spawnSync(process.execPath, [cli, ...args], {
    cwd: tmpdir(),
    encoding: 'utf8',
    timeout: 10_000,
    windowsHide: true,
  });
  assert.equal(result.error, undefined);
  assert.equal(result.signal, null);
  return result;
}

const validCases = [
  ['mixed', [{ completed: true }, { completed: false }, { completed: true }],
    { total: 3, completed: 2, pending: 1 }],
  ['empty', [], { total: 0, completed: 0, pending: 0 }],
  ['all completed', [{ completed: true }, { completed: true }],
    { total: 2, completed: 2, pending: 0 }],
  ['all pending', [{ completed: false }, { completed: false }],
    { total: 2, completed: 0, pending: 2 }],
  ['extra fields', [{ id: 1, title: '任務', completed: false }],
    { total: 1, completed: 0, pending: 1 }],
];

for (const [name, tasks, expected] of validCases) {
  test(`valid input: ${name}`, () => {
    const result = run([JSON.stringify(tasks)]);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stderr, '');
    const actual = JSON.parse(result.stdout);
    assert.deepEqual(actual, expected);
    assert.deepEqual(Object.keys(actual).sort(), ['completed', 'pending', 'total']);
    assert.ok(Object.values(actual).every(Number.isInteger));
    assert.equal(result.stdout.trim().split('\n').length, 1);
  });
}

const invalidCases = [
  ['missing argument', []],
  ['empty argument', ['']],
  ['malformed JSON', ['[']],
  ['trailing comma', ['[{"completed":true},]']],
  ['trailing garbage', ['[]garbage']],
  ['object root', ['{}']],
  ['null root', ['null']],
  ['string root', ['"tasks"']],
  ['number root', ['42']],
  ['boolean root', ['true']],
  ['missing completed', ['[{}]']],
  ['null element', ['[null]']],
  ['array element', ['[[]]']],
  ['string element', ['["task"]']],
  ['number element', ['[1]']],
  ['boolean element', ['[false]']],
  ['string true', ['[{"completed":"true"}]']],
  ['string false', ['[{"completed":"false"}]']],
  ['numeric zero', ['[{"completed":0}]']],
  ['numeric one', ['[{"completed":1}]']],
  ['null completed', ['[{"completed":null}]']],
  ['array completed', ['[{"completed":[]}]']],
  ['object completed', ['[{"completed":{}}]']],
  ['completed only in data named __proto__', ['[{"__proto__":{"completed":true}}]']],
  ['invalid after valid prefix', ['[{"completed":true},{"completed":false},{}]']],
];

for (const [name, args] of invalidCases) {
  test(`reject input without success output: ${name}`, () => {
    const result = run(args);
    assert.equal(result.status, 1);
    assert.equal(result.stdout, '');
    assert.match(result.stderr, /^Invalid task input: /);
  });
}
