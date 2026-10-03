export const MODEL = 'jev-1.13.0';
export const ENDPOINT = 'https://api.typesafe.ai/v1/systemone';
// Local byte budgets, not tokenizer measurements or the service's token limits.
export const MAX_REQUEST_BYTES = 8 * 1024;
export const MAX_RESPONSE_BYTES = 16 * 1024;
const SUM_TOLERANCE = 1e-6; // Local rounding policy, not an API guarantee.
const exactKeys = (value, keys) => value !== null && typeof value === 'object' && !Array.isArray(value)
  && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
const probability = value => typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= 1;
const fail = reason => { throw Object.assign(new Error(reason), { reason }); };

function parseResponse(value, options) {
  if (!exactKeys(value, ['model', 'answers', 'usage']) || value.model !== MODEL
      || !exactKeys(value.answers, ['diagnosis']) || !exactKeys(value.usage, ['input_tokens', 'output_tokens'])
      || !Object.values(value.usage).every(n => Number.isSafeInteger(n) && n >= 0)) fail('invalid_response');
  const answer = value.answers.diagnosis;
  if (!exactKeys(answer, ['type', 'choice', 'probabilities', 'confidence']) || answer.type !== 'choice'
      || !options.includes(answer.choice) || !probability(answer.confidence)
      || !exactKeys(answer.probabilities, options) || !Object.values(answer.probabilities).every(probability)) fail('invalid_response');
  const values = Object.values(answer.probabilities);
  if (Math.abs(values.reduce((a, b) => a + b, 0) - 1) > SUM_TOLERANCE
      || answer.probabilities[answer.choice] !== Math.max(...values)) fail('invalid_response');
  return { answer, usage: value.usage };
}

async function readResponse(response, signal) {
  if (signal.aborted) fail('deadline_exceeded');
  if (!response || !Number.isInteger(response.status)) fail('invalid_response');
  if (response.redirected || (response.url && response.url !== ENDPOINT)
      || (response.status >= 300 && response.status < 400)) fail('redirect_rejected');
  if (response.status !== 200) fail('http_error');
  if (!/^application\/json(?:\s*;|$)/iu.test(response.headers?.get('content-type') ?? '')) fail('invalid_response');
  const encoding = response.headers.get('content-encoding');
  if (encoding && encoding.toLowerCase() !== 'identity') fail('unsupported_encoding');
  const declared = response.headers.get('content-length');
  if (declared !== null && (!/^(?:0|[1-9][0-9]*)$/u.test(declared) || !Number.isSafeInteger(Number(declared)))) fail('invalid_response');
  if (Number(declared) > MAX_RESPONSE_BYTES) fail('response_too_large');
  if (!response.body?.getReader) fail('invalid_response');
  const reader = response.body.getReader(), chunks = [];
  const cancel = () => { void reader.cancel().catch(() => {}); };
  signal.addEventListener('abort', cancel, { once: true });
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!(value instanceof Uint8Array)) fail('invalid_response');
      size += value.byteLength;
      if (size > MAX_RESPONSE_BYTES) fail('response_too_large');
      chunks.push(value);
    }
    if (declared !== null && size !== Number(declared)) fail('truncated_response');
    try { return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks))); }
    catch { fail('invalid_response'); }
  } finally { signal.removeEventListener('abort', cancel); cancel(); }
}

export async function exchangeJevChoice(body, options, { headers, transport, deadlineMs }) {
  const controller = new AbortController();
  let timer, httpStatus = null;
  try {
    const timeout = new Promise((_, reject) => {
      timer = setTimeout(() => { reject(Object.assign(new Error('deadline_exceeded'), { reason: 'deadline_exceeded' })); controller.abort(); }, deadlineMs);
    });
    const operation = Promise.resolve().then(() => (transport ?? globalThis.fetch)(ENDPOINT,
      { method: 'POST', headers, body, redirect: 'error', signal: controller.signal }))
      .then(response => {
        if (Number.isInteger(response?.status) && response.status >= 100 && response.status <= 599) httpStatus = response.status;
        return readResponse(response, controller.signal);
      });
    const value = await Promise.race([operation, timeout]);
    return { ...parseResponse(value, options), httpStatus };
  } catch (error) {
    const allowed = ['invalid_response', 'redirect_rejected', 'http_error', 'unsupported_encoding',
      'response_too_large', 'truncated_response', 'deadline_exceeded'];
    const reason = allowed.includes(error?.reason) ? error.reason : 'transport_failed';
    throw Object.assign(new Error(reason), { reason, httpStatus });
  } finally { clearTimeout(timer); controller.abort(); }
}
