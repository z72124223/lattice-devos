// One explicitly named, authorized Codex rollout; never scans session folders or executes probes.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '../..');
const fixture = path.join(here, 'native-fixtures');
const hash = data => createHash('sha256').update(data).digest('hex');
const captureFile = path.join(fixture, 'captured-events.jsonl');
const reviewedCaptureHash = '6fa9f76741d45983f7df534eedfcf150cac7f194ba427a098c4ad373d41c1a3a';
const localRaw = path.join(root, '.lattice/life-harness/native-node-20260923/raw-capture.jsonl');
export const scenarios = [
  { id: 'entry-missing', command: 'node ./intentionally-absent-entry.mjs', category: 'tool_launch' },
  { id: 'dependency-missing', command: 'node ./dependency-failure.mjs', category: 'dependency_resolution' },
  { id: 'success', command: 'node ./success.mjs', category: 'dependency_resolution' },
];
const readLines = file => fs.readFileSync(file, 'utf8').trimEnd().split('\n');

export function selectNative(lines, threadId, turnId, fixtureRoot) {
  const rows = lines.map((raw, index) => ({ raw, line: index + 1, record: JSON.parse(raw) }));
  assert.equal(rows[0].record.type, 'session_meta');
  assert.equal(rows[0].record.payload.id, threadId, 'wrong Codex task');
  const start = rows.find(r => r.record.type === 'event_msg' && r.record.payload.type === 'task_started' && r.record.payload.turn_id === turnId);
  const context = rows.find(r => r.record.type === 'turn_context' && r.record.payload.turn_id === turnId);
  assert.ok(start && context, 'missing original turn scope');
  const selected = scenarios.map(s => {
    const matches = rows.filter(({ record: r }) => r.type === 'event_msg' && r.payload.type === 'item_completed'
      && r.payload.thread_id === threadId && r.payload.turn_id === turnId
      && r.payload.item?.type === 'CommandExecution' && Array.isArray(r.payload.item.command)
      && r.payload.item.command.at(-1) === s.command);
    assert.equal(matches.length, 1, `expected exactly one native event for ${s.id}`);
    const row = matches[0], item = row.record.payload.item;
    assert.ok(start.line < row.line && context.line < row.line);
    assert.equal(path.resolve(fileURLToPath(item.cwd)), path.resolve(fixtureRoot), 'wrong fixture scope');
    assert.ok(!rows.some(r => r.line < row.line && r.line > start.line && r.record.type === 'event_msg'
      && ['task_complete', 'turn_aborted'].includes(r.record.payload.type) && r.record.payload.turn_id === turnId), 'terminal capture turn');
    assert.ok(['failed', 'completed', 'declined'].includes(item.status), 'unknown native status');
    assert.ok(Number.isSafeInteger(item.exit_code) || item.exit_code === null);
    return { ...row, scenario: s };
  });
  return { selected, start, context, cliVersion: rows[0].record.payload.cli_version };
}

function deidentify(record, threadId, turnId, fixtureRoot, caseId) {
  const item = record.payload.item;
  const replacements = [
    [item.cwd, '[FIXTURE]'], [pathToFileURL(fixtureRoot).href, '[FIXTURE]'],
    [fixtureRoot.replaceAll('\\', '/'), '[FIXTURE]'], [fixtureRoot, '[FIXTURE]'],
    [item.command[0], '[SHELL]'], [threadId, '[THIS_CODEX_TASK]'], [turnId, '[CAPTURE_TURN]'],
    [item.id, caseId],
  ];
  const walk = value => typeof value === 'string'
    ? replacements.reduce((v, [from, to]) => v.replaceAll(from, to), value)
    : Array.isArray(value) ? value.map(walk)
      : value && typeof value === 'object' ? Object.fromEntries(Object.entries(value).map(([k, v]) => [k, walk(v)])) : value;
  const result = walk(record);
  assert.ok(!/[A-Za-z]:[\\/]/u.test(JSON.stringify(result)), 'unredacted absolute path');
  return result;
}

export function capture(rollout, threadId, turnId) {
  const rawPath = localRaw;
  const indexPath = path.join(path.dirname(rawPath), 'source-index.json');
  const casesPath = path.join(here, 'controlled-cases.jsonl');
  for (const output of [captureFile, rawPath, indexPath, casesPath]) {
    assert.ok(!fs.existsSync(output), `capture archive already exists: ${output}`);
  }
  assert.ok(path.isAbsolute(rollout));
  assert.ok(path.basename(rollout).endsWith(`-${threadId}.jsonl`), 'explicit own-task rollout required');
  const { selected, start, context, cliVersion } = selectNative(readLines(rollout), threadId, turnId, fixture);
  const raw = [start, context, ...selected].sort((a, b) => a.line - b.line).map(r => r.raw).join('\n') + '\n';
  fs.mkdirSync(path.dirname(rawPath), { recursive: true });
  fs.writeFileSync(rawPath, raw, { flag: 'wx' }); // Preserve an existing raw capture.
  const baseline = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim();
  const fileHashes = Object.fromEntries(['dependency-failure.mjs', 'available-dependency.mjs', 'success.mjs']
    .map(file => [file, hash(fs.readFileSync(path.join(fixture, file)))]));
  const captured = selected.map(({ raw: line, line: sourceLine, record, scenario }) => ({
    schema_version: 1, kind: 'controlled_replay', scenario: scenario.id,
    source: { format: 'codex-rollout-item_completed', line: sourceLine, raw_event_sha256: hash(line),
      raw_capture_sha256: hash(raw), raw_turn_context_sha256: hash(context.raw),
      cli_version: cliVersion, baseline_revision: baseline, fixture_files_sha256: fileHashes },
    scope: { authorization: 'explicit_user_controlled_fault_injection', as_of: 'native_item_completion',
      original_thread_and_turn_matched: true, fixture_cwd_matched: true, turn_active_at_event: true,
      formal_project: null, formal_task: null },
    record: deidentify(record, threadId, turnId, fixture, `offline-controlled-${scenario.id}`),
  }));
  fs.writeFileSync(captureFile, captured.map(r => JSON.stringify(r)).join('\n') + '\n', { flag: 'wx' });
  fs.writeFileSync(indexPath, JSON.stringify({ rollout,
    threadId, turnId, sourceLines: [start.line, context.line, ...selected.map(r => r.line)], raw_capture_sha256: hash(raw) }, null, 2) + '\n', { flag: 'wx' });
  fs.writeFileSync(casesPath, buildControlledCases().map(r => JSON.stringify(r)).join('\n') + '\n', { flag: 'wx' });
  console.log(JSON.stringify(captured.map(c => ({ scenario: c.scenario, native_type: c.record.payload.item.type,
    status: c.record.payload.item.status, exit_code: c.record.payload.item.exit_code, raw_sha256: c.source.raw_event_sha256 }))));
}

export function reviewedCapture(content) {
  assert.equal(hash(content), reviewedCaptureHash, 'captured native projection differs from reviewed snapshot');
  return content.trimEnd().split('\n').map(JSON.parse);
}

export function buildControlledCases() {
  if (!fs.existsSync(captureFile)) return [];
  const content = fs.readFileSync(captureFile, 'utf8'), captured = reviewedCapture(content);
  assert.equal(captured.length, scenarios.length);
  return captured.map((c, index) => {
    const s = scenarios[index], p = c.record.payload, item = p.item;
    assert.equal(c.schema_version, 1); assert.equal(c.kind, 'controlled_replay'); assert.equal(c.scenario, s.id);
    assert.equal(c.record.type, 'event_msg'); assert.equal(p.type, 'item_completed');
    assert.equal(item.type, 'CommandExecution'); assert.equal(item.command.at(-1), s.command);
    assert.equal(item.cwd, '[FIXTURE]');
    for (const [file, digest] of Object.entries(c.source.fixture_files_sha256)) {
      assert.ok(['dependency-failure.mjs', 'available-dependency.mjs', 'success.mjs'].includes(file));
      assert.equal(hash(fs.readFileSync(path.join(fixture, file))), digest, 'fixture drift');
    }
    assert.deepEqual(c.scope, { authorization: 'explicit_user_controlled_fault_injection', as_of: 'native_item_completion',
      original_thread_and_turn_matched: true, fixture_cwd_matched: true, turn_active_at_event: true, formal_project: null, formal_task: null });
    const event = { id: item.id, type: 'commandExecution', status: item.status,
      command: item.command.at(-1), aggregatedOutput: item.aggregated_output, exitCode: item.exit_code };
    const context = { project_scope: 'matched', authority: 'authorized', lifecycle: 'active', freshness: 'current',
      circuit_open: false, category: s.category, platform: 'win32' };
    return { schema_version: 1, case_id: `offline-controlled-${s.id}`, group_id: 'controlled-node-capture-20260923',
      source: { kind: 'controlled_replay', reference: 'experiments/life-harness/native-fixtures/captured-events.jsonl',
        selector: `line/${index + 1}`, revision: c.source.baseline_revision, sha256: hash(content),
        evidence_sha256: hash(JSON.stringify({ event, context })), native_observed: true,
        redactions: ['exact native event retained locally in ignored .lattice capture; paths and Codex identities replaced in tracked projection',
          'CommandExecution to commandExecution is casing only; status/exit_code retained without success inference',
          'context describes the authorized controlled slice at native item completion, never a current task authorization'] },
      formal_refs: { project: null, task: null }, context, event };
  });
}

export function verifyLocalCapture(rawPath = localRaw) {
  buildControlledCases(); // Always verify the reviewed portable projection and fixture bytes.
  if (!fs.existsSync(rawPath)) return { portable_projection: 'verified', native_original: 'not_available_locally' };
  verifyRawCapture(fs.readFileSync(rawPath, 'utf8'));
  return { portable_projection: 'verified', native_original: 'verified_from_local_raw_capture' };
}

export function verifyRawCapture(raw) {
  const captured = reviewedCapture(fs.readFileSync(captureFile, 'utf8'));
  const rows = raw.trimEnd().split('\n').map(line => ({ line, record: JSON.parse(line) }));
  for (const c of captured) {
    assert.equal(hash(raw), c.source.raw_capture_sha256, 'raw capture drift');
    const source = rows.find(r => r.record.ordinal === c.record.ordinal);
    assert.ok(source); assert.equal(hash(source.line), c.source.raw_event_sha256);
    const p = source.record.payload;
    const context = rows.find(r => r.record.type === 'turn_context' && r.record.payload.turn_id === p.turn_id);
    assert.ok(context); assert.equal(hash(context.line), c.source.raw_turn_context_sha256);
    assert.deepEqual(deidentify(source.record, p.thread_id, p.turn_id, fileURLToPath(p.item.cwd), c.record.payload.item.id), c.record);
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  assert.equal(process.argv.length, 5, 'usage: node capture.mjs <explicit-own-rollout> <own-task-id> <capture-turn-id>');
  capture(...process.argv.slice(2));
}
