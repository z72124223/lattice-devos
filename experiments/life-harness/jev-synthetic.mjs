import { hash } from './harness.mjs';

// New, non-sensitive protocol fixtures. Zero revision is a synthetic sentinel,
// not a native observation, commit provenance, formal identity or quality label.
export function syntheticCases() {
  return ['eligible', 'missing-output', 'denied'].map(name => {
    const event = { id: `jev-synthetic-${name}`, type: 'commandExecution', status: 'failed',
      command: 'synthetic git codex lookup (data only; do not execute)',
      aggregatedOutput: name === 'missing-output' ? null : 'Synthetic protocol fixture: spawn git ENOENT', exitCode: 1 };
    const context = { project_scope: 'matched', authority: name === 'denied' ? 'denied' : 'authorized',
      lifecycle: 'active', freshness: 'current', circuit_open: false, category: 'tool_launch', platform: 'win32' };
    return { schema_version: 1, case_id: `offline-jev-contract-${name}`, group_id: 'jev-contract-synthetic-v1',
      source: { kind: 'synthetic', reference: 'inline:jev-contract-synthetic-v1', selector: name,
        revision: '0'.repeat(40), sha256: hash(JSON.stringify({ name, event, context })),
        evidence_sha256: hash(JSON.stringify({ event, context })), native_observed: false, redactions: [] },
      formal_refs: { project: null, task: null }, context, event };
  });
}

// Canned protocol response only; usage is simulated, never billing or accuracy.
export function simulatedResponse() {
  return { model: 'jev-1.13.0', answers: { diagnosis: { type: 'choice', choice: 'git-resolution',
    probabilities: { 'git-resolution': 0.9, 'codex-resolution': 0.05, none_applicable: 0.03, insufficient_information: 0.02 },
    confidence: 0.85 } }, usage: { input_tokens: 100, output_tokens: 20 } };
}
