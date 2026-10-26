// Local acceptance only: use the actual application, recording selected HTTP headers.
import { startServer } from '../../server.mjs';
import { appendFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const phase = process.argv[2];
if (!['before', 'after'].includes(phase)) throw new Error('Use before or after');
const server = await startServer({
  port: Number(process.env.PORT || 0),
  dataFile: fileURLToPath(new URL(`./browser-${phase}-data.json`, import.meta.url)),
});
const logFile = fileURLToPath(new URL(`./browser-${phase}-http.jsonl`, import.meta.url));
server.prependListener('request', (request, response) => {
  const writeHead = response.writeHead;
  let policy;
  response.writeHead = function (...args) {
    policy = args.at(-1)?.['Referrer-Policy'];
    return writeHead.apply(this, args);
  };
  response.once('finish', () => appendFileSync(logFile, JSON.stringify({
    time: new Date().toISOString(), method: request.method, path: request.url,
    host: request.headers.host, origin: request.headers.origin ?? null,
    referer: request.headers.referer ?? null,
    fetchSite: request.headers['sec-fetch-site'] ?? null,
    fetchMode: request.headers['sec-fetch-mode'] ?? null,
    status: response.statusCode, referrerPolicy: policy,
  }) + '\n'));
});
console.log(`http://127.0.0.1:${server.address().port}`);
for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => server.close());
