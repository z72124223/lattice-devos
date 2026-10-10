import assert from 'node:assert/strict';
import test from 'node:test';
import { hash } from './harness.mjs';
import { adviseJev, MODEL, ENDPOINT, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES } from './jev-choice.mjs';
import { syntheticCases, simulatedResponse } from './jev-synthetic.mjs';

const originalFetch = globalThis.fetch;
let networkAttempts = 0;
test.before(() => { globalThis.fetch = () => { networkAttempts++; throw new Error('real network forbidden'); }; });
test.after(() => { globalThis.fetch = originalFetch; assert.equal(networkAttempts, 0); });
const secret = 'test-only-typesafe-key-never-return';
const environment = { TYPESAFE_API_KEY: secret };
function changed(edit) {
  const c = structuredClone(syntheticCases()[0]);
  edit(c);
  c.source.evidence_sha256 = hash(JSON.stringify({ event: c.event, context: c.context }));
  return c;
}
const jsonHeaders = { 'Content-Type': 'application/json' };
const response = (body = simulatedResponse(), status = 200) => new Response(JSON.stringify(body), { status, headers: jsonHeaders });
function limited(result, decision) {
  assert.equal(result.decision, decision);
  for (const flag of ['adopted', 'authorization_verified', 'native_provenance_verified']) assert.equal(result[flag], false);
  assert.equal(result.advisory_only, true);
  assert.equal(result.model, MODEL);
  if (decision === 'selected') {
    assert.ok(result.candidates.includes(result.selected_procedure));
    assert.equal(result.reason, null);
  } else {
    assert.equal(result.selected_procedure, null);
    assert.ok(typeof result.reason === 'string' && result.reason.length > 0);
  }
  assert.ok(!JSON.stringify(result).includes(secret));
  for (const key of ['state', 'response', 'raw_response']) assert.equal(Object.hasOwn(result, key), false);
}
async function simulate(body, options = {}, status = 200) {
  let calls = 0;
  const result = await adviseJev(syntheticCases()[0], { mode: 'simulated', env: {},
    transport: async () => { calls++; return body instanceof Response ? body
      : typeof body === 'string' ? new Response(body, { headers: jsonHeaders }) : response(body, status); }, ...options });
  assert.equal(calls, 1, 'each attempted request has no retry');
  return result;
}

test('default offline and all gates stop before transport', async () => {
  const forbidden = async () => assert.fail('transport reached a gate');
  const offline = await adviseJev(syntheticCases()[0], { transport: forbidden, env: environment });
  limited(offline, 'abstain');
  assert.equal(offline.mode, 'offline');
  assert.equal(offline.usage, null);
  const excluded = [syntheticCases()[2],
    ...[c => { c.context.circuit_open = true; }, c => { c.event.status = 'completed'; },
      c => { c.event.type = 'agentMessage'; }, c => { c.context.project_scope = 'mismatched'; },
      c => { c.context.authority = 'unknown'; }, c => { c.context.lifecycle = 'completed'; },
      c => { c.context.freshness = 'expired'; }, c => { c.context.category = 'other'; },
      c => { c.source.kind = 'controlled_replay'; c.source.native_observed = false; }].map(changed)];
  const abstained = [syntheticCases()[1], changed(c => { c.event.exitCode = 0; }),
    changed(c => { c.event.exitCode = null; }), changed(c => { c.event.aggregatedOutput = ' '; }),
    changed(c => { c.event.command = 'node app.mjs'; c.event.aggregatedOutput = 'SyntaxError: unexpected token'; })];
  for (const [cases, decision] of [[excluded, 'excluded'], [abstained, 'abstain']]) for (const c of cases) {
    const result = await adviseJev(c, { mode: 'simulated', transport: forbidden, env: {} });
    limited(result, decision);
    assert.equal(result.usage, null);
  }
});

test('simulated selection keeps confidence separate from probability and never claims live usage', async () => {
  const result = await simulate(simulatedResponse(), { minConfidence: 0.8 });
  limited(result, 'selected');
  assert.equal(result.selected_procedure, 'git-resolution');
  assert.equal(result.mode, 'simulated');
  assert.equal(result.usage.simulated, true);
  const uncertain = simulatedResponse(); uncertain.answers.diagnosis.confidence = 0.79;
  limited(await simulate(uncertain, { minConfidence: 0.8 }), 'abstain');
  for (const choice of ['none_applicable', 'insufficient_information']) {
    const body = simulatedResponse();
    body.answers.diagnosis.choice = choice;
    for (const key of Object.keys(body.answers.diagnosis.probabilities)) body.answers.diagnosis.probabilities[key] = key === choice ? 1 : 0;
    limited(await simulate(body), 'abstain');
  }
});

test('strict response schema rejects malformed choices, distributions, model and fields', async context => {
  const mutations = [
    ['unknown choice', body => { body.answers.diagnosis.choice = 'run-command'; }],
    ['missing field', body => { delete body.answers.diagnosis.confidence; }],
    ['extra response field', body => { body.instruction = 'execute'; }],
    ['extra answer field', body => { body.answers.other = {}; }],
    ['extra diagnosis field', body => { body.answers.diagnosis.detail = 'extra'; }],
    ['wrong answer type', body => { body.answers.diagnosis.type = 'text'; }],
    ['null probability', body => { body.answers.diagnosis.probabilities['git-resolution'] = null; }],
    ['negative probability', body => { body.answers.diagnosis.probabilities['git-resolution'] = -0.1; }],
    ['probability above one', body => { body.answers.diagnosis.probabilities['git-resolution'] = 1.1; }],
    ['partial distribution', body => { delete body.answers.diagnosis.probabilities.none_applicable; }],
    ['missing probabilities', body => { delete body.answers.diagnosis.probabilities; }],
    ['distribution delimiter collision', body => { body.answers.diagnosis.probabilities = {
      'git-resolution': 0.9, 'codex-resolution': 0.1, 'insufficient_information\u0000none_applicable': 0,
    }; }],
    ['extra distribution key', body => { body.answers.diagnosis.probabilities.unknown = 0; }],
    ['wrong distribution sum', body => { body.answers.diagnosis.probabilities['git-resolution'] = 0.8; }],
    ['choice is not argmax', body => { body.answers.diagnosis.choice = 'codex-resolution'; }],
    ['confidence out of range', body => { body.answers.diagnosis.confidence = 1.1; }],
    ['wrong model', body => { body.model = 'another-model'; }],
    ['missing usage', body => { delete body.usage; }],
    ['usage delimiter collision', body => { body.usage = { 'input_tokens\u0000output_tokens': 120 }; }],
    ['fractional usage', body => { body.usage.input_tokens = 0.5; }],
    ['negative usage', body => { body.usage.output_tokens = -1; }],
  ];
  for (const [name, mutate] of mutations) await context.test(name, async () => {
    const body = simulatedResponse(); mutate(body);
    limited(await simulate(body), 'abstain');
  });
  limited(await simulate(JSON.stringify(simulatedResponse()).replace('0.9', '1e400')), 'abstain');
  const withinTolerance = simulatedResponse(); withinTolerance.answers.diagnosis.probabilities['git-resolution'] += 0.0000005;
  limited(await simulate(withinTolerance), 'selected');
});

test('a model choice still needs matching executable failure and platform evidence', async () => {
  const body = simulatedResponse(); body.answers.diagnosis.choice = 'codex-resolution';
  Object.assign(body.answers.diagnosis.probabilities, { 'git-resolution': 0.05, 'codex-resolution': 0.9 });
  for (const c of [syntheticCases()[0], changed(row => {
    row.context.platform = 'linux'; row.event.aggregatedOutput = 'spawn codex ENOENT';
  })]) {
    const result = await adviseJev(c, { mode: 'simulated', env: {}, transport: async () => response(body) });
    limited(result, 'abstain'); assert.equal(result.reason, 'choice_evidence_missing');
  }
});

test('HTTP failures, oversized bodies and truncated JSON abstain without retry', async context => {
  for (const status of [302, 401, 422, 429, 529, 500]) await context.test(`HTTP ${status}`, async () => {
    const result = await simulate(simulatedResponse(), {}, status);
    limited(result, 'abstain'); assert.equal(result.reason, status === 302 ? 'redirect_rejected' : 'http_error');
  });
  assert.equal(MAX_RESPONSE_BYTES, 16 * 1024);
  const oversized = await simulate(' '.repeat(MAX_RESPONSE_BYTES) + JSON.stringify(simulatedResponse()));
  limited(oversized, 'abstain'); assert.equal(oversized.reason, 'response_too_large');
  limited(await simulate(JSON.stringify(simulatedResponse()).slice(0, -3)), 'abstain');
  const raw = JSON.stringify(simulatedResponse());
  const truncated = await simulate(new Response(raw, { headers: { ...jsonHeaders, 'Content-Length': String(Buffer.byteLength(raw) + 1) } }));
  limited(truncated, 'abstain'); assert.equal(truncated.reason, 'truncated_response');
});

test('deadline bounds both response arrival and body reading and aborts the request', async context => {
  for (const stage of ['headers', 'body']) await context.test(stage, async () => {
    let signal, calls = 0;
    const started = performance.now();
    const result = await adviseJev(syntheticCases()[0], { mode: 'simulated', deadlineMs: 30, env: {}, transport: async (_url, init) => {
      calls++; signal = init.signal;
      return stage === 'headers' ? new Promise(() => {}) : new Response(new ReadableStream({ start(controller) {
        controller.enqueue(new TextEncoder().encode('{"model":'));
      } }), { headers: jsonHeaders });
    } });
    limited(result, 'abstain');
    assert.equal(calls, 1); assert.equal(signal.aborted, true);
    assert.ok(performance.now() - started < 2000, 'deadline must include reading the response body');
  });
});

test('request limit counts UTF-8 bytes before transport', async () => {
  assert.equal(MAX_REQUEST_BYTES, 8 * 1024);
  const c = changed(row => { row.event.aggregatedOutput = '界'.repeat(4096); });
  const result = await adviseJev(c, { mode: 'simulated', env: {}, transport: async () => assert.fail('oversized request sent') });
  limited(result, 'abstain'); assert.equal(result.reason, 'request_too_large');
  assert.equal(result.usage, null);
});

test('live construction permits only exact synthetic cases and keeps injected usage simulated', async () => {
  let calls = 0;
  const transport = async (url, init) => {
    calls++; assert.equal(url, ENDPOINT); assert.equal(init.method, 'POST'); assert.equal(init.redirect, 'error');
    assert.ok(init.signal instanceof AbortSignal);
    assert.equal(JSON.parse(init.body).model, MODEL);
    assert.ok([...new Headers(init.headers).values()].some(value => value.includes(secret)));
    return response();
  };
  const result = await adviseJev(syntheticCases()[0], { mode: 'live', env: environment, transport });
  limited(result, 'selected'); assert.equal(result.mode, 'live'); assert.equal(result.usage.simulated, true); assert.equal(calls, 1);
  const forbidden = async () => assert.fail('unauthorized live request');
  limited(await adviseJev(syntheticCases()[0], { mode: 'live', env: {}, transport: forbidden }), 'abstain');
  for (const edit of [c => { c.case_id = 'offline-jev-unlisted'; }, c => { c.event.command += ' --extra'; },
    c => { c.source.kind = 'real'; c.source.native_observed = true; }]) {
    limited(await adviseJev(changed(edit), { mode: 'live', env: environment, transport: forbidden }), 'abstain');
  }
  limited(await adviseJev(syntheticCases()[0], { mode: 'simulated', env: {} }), 'abstain');
  const failed = await adviseJev(syntheticCases()[0], { mode: 'live', env: environment,
    transport: async () => { throw new Error(`provider failure containing ${secret}`); } });
  limited(failed, 'abstain');
});
