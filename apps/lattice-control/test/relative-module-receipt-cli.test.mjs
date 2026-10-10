import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { createHash } from 'node:crypto';
import { mkdtemp, realpath, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { diagnosticReceiptCommand } from '../src/relative-module-receipt.mjs';
const execute = promisify(execFile);
const cli = fileURLToPath(new URL('../src/relative-module-diagnostic-cli.mjs', import.meta.url));
const sha = bytes => createHash('sha256').update(bytes).digest('hex');

test('CLI receipt binds actual input bytes and keeps diagnosis advisory; bad hashes/binding fail', async t => {
  // Isolated CLI contract exercise, not a native formal-task/Runtime acceptance.
  const root = await realpath(await mkdtemp(path.join(tmpdir(), 'lattice-receipt-')));
  t.after(async () => { assert.equal(await realpath(root), root); await rm(root, { recursive: true }); });
  const importer = path.join(root, 'entry.mjs'), evidenceFile = path.join(root, 'evidence.json');
  const requestPath = path.join(root, 'request.json'), taskRef = 'a'.repeat(64);
  const source = 'import "./missing.mjs";\nthrow new Error("must not evaluate");\n';
  await writeFile(importer, source);
  const evidence = { schema: 'lattice.relative-module-evidence.v1',
    subject: { projectId: 'project', taskId: taskRef, projectRoot: root, importer, importerSha256: sha(source) },
    event: { id: 'failure', type: 'commandExecution', status: 'failed', command: 'node ./entry.mjs',
      aggregatedOutput: `Error [ERR_MODULE_NOT_FOUND]: Cannot find module '${path.join(root, 'missing.mjs')}' imported from ${importer}`, exitCode: 1 } };
  const bytes = JSON.stringify(evidence) + '\n'; await writeFile(evidenceFile, bytes);
  const request = { projectRoot: root, importer, evidenceFile, evidenceSha256: sha(bytes),
    context: { projectId: 'project', taskId: taskRef, authority: 'authorized', projectScope: 'matched',
      lifecycle: 'active', freshness: 'current', circuitOpen: false, validUntil: new Date(Date.now() + 120000).toISOString() },
    receiptBinding: { projectId: 'project', taskRef, claimId: 'claim', threadId: 'thread', turnId: 'turn', inputId: 'input', failureItemId: 'failure' } };
  const requestBytes = JSON.stringify(request) + '\n'; await writeFile(requestPath, requestBytes);
  const run = digest => execute(process.execPath,
    ['--experimental-vm-modules', '--disable-warning=ExperimentalWarning', cli, requestPath, '--receipt', digest],
    { windowsHide: true, timeout: 10000, encoding: 'utf8' });
  const output = JSON.parse((await run(sha(requestBytes))).stdout);
  assert.equal(output.schema, 'lattice.relative-module-receipt.v1');
  assert.equal(output.requestSha256, sha(requestBytes)); assert.equal(output.evidenceSha256, sha(bytes));
  assert.equal(output.failureEventSha256, sha(JSON.stringify(evidence.event)));
  assert.deepEqual(output.binding, request.receiptBinding); assert.equal(output.result.decision, 'selected');
  assert.equal(output.result.advisoryOnly, true); assert.equal(output.result.adopted, false);
  assert.equal(output.result.trust.authorizationVerified, false); assert.equal(output.result.trust.nativeProvenanceVerified, false);
  const command = diagnosticReceiptCommand(requestPath, sha(requestBytes));
  const shellOutput = await execute(process.platform === 'win32' ? 'powershell.exe' : '/bin/sh',
    process.platform === 'win32' ? ['-NoProfile', '-NonInteractive', '-Command', command] : ['-c', command],
    { windowsHide: true, timeout: 10000, encoding: 'utf8' });
  assert.deepEqual(JSON.parse(shellOutput.stdout), output, 'documented exact command must run through the native shell');
  await assert.rejects(run('0'.repeat(64)), error => error.code === 1 && /Request digest mismatch/u.test(error.stderr));
  request.receiptBinding.failureItemId = 'different-item';
  const mismatched = JSON.stringify(request); await writeFile(requestPath, mismatched);
  await assert.rejects(run(sha(mismatched)), error => error.code === 1 && /BINDING_REJECTED/u.test(error.stderr));
  request.receiptBinding.failureItemId = 'failure'; request.evidenceSha256 = '0'.repeat(64);
  const drift = JSON.stringify(request); await writeFile(requestPath, drift);
  await assert.rejects(run(sha(drift)), error => error.code === 1 && /BINDING_REJECTED/u.test(error.stderr));
});
