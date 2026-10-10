import { createHash } from 'node:crypto';
import { mkdtemp, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { normalizeControlOrigin } from './control-client.mjs';
import { canonicalizeProjectPath } from './project-inspector.mjs';
import { prepareRelativeModule } from './relative-module-diagnostic.mjs';
import { failureEventDigest } from './relative-module-receipt.mjs';

const cli = fileURLToPath(import.meta.url);
const hash = value => createHash('sha256').update(value).digest('hex');
const id = value => typeof value === 'string' && /^[A-Za-z0-9._:-]{1,256}$/u.test(value);
const fail = code => { throw Object.assign(new Error(code), { code }); };
const inside = (root, filename) => {
  const relative = path.relative(root, filename);
  return relative !== '' && !path.isAbsolute(relative) && relative !== '..' && !relative.startsWith(`..${path.sep}`);
};
const literal = value => {
  if (typeof value !== 'string' || /['"\u0000-\u001f\u007f\u2018-\u201f$`!^]/u.test(value)) fail('LIFE_HARNESS_UNSAFE_ARGUMENT');
  return `'${value}'`;
};

export function parseLifeHarnessArguments(argv) {
  const [action, ...rest] = argv;
  if (!['prepare', 'readback'].includes(action) || rest.length % 2) fail('LIFE_HARNESS_ARGUMENTS');
  const input = { action };
  for (let i = 0; i < rest.length; i += 2) {
    const name = rest[i].slice(2);
    if (!rest[i].startsWith('--') || !['origin', 'project', 'task', 'claim', 'thread', 'turn', 'failure'].includes(name)
      || Object.hasOwn(input, name)) fail('LIFE_HARNESS_ARGUMENTS');
    input[name] = rest[i + 1];
  }
  if (!['project', 'claim', 'thread', 'turn', 'failure'].every(key => id(input[key]))
    || !/^[a-f0-9]{64}$/u.test(input.task ?? '')) fail('LIFE_HARNESS_ARGUMENTS');
  input.origin = normalizeControlOrigin(input.origin);
  return input;
}

export function lifeHarnessCommand(input, action = 'prepare') {
  const validated = parseLifeHarnessArguments([action, ...['origin', 'project', 'task', 'claim', 'thread', 'turn', 'failure']
    .flatMap(key => [`--${key}`, input[key]])]);
  return `node --experimental-vm-modules --disable-warning=ExperimentalWarning ${literal(cli)} ${action} `
    + ['origin', 'project', 'task', 'claim', 'thread', 'turn', 'failure']
      .map(key => `--${key} ${literal(validated[key])}`).join(' ');
}

async function boundedJson(response) {
  const reader = response.body?.getReader();
  if (!reader) fail('LIFE_HARNESS_RESPONSE');
  const parts = []; let bytes = 0;
  try {
    for (;;) {
      const part = await reader.read();
      if (part.done) break;
      bytes += part.value.byteLength;
      if (bytes > 2 * 1024 * 1024) fail('LIFE_HARNESS_RESPONSE_TOO_LARGE');
      parts.push(Buffer.from(part.value));
    }
  } finally { await reader.cancel().catch(() => {}); }
  return JSON.parse(new TextDecoder('utf8', { fatal: true }).decode(Buffer.concat(parts)));
}

function bindingMatches(binding, input) {
  return binding && binding.projectId === input.project && binding.taskRef === input.task
    && binding.claimId === input.claim && binding.threadId === input.thread && binding.turnId === input.turn
    && binding.failureItemId === input.failure && id(binding.inputId);
}

// A Codex-invoked client, not a Control-side executor. The receipt command must
// be executed by Codex in a separate native tool call; it is never run here.
export async function runLifeHarnessClient(argv, { fetchImpl = fetch } = {}) {
  const input = parseLifeHarnessArguments(argv);
  const selectors = { projectId: input.project, claimId: input.claim, threadId: input.thread,
    turnId: input.turn, failureItemId: input.failure };
  const route = input.action === 'prepare' ? 'diagnosticsource' : 'diagnostic';
  const response = await fetchImpl(`${input.origin}/api/formal-work/${input.task}/${route}?${new URLSearchParams(selectors)}`,
    { redirect: 'error', signal: AbortSignal.timeout(8000) });
  const value = await boundedJson(response);
  if (!response.ok) fail(typeof value.code === 'string' && /^[A-Z][A-Z0-9_]{0,127}$/u.test(value.code)
    ? value.code : 'LIFE_HARNESS_HTTP_REJECTED');
  if (!bindingMatches(value.binding, input)) fail('LIFE_HARNESS_BINDING_REJECTED');
  if (input.action === 'readback') {
    const expiry = Date.parse(value.receipt?.validUntil);
    const trust = value.receipt?.result?.trust;
    if (value.schema !== 'lattice.control.relative-module-diagnostic.v1' || value.nativeClaimBindingVerified !== true
      || !bindingMatches(value.receipt?.binding, input) || value.receipt?.result?.advisoryOnly !== true
      || value.receipt.binding.inputId !== value.binding.inputId
      || typeof value.receipt.validUntil !== 'string' || !Number.isFinite(expiry) || expiry <= Date.now() || expiry > Date.now() + 300000
      || trust?.nativeProvenanceVerified !== false || trust.authorizationVerified !== false
      || value.receipt.result.adopted !== false || value.producerVerified !== false
      || value.inputFileBytesVerified !== false || value.diagnosticSemanticsVerified !== false) fail('LIFE_HARNESS_READBACK_REJECTED');
    return value;
  }
  const context = value.context;
  if (value.schema !== 'lattice.control.diagnostic-source.v1' || value.nativeSourceVerified !== true
    || value.contextBasis !== 'existing_owned_formal_execution' || value.advisoryOnly !== true || value.adopted !== false
    || value.grantsNewAuthority !== false || value.repairAuthorized !== false
    || context?.projectId !== input.project || context.taskId !== input.task || context.authority !== 'authorized'
    || context.projectScope !== 'matched' || context.lifecycle !== 'active' || context.freshness !== 'current'
    || context.circuitOpen !== false || !Number.isFinite(Date.parse(context.validUntil))
    || Date.parse(context.validUntil) <= Date.now() || Date.parse(context.validUntil) > Date.now() + 300000
    || typeof value.failureJson !== 'string' || hash(value.failureJson) !== value.failureSha256) fail('LIFE_HARNESS_SOURCE_REJECTED');
  const event = JSON.parse(value.failureJson);
  failureEventDigest(event);
  if (event.id !== input.failure) fail('LIFE_HARNESS_BINDING_REJECTED');
  const matches = [...event.aggregatedOutput.matchAll(/^Error \[ERR_MODULE_NOT_FOUND\]: Cannot find module '([^'\r\n]+)' imported from ([^\r\n]+)$/gmu)];
  if (matches.length !== 1) fail('LIFE_HARNESS_UNSUPPORTED_FAILURE');
  const importer = matches[0][2];
  if (typeof value.projectRoot !== 'string' || !path.isAbsolute(value.projectRoot)
    || !path.isAbsolute(importer) || !inside(value.projectRoot, importer)) fail('LIFE_HARNESS_PATH_REJECTED');
  const root = await canonicalizeProjectPath(value.projectRoot);
  if (path.relative(root, value.projectRoot) !== '') fail('LIFE_HARNESS_PATH_REJECTED');
  // Reject unrepresentable shell literals before writing any artifacts.
  const readbackCommand = lifeHarnessCommand(input, 'readback');
  literal(root);
  // Native output belongs in local retained evidence, outside repository trees.
  // Otherwise an ordinary git add could accidentally commit command/log data.
  const archiveRoot = await canonicalizeProjectPath(tmpdir());
  literal(archiveRoot);
  const directory = await mkdtemp(path.join(archiveRoot, 'lattice-diagnostic-'));
  const failureFile = path.join(directory, 'failure.json');
  await writeFile(failureFile, value.failureJson, { flag: 'wx', mode: 0o600 });
  await writeFile(path.join(directory, 'source.json'), JSON.stringify(value), { flag: 'wx', mode: 0o600 });
  const result = await prepareRelativeModule({ mode: 'receipt', projectRoot: root, importer, failureFile,
    failureSha256: value.failureSha256, outputDirectory: path.join(directory, 'receipt'),
    context, receiptBinding: value.binding });
  return { ...result, sourceFile: path.join(directory, 'source.json'), binding: value.binding,
    readbackCommand: result.decision === 'prepared' ? readbackCommand : null };
}

if (import.meta.url === pathToFileURL(process.argv[1] || '').href) {
  try {
    const result = await runLifeHarnessClient(process.argv.slice(2));
    console.log(JSON.stringify(result, null, 2));
    if (result.decision && result.decision !== 'prepared') process.exitCode = 1;
  } catch (error) {
    console.error(JSON.stringify({ error: /^[A-Z][A-Z0-9_]{0,127}$/u.test(error.code ?? '') ? error.code : 'LIFE_HARNESS_CLIENT_FAILED' }));
    process.exitCode = 1;
  }
}
