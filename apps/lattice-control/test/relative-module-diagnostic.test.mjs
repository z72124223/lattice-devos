import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { lstat, mkdir, mkdtemp, readFile, realpath, rename, rm, symlink, unlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import test from 'node:test';

import { diagnoseRelativeModule } from '../src/relative-module-diagnostic.mjs';

const execute = promisify(execFile);
const cli = fileURLToPath(new URL('../src/relative-module-diagnostic-cli.mjs', import.meta.url));
const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');
const normalized = (value) => process.platform === 'win32' ? path.resolve(value).toLowerCase() : path.resolve(value);

async function fixture(context, { filename = 'entry.mjs', specifier = './missing.mjs', source } = {}) {
  const directory = await realpath(await mkdtemp(path.join(tmpdir(), 'lattice-relative-module-')));
  const createdRoot = normalized(directory);
  const links = [];
  context.after(async () => {
    // Never recursively delete a replaced root or follow a test-created junction.
    assert.equal(normalized(await realpath(directory)), createdRoot);
    for (const link of links.reverse()) {
      assert.equal(path.relative(directory, link).startsWith('..'), false);
      assert.equal((await lstat(link)).isSymbolicLink(), true);
      await unlink(link);
    }
    assert.equal(normalized(await realpath(directory)), createdRoot);
    await rm(directory, { recursive: true, force: false });
  });
  const projectRoot = path.join(directory, 'project');
  const importer = path.join(projectRoot, 'src', filename);
  const target = path.resolve(path.dirname(importer), specifier);
  const evidenceFile = path.join(projectRoot, 'failure.json');
  const text = source ?? `import ${JSON.stringify(specifier)};\n`;
  await mkdir(path.dirname(importer), { recursive: true });
  await writeFile(importer, text);
  const now = Date.now();
  const request = {
    projectRoot, importer, evidenceFile, evidenceSha256: '',
    context: {
      projectId: 'project-fixture', taskId: 'task-fixture', projectScope: 'matched',
      authority: 'authorized', lifecycle: 'active', freshness: 'current', circuitOpen: false,
      validUntil: new Date(now + 60 * 60 * 1000).toISOString(),
    },
  };
  const evidence = {
    schema: 'lattice.relative-module-evidence.v1',
    subject: {
      projectId: request.context.projectId, taskId: request.context.taskId,
      projectRoot, importer, importerSha256: digest(Buffer.from(text)),
    },
    event: {
      id: 'native-fixture-event', type: 'commandExecution', status: 'failed',
      command: `node src/${filename}`,
      aggregatedOutput: `Error [ERR_MODULE_NOT_FOUND]: Cannot find module '${target}' imported from ${importer}`,
      exitCode: 1,
    },
  };
  const saveEvidence = async () => {
    const bytes = Buffer.from(`${JSON.stringify(evidence)}\n`);
    await writeFile(request.evidenceFile, bytes);
    request.evidenceSha256 = digest(bytes);
  };
  await saveEvidence();
  return { directory, projectRoot, importer, target, specifier, request, evidence, now, links, saveEvidence };
}

function advisory(result) {
  assert.equal(result.advisoryOnly, true);
  assert.equal(result.adopted, false);
  assert.equal(result.trust.nativeProvenanceVerified, false);
  assert.equal(result.trust.authorizationVerified, false);
  assert.ok(Array.isArray(result.hints));
}

function notSelected(result) {
  advisory(result);
  assert.ok(['abstain', 'excluded'].includes(result.decision));
  assert.equal(result.location, null);
}

async function makeLink(context, sample, target, link, type) {
  try {
    await symlink(target, link, type);
  } catch (error) {
    if (['EPERM', 'EACCES', 'ENOTSUP'].includes(error?.code)) {
      context.skip(`${type} creation unavailable on ${process.platform}: ${error.code}`);
      return false;
    }
    throw error;
  }
  sample.links.push(link);
  assert.equal((await lstat(link)).isSymbolicLink(), true);
  return true;
}

test('selects distinct explicit importer/target names and keeps evidence claims unverified', async (context) => {
  for (const options of [
    { filename: 'alpha-loader.mjs', specifier: './alpha-helper.mjs' },
    { filename: 'other entry.mjs', specifier: './different utility.mjs' },
  ]) await context.test(options.filename, async (subtest) => {
    const sample = await fixture(subtest, options);
    const result = await diagnoseRelativeModule(sample.request, { now: sample.now });
    advisory(result);
    assert.equal(result.decision, 'selected');
    assert.equal(normalized(result.location.projectRoot), normalized(sample.projectRoot));
    assert.equal(normalized(result.location.importer), normalized(sample.importer));
    assert.equal(result.location.specifier, sample.specifier);
    assert.equal(normalized(result.location.target), normalized(sample.target));
    assert.equal(normalized(result.location.nearestExistingParent), normalized(path.dirname(sample.importer)));
    assert.equal(result.location.targetExistsNow, false);
    assert.equal(result.trust.sourceBytesVerified, true);
    assert.equal(result.trust.importerBytesVerified, true);
  });
});

test('parses importer syntax without evaluating its top-level statements', async (context) => {
  const sample = await fixture(context, {
    source: 'throw new Error("this source must never execute");\nimport "./missing.mjs";\n',
  });
  const result = await diagnoseRelativeModule(sample.request, { now: sample.now });
  assert.equal(result.decision, 'selected');
  advisory(result);
});

test('rejects raw evidence drift, importer drift, and mismatched subject identity', async (context) => {
  for (const kind of ['evidence-bytes', 'importer-bytes', 'subject-task', 'subject-importer-hash']) {
    await context.test(kind, async (subtest) => {
      const sample = await fixture(subtest);
      if (kind === 'evidence-bytes') {
        await writeFile(sample.request.evidenceFile, `${await readFile(sample.request.evidenceFile, 'utf8')} `);
      } else if (kind === 'importer-bytes') {
        await writeFile(sample.importer, 'import "./different.mjs";\n');
      } else {
        if (kind === 'subject-task') sample.evidence.subject.taskId = 'another-task';
        else sample.evidence.subject.importerSha256 = '0'.repeat(64);
        await sample.saveEvidence();
      }
      notSelected(await diagnoseRelativeModule(sample.request, { now: sample.now }));
    });
  }
});

test('comments, strings, templates, dynamic imports, and changed static targets are not static evidence', async (context) => {
  for (const [name, source] of [
    ['comment', '// import "./missing.mjs";\n'],
    ['string', 'const example = \'import "./missing.mjs";\';\n'],
    ['template', 'const example = `import "./missing.mjs";`;\n'],
    ['dynamic', 'await import("./missing.mjs");\n'],
    ['different-static-target', 'import "./different.mjs";\n'],
  ]) await context.test(name, async (subtest) => {
    const sample = await fixture(subtest, { source });
    const result = await diagnoseRelativeModule(sample.request, { now: sample.now });
    notSelected(result);
    assert.equal(result.decision, 'abstain');
  });
});

test('non-applicable event and context states are excluded', async (context) => {
  for (const kind of ['completed', 'declined', 'completed-null-output', 'declined-null-output', 'agent-message', 'cancelled', 'expired', 'circuit-open', 'unknown-authority', 'wrong-project']) {
    await context.test(kind, async (subtest) => {
      const sample = await fixture(subtest);
      if (kind === 'completed') Object.assign(sample.evidence.event, { status: 'completed', exitCode: 0 });
      if (kind === 'declined') Object.assign(sample.evidence.event, { status: 'declined', exitCode: null });
      if (kind === 'completed-null-output') Object.assign(sample.evidence.event, {
        status: 'completed', exitCode: 0, command: null, aggregatedOutput: null,
      });
      if (kind === 'declined-null-output') Object.assign(sample.evidence.event, {
        status: 'declined', exitCode: null, command: null, aggregatedOutput: null,
      });
      if (kind === 'agent-message') sample.evidence.event.type = 'agentMessage';
      if (kind === 'cancelled') sample.request.context.lifecycle = 'cancelled';
      if (kind === 'expired') sample.request.context.validUntil = new Date(sample.now - 1).toISOString();
      if (kind === 'circuit-open') sample.request.context.circuitOpen = true;
      if (kind === 'unknown-authority') sample.request.context.authority = 'unknown';
      if (kind === 'wrong-project') sample.request.context.projectScope = 'mismatched';
      await sample.saveEvidence();
      const result = await diagnoseRelativeModule(sample.request, { now: sample.now });
      notSelected(result);
      assert.equal(result.decision, 'excluded');
    });
  }
});

test('third-party package and entry-point failures do not select a relative dependency', async (context) => {
  for (const kind of ['package', 'entry']) await context.test(kind, async (subtest) => {
    const sample = await fixture(subtest, { source: 'import "fixture-package";\n' });
    sample.evidence.event.aggregatedOutput = kind === 'package'
      ? `Error [ERR_MODULE_NOT_FOUND]: Cannot find package 'fixture-package' imported from ${sample.importer}`
      : `Error: Cannot find module '${sample.importer}'\ncode: 'MODULE_NOT_FOUND'`;
    await sample.saveEvidence();
    const result = await diagnoseRelativeModule(sample.request, { now: sample.now });
    notSelected(result);
    assert.equal(result.decision, 'abstain');
  });
});

test('target and importer cannot escape into outside or prefix-similar roots', async (context) => {
  for (const specifier of ['../../outside.mjs', '../../project-neighbor/missing.mjs']) {
    await context.test(specifier, async (subtest) => {
      const sample = await fixture(subtest, { specifier });
      await mkdir(path.join(sample.directory, 'project-neighbor'));
      notSelected(await diagnoseRelativeModule(sample.request, { now: sample.now }));
    });
  }
  await context.test('importer in prefix-similar sibling', async (subtest) => {
    const sample = await fixture(subtest);
    const outside = path.join(sample.directory, 'project-neighbor', 'entry.mjs');
    await mkdir(path.dirname(outside));
    await writeFile(outside, await readFile(sample.importer));
    sample.request.importer = outside;
    sample.evidence.subject.importer = outside;
    sample.evidence.event.aggregatedOutput = `Error [ERR_MODULE_NOT_FOUND]: Cannot find module '${path.join(path.dirname(outside), 'missing.mjs')}' imported from ${outside}`;
    await sample.saveEvidence();
    notSelected(await diagnoseRelativeModule(sample.request, { now: sample.now }));
  });
});

test('rejects a real project-root junction or directory symlink', async (context) => {
  const sample = await fixture(context);
  const alias = path.join(sample.directory, 'root-alias');
  if (!await makeLink(context, sample, sample.projectRoot, alias, process.platform === 'win32' ? 'junction' : 'dir')) return;
  sample.request.projectRoot = sample.evidence.subject.projectRoot = alias;
  sample.request.importer = sample.evidence.subject.importer = path.join(alias, 'src', 'entry.mjs');
  sample.evidence.event.aggregatedOutput = `Error [ERR_MODULE_NOT_FOUND]: Cannot find module '${path.join(alias, 'src', 'missing.mjs')}' imported from ${sample.request.importer}`;
  await sample.saveEvidence();
  notSelected(await diagnoseRelativeModule(sample.request, { now: sample.now }));
});

test('rejects a real junction or directory symlink in importer ancestry', async (context) => {
  const sample = await fixture(context);
  const original = path.join(sample.projectRoot, 'src');
  const outside = path.join(sample.directory, 'source-store');
  await rename(original, outside);
  if (!await makeLink(context, sample, outside, original, process.platform === 'win32' ? 'junction' : 'dir')) return;
  notSelected(await diagnoseRelativeModule(sample.request, { now: sample.now }));
});

test('rejects a real junction or directory symlink in the missing target ancestry', async (context) => {
  const sample = await fixture(context, { specifier: './redirected/missing.mjs' });
  const outside = path.join(sample.directory, 'target-store');
  await mkdir(outside);
  const link = path.join(path.dirname(sample.importer), 'redirected');
  if (!await makeLink(context, sample, outside, link, process.platform === 'win32' ? 'junction' : 'dir')) return;
  notSelected(await diagnoseRelativeModule(sample.request, { now: sample.now }));
});

test('rejects an actual importer file symlink; unavailable creation is explicitly skipped', async (context) => {
  const sample = await fixture(context);
  const stored = path.join(sample.directory, 'real-entry.mjs');
  await rename(sample.importer, stored);
  if (!await makeLink(context, sample, stored, sample.importer, 'file')) return;
  notSelected(await diagnoseRelativeModule(sample.request, { now: sample.now }));
});

test('thin CLI emits JSON with exit zero for selected, abstain, and excluded decisions', async (context) => {
  for (const decision of ['selected', 'abstain', 'excluded']) await context.test(decision, async (subtest) => {
    const sample = await fixture(subtest, decision === 'abstain' ? { source: 'const noImport = true;\n' } : {});
    if (decision === 'excluded') {
      sample.evidence.event.status = 'completed';
      sample.evidence.event.exitCode = 0;
      await sample.saveEvidence();
    }
    const input = path.join(sample.directory, 'request.json');
    await writeFile(input, JSON.stringify(sample.request));
    const result = await execute(process.execPath, ['--experimental-vm-modules', cli, input], {
      cwd: sample.directory, windowsHide: true, encoding: 'utf8', timeout: 10_000,
    });
    const output = JSON.parse(result.stdout);
    advisory(output);
    assert.equal(output.decision, decision);
  });
});

test('thin CLI rejects invalid arguments, malformed or non-object JSON, and oversized input', async (context) => {
  const sample = await fixture(context);
  const input = path.join(sample.directory, 'invalid-request.json');
  await writeFile(input, '{invalid JSON');
  for (const args of [[], [input, 'extra'], [input]]) {
    await assert.rejects(execute(process.execPath, ['--experimental-vm-modules', cli, ...args], {
      cwd: sample.directory, windowsHide: true, encoding: 'utf8', timeout: 10_000,
    }), (error) => error.code === 1);
  }
  for (const content of ['null', '[]', '42', '"not a request"',
    JSON.stringify({ ...sample.request, padding: 'x'.repeat(1024 * 1024) })]) {
    await writeFile(input, content);
    await assert.rejects(execute(process.execPath, ['--experimental-vm-modules', cli, input], {
      cwd: sample.directory, windowsHide: true, encoding: 'utf8', timeout: 10_000,
    }), (error) => error.code === 1);
  }
});
