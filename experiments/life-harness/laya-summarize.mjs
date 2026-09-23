import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { here, hash, evaluate, validateAnnotations } from './harness.mjs';
import { prepareFrozen } from './laya-inputs.mjs';

// Evaluation reads labels only after the isolated worker has finished. It does
// not change questions, scores, labels, retrieval, or the frozen A/B artifacts.
const read = file => fs.readFileSync(path.join(here, file), 'utf8');
const rows = file => read(file).trimEnd().split('\n').map(JSON.parse);
const report = JSON.parse(read('results/laya-advisories.json'));
assert.equal(report.exit_code, 0);
assert.equal(report.stopped, null);
assert.equal(report.result.mode, 'advisories');
const expected = prepareFrozen(), actual = report.result.records;
assert.equal(actual.length, expected.length);
for (const [i, e] of expected.entries()) {
  for (const [key, value] of Object.entries(e)) {
    if (key !== 'request') assert.deepEqual(actual[i][key], value);
  }
  assert.equal(actual[i].adopted, false);
  assert.equal(actual[i].calibrated_score, null);
  if (e.gate.decision !== 'request') assert.equal(actual[i].model_called, false);
}
const datasets = [];
for (const prefix of ['', 'controlled-', 'resolver-']) {
  const file = `${prefix}cases.jsonl`, cases = rows(file), gold = rows(`annotations/${prefix}gold.jsonl`);
  const selected = actual.filter(r => r.dataset_file === file);
  const bindings = { dataset: hash(read(file)), procedures: hash(read('procedures.json')) };
  validateAnnotations(gold, cases, bindings.dataset, bindings.procedures, 'gold');
  for (const r of selected) {
    assert.equal(r.dataset_sha256, bindings.dataset);
    assert.equal(r.procedures_sha256, bindings.procedures);
  }
  const baseline = JSON.parse(read(`results/${prefix}summary.json`));
  assert.deepEqual(baseline.A, baseline.B);
  const advisoryView = selected.map(r => ({ ...r, abstention_reason: r.reason }));
  const disagreements = selected.flatMap(r => {
    const g = gold.find(g => g.case_id === r.case_id);
    const correct = g.label === 'valid' ? r.decision === 'selected' && g.valid_procedures.includes(r.selected_procedure)
      : g.label === 'excluded' ? r.decision === 'excluded' : r.decision === 'abstain' && r.reason === g.label;
    return correct ? [] : [{ case_id: r.case_id, gold_label: g.label, valid_procedures: g.valid_procedures,
      model_called: r.model_called, decision: r.decision, choice: r.selected_procedure ?? r.reason }];
  });
  datasets.push({ file, dataset_sha256: bindings.dataset, cases: cases.length,
    model_calls: selected.filter(r => r.model_called).length,
    excluded_before_model: selected.filter(r => r.gate.decision === 'excluded').length,
    empty_candidates_before_model: selected.filter(r => r.gate.decision === 'abstain').length,
    refused_truncation: selected.filter(r => r.reason === 'input_would_be_truncated').length,
    A_equals_B: baseline.A, C_equals_D: evaluate(cases, gold, advisoryView), disagreements });
}
const summary = { schema_version: 1, experiment: 'local-laya-multilingual-choice-v1',
  baseline_commit: '792e0104f84d0132d84f23a506ad1ae176c1812b',
  model_output_sha256: hash(read('results/laya-advisories.json')),
  sdk_revision: report.result.sdk_revision, model_revision: report.result.model_revision,
  comparison: { status: 'offline_observation_only', A_equals_B: true, C_equals_D: true,
    model_passes: 1, independent_C_D_runs: false, product_benefit: null, calibrated: false, adopted: false,
    explanation: 'C=A retrieval plus local choice; D=B retrieval plus local choice. A=B, so one identical-input output set represents C=D. Development sets, no causal or generalization claim.' },
  datasets };
fs.writeFileSync(path.join(here, 'results/laya-summary.json'), JSON.stringify(summary, null, 2) + '\n');
console.log(JSON.stringify(summary));
