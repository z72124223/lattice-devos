import { open } from 'node:fs/promises';
import process from 'node:process';
import { pathToFileURL } from 'node:url';
import { defaultControlOrigin, normalizeControlOrigin } from './control-client.mjs';

const maximumInputBytes = 65_536;
const maximumResponseBytes = 1_048_576;
const commands = new Set(['state', 'projects', 'list', 'status', 'create', 'start', 'continue', 'interrupt']);
const taskPattern = /^[a-f0-9]{64}$/u;
const identifierPattern = /^[A-Za-z0-9._:-]+$/u;
const controlCharacters = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/u;

function failure(code, message, status) {
  return Object.assign(new Error(message), { code, ...(status === undefined ? {} : { status }) });
}
function object(value, label) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError(`${label} must be an object`);
  return value;
}
function identifier(value, label, maximum = 64) {
  if (typeof value !== 'string' || !identifierPattern.test(value) || value.length > maximum) {
    throw new TypeError(`${label} must be a bounded ASCII identifier`);
  }
  return value;
}
function taskReference(value) {
  if (typeof value !== 'string' || !taskPattern.test(value)) throw new TypeError('task must be a lowercase SHA-256 identifier');
  return value;
}
function text(value, label, maximum) {
  if (typeof value !== 'string' || !value.trim() || controlCharacters.test(value) || Buffer.byteLength(value) > maximum) {
    throw new TypeError(`${label} must be bounded nonempty text without control characters`);
  }
  return value;
}
function exactFields(value, required, optional = []) {
  object(value, 'input');
  const allowed = new Set([...required, ...optional]);
  if (required.some(key => !Object.hasOwn(value, key)) || Object.keys(value).some(key => !allowed.has(key))) {
    throw new TypeError('input has missing or unsupported fields');
  }
}

// JSON.parse alone silently accepts duplicate request IDs. Inspect decoded keys
// before accepting the flat input object, including escaped spellings of keys.
function parseInput(source) {
  let value;
  try { value = JSON.parse(source); } catch { throw new TypeError('input must contain valid UTF-8 JSON'); }
  object(value, 'input');
  const keys = new Set();
  let depth = 0, wantsKey = false;
  for (const match of source.matchAll(/"(?:[^"\\]|\\.)*"|[{}\[\],:]/gu)) {
    const token = match[0];
    if (token === '{' || token === '[') { depth += 1; if (depth === 1) wantsKey = true; }
    else if (token === '}' || token === ']') depth -= 1;
    else if (token === ',' && depth === 1) wantsKey = true;
    else if (token.startsWith('"') && depth === 1 && wantsKey) {
      const key = JSON.parse(token);
      if (keys.has(key)) throw new TypeError('input contains a duplicate JSON field');
      keys.add(key); wantsKey = false;
    }
  }
  return value;
}

async function readInput(inputFile) {
  text(inputFile, 'input file', 32_768);
  const file = await open(inputFile, 'r');
  try {
    if (!(await file.stat()).isFile()) throw new TypeError('input must be a regular JSON file');
    const buffer = Buffer.alloc(maximumInputBytes + 1);
    let length = 0;
    while (length < buffer.length) {
      const { bytesRead } = await file.read(buffer, length, buffer.length - length, null);
      if (bytesRead === 0) break;
      length += bytesRead;
    }
    if (length > maximumInputBytes) throw new TypeError('input JSON exceeds 65536 bytes');
    let source;
    try { source = new TextDecoder('utf-8', { fatal: true }).decode(buffer.subarray(0, length)); }
    catch { throw new TypeError('input must contain valid UTF-8 JSON'); }
    return parseInput(source);
  } finally { await file.close(); }
}

function validateInput(command, input) {
  if (command === 'create') {
    exactFields(input, ['projectId', 'objective', 'clientRequestId'], ['parentTaskRef', 'title', 'successCriteria', 'priority']);
    identifier(input.projectId, 'project');
    identifier(input.clientRequestId, 'clientRequestId');
    text(input.objective, 'objective', 2048);
    if ([...input.objective].length > 512) throw new TypeError('objective exceeds 512 characters');
    if (input.parentTaskRef != null) taskReference(input.parentTaskRef);
    if (Object.hasOwn(input, 'title')) text(typeof input.title === 'string' ? input.title.trim() : input.title, 'title', 240);
    if (Object.hasOwn(input, 'successCriteria')) text(typeof input.successCriteria === 'string' ? input.successCriteria.trim() : input.successCriteria, 'successCriteria', 8192);
    if (Object.hasOwn(input, 'priority') && (!Number.isInteger(input.priority) || input.priority < 0 || input.priority > 3)) {
      throw new TypeError('priority must be an integer from 0 to 3');
    }
  } else {
    exactFields(input, ['projectId', 'taskRef', 'inputId', 'text'], ['phase']);
    identifier(input.projectId, 'project'); taskReference(input.taskRef);
    identifier(input.inputId, 'inputId', 128); text(input.text, 'text', 16_384);
    if (Object.hasOwn(input, 'phase') && !['EXECUTION', 'VERIFICATION'].includes(input.phase)) {
      throw new TypeError('phase must be EXECUTION or VERIFICATION');
    }
  }
  return input;
}

async function boundedJson(response, signal) {
  const declared = response.headers.get('content-length');
  if (declared !== null && (!/^\d+$/u.test(declared) || Number(declared) > maximumResponseBytes)) {
    await response.body?.cancel();
    throw failure('CONTROL_RESPONSE_LIMIT_EXCEEDED', 'Control response exceeds the byte limit');
  }
  if (!response.body) throw failure('CONTROL_RESPONSE_REJECTED', 'Control returned no JSON body');
  const reader = response.body.getReader(), chunks = [];
  let length = 0;
  try {
    while (true) {
      signal.throwIfAborted();
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > maximumResponseBytes) {
        await reader.cancel();
        throw failure('CONTROL_RESPONSE_LIMIT_EXCEEDED', 'Control response exceeds the byte limit');
      }
      chunks.push(value);
    }
  } finally { reader.releaseLock(); }
  try { return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks, length))); }
  catch { throw failure('CONTROL_RESPONSE_REJECTED', 'Control returned invalid JSON'); }
}

async function request(fetchImpl, origin, route, body, timeoutMs, expectedStatus = 200) {
  const signal = AbortSignal.timeout(timeoutMs);
  let response, result;
  try {
    response = await fetchImpl(`${origin}${route}`, {
      method: body === undefined ? 'GET' : 'POST', redirect: 'error', signal,
      ...(body === undefined ? {} : { headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) }),
    });
    if (response.redirected || response.url && new URL(response.url).origin !== origin) {
      throw failure('CONTROL_REDIRECT_REJECTED', 'Control redirects are not accepted');
    }
    result = await boundedJson(response, signal);
  } catch (error) {
    if (typeof error?.code === 'string' && error.code.startsWith('CONTROL_')) throw error;
    throw failure(signal.aborted ? 'CONTROL_REQUEST_TIMEOUT' : 'CONTROL_REQUEST_FAILED',
      'Control request did not complete; query its state before retrying the same request identity');
  }
  if (!response.ok) {
    const code = typeof result?.code === 'string' && /^[A-Z0-9_]{1,100}$/u.test(result.code)
      ? result.code : 'CONTROL_REQUEST_REJECTED';
    // Server error bodies can contain command output. Keep only the safe code.
    throw failure(code, `Control rejected the request (HTTP ${response.status})`, response.status);
  }
  if (response.status !== expectedStatus) throw failure('CONTROL_RESPONSE_REJECTED', 'Control returned an unexpected HTTP status');
  return object(result, 'Control response');
}

function small(value, maximum = 128) {
  return typeof value === 'string' ? value.replace(/[\u0000-\u001f\u007f-\u009f]/gu, ' ').slice(0, maximum) : null;
}
function sourceIdentity(value) {
  if (value?.kind !== 'POSTGRESQL_CONTROL_PRODUCT' || value.authority !== 'POSTGRESQL_TASK_LEDGER') {
    throw failure('CONTROL_WORK_AUTHORITY_REJECTED', 'Control did not return authoritative formal work');
  }
  return { kind: value.kind, authority: value.authority };
}
function candidateProgress(detail) {
  const prefix = 'LIFE_HARNESS_CANDIDATE_V1 ';
  const observations = detail.product?.observations ?? [];
  if (!Array.isArray(observations)) throw failure('CONTROL_RESPONSE_REJECTED', 'Control observations are invalid');
  const candidates = observations.filter(row => row.kind === 'PROGRESS' && typeof row.summary === 'string'
    && row.summary.startsWith(prefix) && detail.claims.some(claim => claim.claim_id === row.claim_id))
    .slice(-20).map(row => {
      const identity = { claimId: small(row.claim_id), turnId: small(row.turn_id), observedAt: small(row.observed_at) };
      let candidate;
      try {
        if (row.summary.length <= 2048 && row.summary_truncated !== true) candidate = JSON.parse(row.summary.slice(prefix.length));
      } catch { /* A bounded Runtime snapshot can clip this summary. */ }
      if (!candidate || Object.keys(candidate).sort().join(',') !== 'adopted,advisoryOnly,failureIdHash,reason,state'
          || !['steer_intent', 'skipped'].includes(candidate.state)
          || typeof candidate.reason !== 'string' || !/^[A-Z0-9_]{1,128}$/u.test(candidate.reason)
          || typeof candidate.failureIdHash !== 'string' || !taskPattern.test(candidate.failureIdHash)
          || candidate.advisoryOnly !== true || candidate.adopted !== false) {
        return { ...identity, state: 'UNREADABLE_CANDIDATE', requiresReadback: true };
      }
      return { ...identity, state: candidate.state, reason: candidate.reason, failureIdHash: candidate.failureIdHash,
        advisoryOnly: true, adopted: false };
    });
  return { status: candidates.length ? 'CANDIDATE_PROGRESS_RECORDED' : 'NOT_OBSERVED', candidates };
}
function taskSummary(detail, projectId, expectedTaskRef) {
  sourceIdentity(detail.source);
  taskReference(detail.id);
  if (detail.project_id !== projectId || detail.project?.id !== projectId
      || detail.task?.task_ref !== detail.id || detail.task?.ledger?.project_id !== projectId
      || expectedTaskRef && detail.id !== expectedTaskRef
      || typeof detail.completion_verified !== 'boolean' || !Array.isArray(detail.claims)) {
    throw failure('CONTROL_WORK_IDENTITY_REJECTED', 'Control task identity does not match the request');
  }
  const claims = detail.claims.map(claim => ({
    claimId: identifier(claim.claim_id, 'claim ID', 128), phase: small(claim.phase),
    threadId: small(claim.thread_id), turnId: small(claim.turn_id), inputId: small(claim.input_id),
    turnStatus: small(claim.turn_status), archived: claim.archived === true,
    pendingQuestions: (claim.pending_questions ?? []).map(question => ({
      approvalId: small(question.approval_id), kind: small(question.kind), method: small(question.payload?.method),
    })),
  }));
  return { taskRef: detail.id, projectId, status: small(detail.status),
    completionVerified: detail.completion_verified, resultDigest: small(detail.result_digest),
    updatedAt: small(detail.updated_at), source: sourceIdentity(detail.source), claims,
    lifeHarness: candidateProgress(detail),
    lastErrorCode: small(detail.last_error?.code),
  };
}

function validateOptions(options) {
  if (!commands.has(options.command)) throw new TypeError('unsupported formal work command');
  const specific = ['create', 'continue'].includes(options.command) ? ['inputFile']
    : ['status', 'start', 'interrupt'].includes(options.command) ? ['projectId', 'taskRef']
      : options.command === 'list' ? ['projectId'] : [];
  exactFields(options, ['command', ...specific], ['controlOrigin', 'requestTimeoutMs', 'fetchImpl']);
  if (specific.includes('projectId')) identifier(options.projectId, 'project');
  if (specific.includes('taskRef')) taskReference(options.taskRef);
  const timeout = options.requestTimeoutMs ?? 30_000;
  if (!Number.isInteger(timeout) || timeout < 1 || timeout > 60_000) throw new TypeError('timeout must be from 1 to 60000 milliseconds');
  const suppliedOrigin = options.controlOrigin ?? defaultControlOrigin;
  if (typeof suppliedOrigin !== 'string' || !/^http:\/\/127\.0\.0\.1(?::\d+)?\/?$/u.test(suppliedOrigin)) {
    throw new TypeError('origin must be an HTTP 127.0.0.1 origin without credentials, path or query');
  }
  return { origin: normalizeControlOrigin(suppliedOrigin), timeout };
}

export async function runFormalWorkCommand(options) {
  const { origin, timeout } = validateOptions(options);
  const fetchImpl = options.fetchImpl ?? fetch;
  if (typeof fetchImpl !== 'function') throw new TypeError('fetchImpl must be a function');
  const { command, projectId, taskRef } = options;
  const get = route => request(fetchImpl, origin, route, undefined, timeout);
  if (command === 'state' || command === 'projects') {
    const state = await get('/api/state');
    if (!Array.isArray(state.projects) || typeof state.formal_work_enabled !== 'boolean') {
      throw failure('CONTROL_RESPONSE_REJECTED', 'Control returned an invalid state');
    }
    const projects = state.projects.map(project => ({ id: small(project.id), name: small(project.name, 240),
      canonicalPath: small(project.canonical_path, 2048), registryAuthority: small(project.registry_authority),
      registryProjectId: small(project.registry_project_id),
    }));
    return { operation: command, formalWorkEnabled: state.formal_work_enabled,
      startupMode: small(state.formal_startup?.mode), projectCount: projects.length,
      lifeHarness: { enabled: typeof state.life_harness?.enabled === 'boolean' ? state.life_harness.enabled : null,
        scope: small(state.life_harness?.scope), mode: small(state.life_harness?.mode) },
      ...(command === 'projects' ? { projects } : {}),
    };
  }
  if (command === 'list') {
    const view = await get(`/api/work-view?projectId=${encodeURIComponent(projectId)}`);
    if (view.context?.project_id !== projectId || view.formal_work_enabled !== true) {
      throw failure('CONTROL_WORK_IDENTITY_REJECTED', 'Control work view does not match the project');
    }
    if (view.work_snapshot === null) return { operation: command, projectId, connected: false,
      reason: small(view.context.reason), tasks: null };
    const snapshot = object(view.work_snapshot, 'work snapshot');
    const source = sourceIdentity(snapshot.source);
    if (snapshot.project_id !== projectId || !Array.isArray(snapshot.tree?.nodes)) {
      throw failure('CONTROL_WORK_IDENTITY_REJECTED', 'Control returned an invalid work snapshot');
    }
    const tasks = snapshot.tree.nodes.map(row => ({ taskRef: taskReference(row.id), title: small(row.title, 240),
      status: small(row.status), priority: small(row.priority), completionVerified: row.completion_verified === true,
      parentTaskRef: row.parent_id == null ? null : taskReference(row.parent_id),
    }));
    return { operation: command, projectId, connected: true, source, revision: small(snapshot.revision),
      digest: small(snapshot.digest), taskCount: tasks.length, tasks };
  }
  if (command === 'create' || command === 'continue') {
    const input = validateInput(command, await readInput(options.inputFile));
    const { taskRef: inputTask, ...body } = input;
    const route = command === 'create' ? '/api/formal-work' : `/api/formal-work/${inputTask}/continue`;
    const detail = await request(fetchImpl, origin, route, body, timeout, command === 'create' ? 201 : 200);
    return { operation: command, ...(command === 'create' ? { clientRequestId: input.clientRequestId } : { inputId: input.inputId }),
      task: taskSummary(detail, input.projectId, inputTask) };
  }
  const route = `/api/formal-work/${taskRef}`;
  const detail = command === 'status' ? await get(`${route}?projectId=${encodeURIComponent(projectId)}`)
    : await request(fetchImpl, origin, `${route}/${command}`, { projectId }, timeout);
  return { operation: command, task: taskSummary(detail, projectId, taskRef) };
}

export function parseFormalWorkArguments(argv) {
  if (argv.length === 0 || argv.length === 1 && argv[0] === '--help') return { help: true };
  const [command, ...rest] = argv;
  if (!commands.has(command)) throw new TypeError('unsupported formal work command');
  if (rest.length === 1 && rest[0] === '--help') return { help: true };
  const options = { command };
  const names = new Map([['--project', 'projectId'], ['--task', 'taskRef'], ['--input', 'inputFile'],
    ['--origin', 'controlOrigin'], ['--timeout-ms', 'requestTimeoutMs']]);
  for (let index = 0; index < rest.length; index += 2) {
    const key = names.get(rest[index]), value = rest[index + 1];
    if (!key) throw new TypeError('unknown formal work option');
    if (Object.hasOwn(options, key)) throw new TypeError('duplicate formal work option');
    if (value === undefined || value.startsWith('--') || value === '') throw new TypeError('missing formal work option value');
    if (key === 'requestTimeoutMs' && !/^\d+$/u.test(value)) throw new TypeError('timeout must be an integer');
    options[key] = key === 'requestTimeoutMs' ? Number(value) : value;
  }
  validateOptions(options);
  return options;
}

export const formalWorkUsage = `LATTICE 正式工作 CLI（輸出必要狀態 JSON）
node apps/lattice-control/src/formal-work-client.mjs state
node apps/lattice-control/src/formal-work-client.mjs projects
node apps/lattice-control/src/formal-work-client.mjs list --project <id>
node apps/lattice-control/src/formal-work-client.mjs status --project <id> --task <taskRef>
node apps/lattice-control/src/formal-work-client.mjs create --input <JSONfile>
node apps/lattice-control/src/formal-work-client.mjs start --project <id> --task <taskRef>
node apps/lattice-control/src/formal-work-client.mjs continue --input <JSONfile>
node apps/lattice-control/src/formal-work-client.mjs interrupt --project <id> --task <taskRef>
create JSON: {"projectId":"已登記ID","objective":"工作內容","clientRequestId":"唯一且重試不變的ID"}
continue JSON: {"projectId":"已登記ID","taskRef":"64位小寫hex","inputId":"唯一且重試不變的ID","text":"補充內容"}
create 只登記；start 才明示派送，HTTP 確認不等於已開始或完成。此 CLI 不自動重試或回答權限問題。
同一 clientRequestId 必須重用同一份 create JSON；先查 status 再決定是否重試。
共通選項：--origin http://127.0.0.1:4317、--timeout-ms 30000（最大 60000）。`;

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const options = parseFormalWorkArguments(process.argv.slice(2));
    process.stdout.write(options.help ? `${formalWorkUsage}\n` : `${JSON.stringify(await runFormalWorkCommand(options))}\n`);
  } catch (error) {
    const code = typeof error.code === 'string' && /^[A-Z0-9_]{1,100}$/u.test(error.code) ? error.code : 'FORMAL_WORK_CLIENT_ERROR';
    process.stderr.write(`${JSON.stringify({ status: 'ERROR', code,
      message: error instanceof TypeError ? error.message : '正式工作操作未完成；請先查詢狀態，再沿用原請求身份重試。',
      ...(Number.isInteger(error.status) ? { httpStatus: error.status } : {}),
    })}\n`);
    process.exitCode = 1;
  }
}
