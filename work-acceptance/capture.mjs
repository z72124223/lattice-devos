import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createHash, randomUUID } from 'node:crypto';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const ROOT = dirname(fileURLToPath(import.meta.url));
export const PROJECT = '78cc242a-fd7c-4d77-a895-1e5ade124618';
export const TASK = 'b3a5d42c96a94048df6084e150ecc6ed1e354f5eed1b0d1449d6e99942a1335d';
export const PARENT = '29ab1e48bf3ac5dd3ec1ed09dbafc224d67eb45551bc8e64520cbdf6d1dadd9c';
export const VERSION = '4cf44233b423f0234006e15b9b6ee460b3cf2ba9';
export const SOURCE_ROOT = `C:/Users/f7212/AppData/Local/Programs/LATTICE/versions/${VERSION}/app/control-runtime/apps/lattice-control`;
export const ENDPOINTS = Object.freeze({
  work: `http://127.0.0.1:4317/api/work-view?projectId=${PROJECT}`,
  detail: `http://127.0.0.1:4317/api/formal-work/${TASK}?projectId=${PROJECT}`,
});
export const SOURCE_FILES = Object.freeze({ formal: 'src/formal-work-store.mjs', view: 'public/work-view.mjs',
  store: 'src/store.mjs', recovery: 'src/execution-recovery.mjs', index: 'public/index.html' });
export const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
export const saveJson = (path, value) => writeFile(path, JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });

// Fixed GET allowlist; no Runtime mutation, retries, redirect following, or credentials.
export async function capture() {
  const directory = join(ROOT, 'snapshots', new Date().toISOString().replaceAll(':', '-') + '-' + randomUUID().slice(0, 8));
  await mkdir(directory, { recursive: true });
  const manifest = { schema_version: 'work-acceptance.capture.v1', started_at: new Date().toISOString(),
    project_id: PROJECT, task_ref: TASK, parent_ref: PARENT, source_version: VERSION, responses: {}, sources: {},
    boundaries: { runtime_mcp_status: 'NOT_VERIFIED: user rejected MCP tool call; not retried', runtime_writes: false } };
  try {
    for (const [name, url] of Object.entries(ENDPOINTS)) {
      const requested_at = new Date().toISOString();
      const response = await fetch(url, { method: 'GET', redirect: 'error', signal: AbortSignal.timeout(20000), headers: { Accept: 'application/json' } });
      const raw = Buffer.from(await response.arrayBuffer());
      const received_at = new Date().toISOString();
      const file = `${name}.response.json`;
      await writeFile(join(directory, file), raw, { flag: 'wx' });
      manifest.responses[name] = { url, method: 'GET', requested_at, received_at, status: response.status,
        content_type: response.headers.get('content-type'), date: response.headers.get('date'), file, bytes: raw.length, sha256: sha256(raw) };
      if (!response.ok) throw new Error(`HTTP_${response.status}: ${name}; response retained, no retry`);
      JSON.parse(raw.toString('utf8'));
    }
    for (const [name, relative] of Object.entries(SOURCE_FILES)) {
      const path = `${SOURCE_ROOT}/${relative}`;
      const raw = await readFile(path);
      const file = `${name}.source.txt`;
      await writeFile(join(directory, file), raw, { flag: 'wx' });
      manifest.sources[name] = { path, file, captured_at: new Date().toISOString(), bytes: raw.length, sha256: sha256(raw) };
    }
    manifest.capture_succeeded = true;
  } catch (error) {
    manifest.capture_succeeded = false;
    manifest.error = { name: error.name, message: error.message };
    throw Object.assign(error, { snapshot_directory: directory });
  } finally {
    manifest.finished_at = new Date().toISOString();
    await saveJson(join(directory, 'manifest.json'), manifest);
  }
  return directory;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  capture().then((directory) => console.log(directory)).catch((error) => { console.error(error.message, error.snapshot_directory); process.exitCode = 1; });
}
