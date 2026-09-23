import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import assert from 'node:assert/strict';
import { isExecutionDenied } from '../../apps/lattice-control/src/execution-recovery.mjs';
import { buildCases } from './prepare.mjs';
import { buildControlledCases, verifyLocalCapture } from './capture.mjs';

export const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '../..');
export const hash = value => createHash('sha256').update(value).digest('hex');
const read = file => fs.readFileSync(file, 'utf8');
export const procedures = JSON.parse(read(path.join(here, 'procedures.json')));
const ids = procedures.map(p => p.id);
const hex = /^[a-f0-9]{64}$/u;
const labels = ['excluded', 'valid', 'none_applicable', 'insufficient_information'];
export const jsonl = file => read(file).trimEnd().split('\n').map(line => JSON.parse(line));
function keys(value, expected) {
  assert.ok(value !== null && typeof value === 'object' && !Array.isArray(value));
  assert.deepEqual(Object.keys(value).sort(), [...expected].sort(), 'unexpected object fields');
}
function string(value, maximum = 4096, allowEmpty = false) {
  assert.ok(typeof value === 'string' && (allowEmpty || value.length > 0) && value.length <= maximum && !value.includes('\0'));
}
const nullableString = value => { if (value !== null) string(value); };
const member = (value, values) => assert.ok(values.includes(value), `invalid enum: ${value}`);
function list(value, allowed) {
  assert.ok(Array.isArray(value) && new Set(value).size === value.length);
  value.forEach(v => member(v, allowed));
}

export function validateCase(c) {
  keys(c, ['schema_version', 'case_id', 'group_id', 'source', 'formal_refs', 'context', 'event']);
  assert.equal(c.schema_version, 1);
  assert.match(c.case_id, /^offline-[a-z0-9-]{1,100}$/u);
  string(c.group_id, 120);
  keys(c.source, ['kind', 'reference', 'selector', 'revision', 'sha256', 'evidence_sha256', 'native_observed', 'redactions']);
  member(c.source.kind, ['real', 'controlled_replay', 'synthetic']);
  string(c.source.reference); string(c.source.selector);
  assert.match(c.source.revision, /^[a-f0-9]{40}$/u);
  assert.match(c.source.sha256, hex); assert.match(c.source.evidence_sha256, hex);
  assert.equal(typeof c.source.native_observed, 'boolean');
  if (c.source.kind === 'synthetic') assert.equal(c.source.native_observed, false);
  assert.ok(Array.isArray(c.source.redactions) && c.source.redactions.length <= 10);
  c.source.redactions.forEach(v => string(v, 500));
  keys(c.formal_refs, ['project', 'task']);
  nullableString(c.formal_refs.project); nullableString(c.formal_refs.task);
  keys(c.context, ['project_scope', 'authority', 'lifecycle', 'freshness', 'circuit_open', 'category', 'platform']);
  member(c.context.project_scope, ['matched', 'mismatched', 'unknown']);
  member(c.context.authority, ['authorized', 'denied', 'unknown']);
  member(c.context.lifecycle, ['active', 'completed', 'cancelled', 'reopened', 'unknown']);
  member(c.context.freshness, ['current', 'expired', 'unknown']);
  member(c.context.category, ['tool_launch', 'dependency_resolution', 'other', 'unknown']);
  member(c.context.platform, ['win32', 'linux', 'darwin', 'unknown']);
  assert.equal(typeof c.context.circuit_open, 'boolean');
  keys(c.event, ['id', 'type', 'status', 'command', 'aggregatedOutput', 'exitCode']);
  string(c.event.id, 160);
  member(c.event.type, ['commandExecution', 'agentMessage', 'mcpToolCall']);
  member(c.event.status, ['failed', 'declined', 'completed', 'inProgress', 'cancelled']);
  string(c.event.command, 1000);
  if (c.event.aggregatedOutput !== null) string(c.event.aggregatedOutput, 4096, true);
  assert.ok(c.event.exitCode === null || Number.isSafeInteger(c.event.exitCode));
  assert.equal(c.source.evidence_sha256, hash(JSON.stringify({ event: c.event, context: c.context })));
  return c;
}

// All rejection/identity/lifecycle checks run before any retrieval. These fields
// are curated offline evidence, never a substitute for a production authority check.
export function exclusion(c) {
  if (isExecutionDenied(c.event) || c.context.authority === 'denied') return 'denied';
  if (c.context.circuit_open) return 'circuit_open';
  if (c.event.type !== 'commandExecution' || c.event.status !== 'failed') return 'not_native_failed';
  if (c.source.kind !== 'synthetic' && !c.source.native_observed) return 'native_evidence_missing';
  if (c.context.project_scope !== 'matched' || c.context.authority !== 'authorized') return 'identity_or_authority_unknown';
  if (c.context.lifecycle !== 'active') return 'lifecycle_inapplicable';
  if (c.context.freshness !== 'current') return 'stale_or_unknown';
  if (!['tool_launch', 'dependency_resolution'].includes(c.context.category)) return 'category_inapplicable';
  return null;
}

// Fixed lexical retrieval is deliberately separate from diagnostic selection.
export function retrieve(c) {
  const text = `${c.event.command}\n${c.event.aggregatedOutput ?? ''}`;
  return ids.filter(id => id === 'git-resolution' ? /\bgit\b|GIT_EXECUTABLE_UNAVAILABLE/iu.test(text) : /\bcodex\b/iu.test(text));
}
export function diagnose(c, candidates) {
  const output = c.event.aggregatedOutput ?? '';
  if (!output || /no diagnostic output/iu.test(output)) return { decision: 'abstain', selected_procedure: null, abstention_reason: 'insufficient_information' };
  const valid = candidates.filter(id => id === 'git-resolution'
    ? /spawn git ENOENT|GIT_EXECUTABLE_UNAVAILABLE|trusted absolute Git executable could not be resolved|git[^\n]*CommandNotFoundException/iu.test(output)
    : c.context.platform === 'win32' && /spawn codex ENOENT|Codex runtime was not found/iu.test(output));
  return valid.length
    ? { decision: 'selected', selected_procedure: valid[0], abstention_reason: null }
    : { decision: 'abstain', selected_procedure: null, abstention_reason: 'none_applicable' };
}

export function advise(c, group, bindings, retrieval = retrieve, annotationPrefix = '') {
  validateCase(c); member(group, ['A', 'B']);
  member(annotationPrefix, ['', 'controlled-']);
  const started = performance.now();
  const reason = exclusion(c);
  const candidates = reason ? [] : retrieval(c);
  list(candidates, ids);
  const decision = reason ? { decision: 'excluded', selected_procedure: null, abstention_reason: reason } : diagnose(c, candidates);
  const result = { schema_version: 1, case_id: c.case_id, group, source_kind: c.source.kind,
    source_revision: c.source.revision, evidence_sha256: c.source.evidence_sha256, ...bindings,
    candidate_version: 'lexical-v1', question_version: 'diagnosis-v1',
    model: { sdk: null, weights: null, encoder: null, tokenizer: null },
    candidates, ...decision, raw_score: null, calibrated_score: null,
    elapsed_ms: Number((performance.now() - started).toFixed(4)),
    outcome_ref: `annotations/${annotationPrefix}gold.jsonl#${c.case_id}`, advisory_only: true };
  validateAdvisory(result);
  return result;
}

export function validateAdvisory(a) {
  keys(a, ['schema_version', 'case_id', 'group', 'source_kind', 'source_revision', 'evidence_sha256',
    'dataset_sha256', 'procedures_sha256', 'implementation_sha256', 'candidate_version', 'question_version',
    'model', 'candidates', 'decision', 'selected_procedure', 'abstention_reason', 'raw_score',
    'calibrated_score', 'elapsed_ms', 'outcome_ref', 'advisory_only']);
  assert.equal(a.schema_version, 1); assert.match(a.case_id, /^offline-[a-z0-9-]{1,100}$/u);
  member(a.group, ['A', 'B']); member(a.source_kind, ['real', 'controlled_replay', 'synthetic']);
  assert.match(a.source_revision, /^[a-f0-9]{40}$/u);
  for (const key of ['dataset_sha256', 'procedures_sha256', 'implementation_sha256', 'evidence_sha256']) assert.match(a[key], hex);
  assert.equal(a.candidate_version, 'lexical-v1'); assert.equal(a.question_version, 'diagnosis-v1');
  keys(a.model, ['sdk', 'weights', 'encoder', 'tokenizer']);
  Object.values(a.model).forEach(v => assert.equal(v, null));
  assert.equal(a.raw_score, null); assert.equal(a.calibrated_score, null);
  assert.ok(Number.isFinite(a.elapsed_ms) && a.elapsed_ms >= 0);
  member(a.outcome_ref, [`annotations/gold.jsonl#${a.case_id}`, `annotations/controlled-gold.jsonl#${a.case_id}`]);
  assert.equal(a.advisory_only, true);
  list(a.candidates, ids); member(a.decision, ['selected', 'abstain', 'excluded']);
  if (a.decision === 'selected') {
    member(a.selected_procedure, a.candidates); assert.equal(a.abstention_reason, null);
  } else {
    assert.equal(a.selected_procedure, null); string(a.abstention_reason, 120);
    if (a.decision === 'excluded') assert.deepEqual(a.candidates, []);
  }
  return a;
}

export function validateAnnotations(rows, cases, datasetHash, proceduresHash, annotator) {
  assert.equal(rows.length, cases.length);
  assert.equal(new Set(rows.map(r => r.case_id)).size, cases.length);
  for (const row of rows) {
    keys(row, ['case_id', 'dataset_sha256', 'procedures_sha256', 'annotator', 'label', 'valid_procedures', 'reason']);
    assert.ok(cases.some(c => c.case_id === row.case_id));
    assert.equal(row.dataset_sha256, datasetHash); assert.equal(row.procedures_sha256, proceduresHash);
    assert.equal(row.annotator, annotator);
    member(row.label, labels); list(row.valid_procedures, ids); string(row.reason, 1000);
    assert.equal(row.valid_procedures.length > 0, row.label === 'valid');
  }
}

export function validateSources(cases) {
  // Only the fixed pilot sources and the explicitly captured controlled slice.
  // New sources require deliberate
  // import/redaction and reannotation, not a fabricated ref or a caller's hash.
  const expected = new Map(buildCases().map(c => [c.case_id, c]));
  if (cases.some(c => c.source.reference === 'experiments/life-harness/native-fixtures/captured-events.jsonl')) {
    for (const c of buildControlledCases()) expected.set(c.case_id, c);
  }
  for (const c of cases) {
    assert.ok(expected.has(c.case_id), 'unsupported source/case');
    assert.deepEqual(c, expected.get(c.case_id), 'source revision, selector or projection drift');
  }
}

export function validateAdjudication(record, alpha, beta, gold, bindings) {
  assert.equal(record.schema_version, 1);
  assert.equal(record.dataset_sha256, bindings.dataset_sha256);
  assert.equal(record.procedures_sha256, bindings.procedures_sha256);
  assert.equal(record.reviewed_cases, gold.length);
  assert.equal(record.annotation_completion_is_product_acceptance, false);
  assert.equal(new Set(Object.values(record.roles)).size, 3);
  const labelKey = row => JSON.stringify([row.label, [...row.valid_procedures].sort()]);
  const disagreed = [];
  for (const g of gold) {
    const a = alpha.find(r => r.case_id === g.case_id), b = beta.find(r => r.case_id === g.case_id);
    assert.ok(a && b);
    if (labelKey(a) !== labelKey(b)) disagreed.push(g.case_id);
    else assert.equal(labelKey(g), labelKey(a), 'unrecorded override of agreed annotation');
  }
  assert.deepEqual(record.disagreements.map(d => d.case_id).sort(), disagreed.sort());
  record.disagreements.forEach(d => string(d.reason, 1000));
}

export function evaluate(cases, gold, advisories) {
  const byId = new Map(gold.map(r => [r.case_id, r]));
  const metrics = {};
  for (const kind of ['real', 'controlled_replay', 'synthetic']) {
    const m = { cases: 0, valid_cases: 0, retrieval_hits: 0, retrieval_misses: 0,
      selection_hits: 0, ranking_errors: 0, abstentions_with_recall: 0,
      excluded: 0, boundary_violations: 0, correct_labels: 0, none_applicable: 0, insufficient_information: 0 };
    for (const c of cases.filter(c => c.source.kind === kind)) {
      const g = byId.get(c.case_id), a = advisories.find(a => a.case_id === c.case_id);
      assert.ok(g && a); m.cases++;
      const predictedLabel = a.decision === 'selected' ? 'valid' : a.decision === 'excluded' ? 'excluded' : a.abstention_reason;
      if (g.label === 'valid') {
        m.valid_cases++;
        const recalled = a.candidates.some(id => g.valid_procedures.includes(id));
        const hit = a.decision === 'selected' && g.valid_procedures.includes(a.selected_procedure);
        m.retrieval_hits += Number(recalled); m.retrieval_misses += Number(!recalled);
        m.selection_hits += Number(hit);
        m.ranking_errors += Number(recalled && a.decision === 'selected' && !hit);
        m.abstentions_with_recall += Number(recalled && a.decision !== 'selected');
        m.correct_labels += Number(hit);
      } else {
        m.correct_labels += Number(predictedLabel === g.label);
        if (g.label === 'excluded') { m.excluded++; m.boundary_violations += Number(a.decision !== 'excluded'); }
        if (g.label === 'none_applicable') m.none_applicable++;
        if (g.label === 'insufficient_information') m.insufficient_information++;
      }
    }
    m.retrieval_recall = m.valid_cases ? m.retrieval_hits / m.valid_cases : null;
    m.selection_accuracy_given_recall = m.retrieval_hits ? m.selection_hits / m.retrieval_hits : null;
    m.label_accuracy = m.cases ? m.correct_labels / m.cases : null;
    metrics[kind] = m;
  }
  return metrics;
}

export function replay({ controlled = false } = {}) {
  const prefix = controlled ? 'controlled-' : '';
  const dataset = read(path.join(here, `${prefix}cases.jsonl`));
  const cases = jsonl(path.join(here, `${prefix}cases.jsonl`));
  cases.forEach(validateCase);
  assert.equal(new Set(cases.map(c => c.case_id)).size, cases.length);
  validateSources(cases);
  const inputFiles = ['harness.mjs', 'prepare.mjs', 'capture.mjs', 'procedures.json', `${prefix}cases.jsonl`,
    ...['alpha.jsonl', 'beta.jsonl', 'gold.jsonl', 'adjudication.json'].map(name => `annotations/${prefix}${name}`),
    ...(controlled ? ['native-fixtures/captured-events.jsonl'] : [])];
  const inputs = Object.fromEntries(inputFiles.map(file => [file, hash(read(path.join(here, file)))]));
  const procedureSources = Object.fromEntries([...procedures.map(p => p.source), 'apps/lattice-control/src/execution-recovery.mjs']
    .map(file => [file, hash(read(path.join(root, file)).replaceAll('\r\n', '\n'))]));
  const bindings = { dataset_sha256: hash(dataset), procedures_sha256: inputs['procedures.json'],
    implementation_sha256: hash(JSON.stringify({ harness: inputs['harness.mjs'], capture: inputs['capture.mjs'], procedureSources })) };
  for (const who of ['alpha', 'beta', 'gold']) validateAnnotations(jsonl(path.join(here, `annotations/${prefix}${who}.jsonl`)), cases, bindings.dataset_sha256, bindings.procedures_sha256, who);
  const gold = jsonl(path.join(here, `annotations/${prefix}gold.jsonl`));
  validateAdjudication(JSON.parse(read(path.join(here, `annotations/${prefix}adjudication.json`))),
    jsonl(path.join(here, `annotations/${prefix}alpha.jsonl`)), jsonl(path.join(here, `annotations/${prefix}beta.jsonl`)), gold, bindings);
  const advisories = ['A', 'B'].flatMap(group => cases.map(c => advise(c, group, bindings, retrieve, prefix)));
  const A = evaluate(cases, gold, advisories.filter(a => a.group === 'A'));
  const B = evaluate(cases, gold, advisories.filter(a => a.group === 'B'));
  const summary = { schema_version: 1, experiment: controlled ? 'controlled-native-node-v1' : 'offline-pilot-v1', node_version: process.version,
    inputs, procedure_sources: procedureSources, bindings,
    ...(controlled ? { capture_verification: verifyLocalCapture() } : {}),
    comparison: { status: 'no_change', applied_change: null, qualifying_native_failure_count: cases.filter(c => c.source.kind !== 'synthetic' && c.source.native_observed && !exclusion(c)).length,
      real_diagnostic_increment: null, explanation: controlled
        ? 'Controlled native development slice; compare metrics against independently frozen labels. B unchanged. No production benefit evaluated.'
        : 'No qualifying native launch/dependency failure found in the original pilot; B equals frozen pilot A. No product or end-to-end benefit evaluated.' }, A, B };
  const out = path.join(here, 'results'); fs.mkdirSync(out, { recursive: true });
  fs.writeFileSync(path.join(out, `${prefix}advisories.jsonl`), advisories.map(a => JSON.stringify(a)).join('\n') + '\n');
  fs.writeFileSync(path.join(out, `${prefix}summary.json`), JSON.stringify(summary, null, 2) + '\n');
  console.log(JSON.stringify({ cases: cases.length, advisories: advisories.length, comparison: summary.comparison, A, B }));
  return summary;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  assert.ok(process.argv.length === 2 || (process.argv.length === 3 && process.argv[2] === '--controlled'));
  replay({ controlled: process.argv[2] === '--controlled' });
}
