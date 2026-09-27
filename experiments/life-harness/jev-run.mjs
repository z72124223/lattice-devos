import { adviseJev } from './jev-choice.mjs';
import { syntheticCases, simulatedResponse } from './jev-synthetic.mjs';

const args = process.argv.slice(2);
let mode = 'offline', minConfidence = 0.8;
if (['--simulate', '--live'].includes(args[0])) mode = args.shift() === '--live' ? 'live' : 'simulated';
if (args[0] === '--min-confidence' && args.length === 2 && args[1].trim()) {
  minConfidence = Number(args[1]); args.splice(0, 2);
}
if (args.length || !Number.isFinite(minConfidence) || minConfidence < 0 || minConfidence > 1) {
  console.error('Usage: node experiments/life-harness/jev-run.mjs [--simulate|--live] [--min-confidence 0..1]');
  process.exitCode = 1;
} else {
  const cases = mode === 'live' ? syntheticCases().slice(0, 1) : syntheticCases();
  let transportCalls = 0;
  const transport = mode === 'simulated' ? async () => {
    transportCalls += 1;
    return Response.json(simulatedResponse());
  } : undefined;
  const results = [];
  for (const c of cases) results.push({ case_id: c.case_id, ...await adviseJev(c, { mode, minConfidence, transport }) });
  console.log(JSON.stringify({ experiment: 'jev-contract-synthetic-v1', mode,
    live_auth: mode === 'live' ? 'NOT_INDEPENDENTLY_VERIFIED' : 'NOT_RUN',
    live_inference: mode === 'live' ? (results[0].usage ? 'RESPONSE_RECEIVED' : 'NOT_COMPLETED') : 'NOT_RUN',
    quality: 'NOT_RUN', simulated_transport_calls: transportCalls, results }, null, 2));
  if (mode === 'live' && !results[0].usage) process.exitCode = 1;
}
