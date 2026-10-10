import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { here, hash, jsonl, validateCase, validateSources, exclusion } from './harness.mjs';

const base = '82e398856d3627b7220057957a64f30bd0f5d2a7';
const read = file => fs.readFileSync(path.join(here, file), 'utf8');
export const candidate = JSON.parse(read('relative-module-candidate.json'));
const fixture = 'native-fixtures/dependency-failure.mjs';

// Only this already captured fixture is a supported static source. Never open
// an importer, target, URL or command obtained from an error message.
function staticEvidence() {
  const captured = jsonl(path.join(here, 'native-fixtures/captured-events.jsonl'))
    .find(row => row.scenario === 'dependency-missing');
  const source = read(fixture).replaceAll('\r\n', '\n');
  assert.equal(candidate.fixture_source, fixture);
  assert.equal(hash(source), candidate.fixture_sha256);
  assert.equal(hash(source), captured.source.fixture_files_sha256['dependency-failure.mjs']);
  assert.equal(captured.record.payload.item.cwd, '[FIXTURE]');
  assert.equal(captured.scope.fixture_cwd_matched, true);
  const imports = [...source.matchAll(/^import ['"](\.\/[a-zA-Z0-9._/-]+)['"];$/gmu)];
  assert.equal(imports.length, 1, 'single static relative import required');
  const importer = path.posix.basename(fixture), specifier = imports[0][1];
  const target = path.posix.normalize(path.posix.join(path.posix.dirname(importer), specifier));
  assert.ok(!target.startsWith('../') && !path.posix.isAbsolute(target));
  return { cwd: '[FIXTURE]', importer, specifier, target, source_sha256: hash(source),
    target_exists_now: fs.existsSync(path.join(here, 'native-fixtures', target)) };
}

export function adviseRelativeModule(c, { static_evidence = true } = {}) {
  validateCase(c);
  const output = { case_id: c.case_id, source_kind: c.source.kind, evidence_sha256: c.source.evidence_sha256,
    candidate_id: candidate.id, decision: 'abstain', reason: null, static_evidence: null, hint: null,
    advisory_only: true, adopted: false };
  const reject = exclusion(c);
  if (reject) return { ...output, decision: 'excluded', reason: reject };
  if (c.source.kind !== 'synthetic') {
    try { validateSources([c]); }
    catch { return { ...output, reason: 'insufficient_information' }; }
  }
  const text = c.event.aggregatedOutput ?? '';
  if (!text) return { ...output, reason: 'insufficient_information' };
  if (c.context.category !== 'dependency_resolution' || /Cannot find package\b/u.test(text)
      || !text.includes('ERR_MODULE_NOT_FOUND')) return { ...output, reason: 'none_applicable' };
  if (!static_evidence) return { ...output, reason: 'insufficient_information' };
  let evidence;
  try { evidence = staticEvidence(); }
  catch { return { ...output, reason: 'insufficient_information' }; }
  if (c.event.command !== `node ./${evidence.importer}`) return { ...output, reason: 'insufficient_information' };
  const errors = [...text.replaceAll('\\', '/').matchAll(/^Error \[ERR_MODULE_NOT_FOUND\]: Cannot find module '([^'\r\n]+)' imported from ([^\r\n]+)$/gmu)];
  if (errors.length !== 1 || errors[0][1] !== `${evidence.cwd}/${evidence.target}`
      || errors[0][2] !== `${evidence.cwd}/${evidence.importer}` || evidence.target_exists_now) {
    return { ...output, reason: 'insufficient_information' };
  }
  return { ...output, decision: 'selected', static_evidence: evidence, hint: candidate.hint };
}

export function developmentCases() {
  const cases = jsonl(path.join(here, 'controlled-cases.jsonl'));
  validateSources(cases);
  return cases;
}

export function syntheticCases() {
  const source = read('relative-module-cases.json'), scenarios = JSON.parse(source).scenarios;
  const template = developmentCases().find(c => c.case_id === 'offline-controlled-dependency-missing');
  return scenarios.map((scenario, index) => {
    const c = structuredClone(template);
    c.case_id = `offline-relative-${scenario.id}`;
    c.group_id = 'relative-module-synthetic-development';
    Object.assign(c.event, scenario.event);
    Object.assign(c.context, scenario.context);
    if (scenario.append_output) c.event.aggregatedOutput += scenario.append_output;
    c.source = { kind: 'synthetic', reference: 'experiments/life-harness/relative-module-cases.json',
      selector: `scenarios/${index}`, revision: base, sha256: hash(source), native_observed: false,
      evidence_sha256: hash(JSON.stringify({ event: c.event, context: c.context })),
      redactions: ['Explicit synthetic development mutation of the known fixture; never a native event.'] };
    return { c, options: { static_evidence: scenario.static_evidence !== false }, expected: scenario.expected };
  });
}

export function replayRelativeModule() {
  const development = developmentCases().map(c => adviseRelativeModule(c));
  const synthetic = syntheticCases().map(({ c, options, expected }) => ({ ...adviseRelativeModule(c, options), expected }));
  const result = { baseline_commit: base, candidate_sha256: hash(read('relative-module-candidate.json')),
    implementation_sha256: hash(read('relative-module.mjs')), synthetic_cases_sha256: hash(read('relative-module-cases.json')),
    scope: 'New candidate library coverage on a seen development example; old A none_applicable was correct. No production integration, model, holdout or benefit claim.',
    development, synthetic };
  fs.writeFileSync(path.join(here, 'results/relative-module.json'), JSON.stringify(result, null, 2) + '\n');
  assert.deepEqual(development.map(r => r.decision === 'abstain' ? r.reason : r.decision), ['none_applicable', 'selected', 'excluded']);
  assert.ok(synthetic.every(r => (r.decision === 'abstain' ? r.reason : r.decision) === r.expected));
  console.log(JSON.stringify({ development: development.map(r => ({ case_id: r.case_id, decision: r.decision, reason: r.reason })),
    synthetic_passed: synthetic.length, adopted: false }));
  return result;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) replayRelativeModule();
