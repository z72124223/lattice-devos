import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
import { prepareFrozen, syntheticSmoke } from './laya-inputs.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const local = path.resolve(here, '../../.lattice/life-harness/laya-local');
const GiB = 1024 ** 3;
const used = dir => fs.readdirSync(dir, { withFileTypes: true }).reduce((sum, e) => sum +
  (e.isDirectory() ? used(path.join(dir, e.name)) : fs.statSync(path.join(dir, e.name)).size), 0);
function resources(enforce = true) {
  const disk = fs.statfsSync(local), projectBytes = used(local), free = disk.bavail * disk.bsize;
  if (enforce) assert.ok(projectBytes < 6 * GiB && free >= 10 * GiB, 'disk budget unavailable');
  return { project_bytes: projectBytes, disk_free_bytes: free };
}
export function workerEnv() {
  const env = Object.fromEntries(['SystemRoot', 'WINDIR', 'ComSpec'].filter(k => process.env[k]).map(k => [k, process.env[k]]));
  const profile = path.join(local, 'home');
  for (const dir of [profile, path.join(local, 'temp'), path.join(profile, 'AppData/Local'), path.join(profile, 'AppData/Roaming')]) fs.mkdirSync(dir, { recursive: true });
  return { ...env, Path: path.join(process.env.SystemRoot, 'System32'),
    USERPROFILE: profile, HOME: profile, APPDATA: path.join(profile, 'AppData/Roaming'), LOCALAPPDATA: path.join(profile, 'AppData/Local'),
    TEMP: path.join(local, 'temp'), TMP: path.join(local, 'temp'), HF_HOME: path.join(profile, 'huggingface'),
    HF_HUB_OFFLINE: '1', TRANSFORMERS_OFFLINE: '1', HF_HUB_DISABLE_IMPLICIT_TOKEN: '1', HF_HUB_DISABLE_TELEMETRY: '1',
    HF_HUB_DISABLE_XET: '1', DO_NOT_TRACK: '1', TOKENIZERS_PARALLELISM: 'false', PYTHONUTF8: '1',
    PYTHONDONTWRITEBYTECODE: '1', TORCHDYNAMO_DISABLE: '1' };
}

export async function run(mode) {
  assert.ok(['smoke', 'advisories'].includes(mode));
  const before = resources();
  const records = mode === 'smoke' ? [syntheticSmoke()] : prepareFrozen();
  const attempts = fs.readdirSync(local).filter(f => new RegExp(`^${mode}-[12]-input\\.json$`).test(f)).length;
  assert.ok(attempts < 2, 'two attempts already recorded; stop this path');
  const prefix = path.join(local, `${mode}-${attempts + 1}`);
  fs.writeFileSync(prefix + '-input.json', JSON.stringify({ mode, records }) + '\n', { flag: 'wx' });
  const stdout = fs.openSync(prefix + '-stdout.txt', 'wx'), stderr = fs.openSync(prefix + '-stderr.txt', 'wx');
  const start = performance.now();
  const child = spawn(path.join(local, 'venv/Scripts/python.exe'), ['-I', '-S', '-B', path.join(here, 'laya_choice.py'), prefix + '-input.json'],
    { cwd: here, env: workerEnv(), windowsHide: true, stdio: ['ignore', stdout, stderr] });
  let stopped = null;
  const deadline = setTimeout(() => { stopped = 'wall_timeout_300s'; child.kill(); }, 300_000);
  const diskGuard = setInterval(() => { try { resources(); } catch { stopped = 'disk_budget'; child.kill(); } }, 1000);
  let exitCode = null, spawnError = null;
  try {
    exitCode = await new Promise((resolve, reject) => { child.once('error', reject); child.once('exit', resolve); });
  } catch (error) {
    spawnError = { code: error.code, message: error.message };
  } finally {
    clearTimeout(deadline); clearInterval(diskGuard); fs.closeSync(stdout); fs.closeSync(stderr);
  }
  const text = fs.readFileSync(prefix + '-stdout.txt', 'utf8').trim();
  let result = null, parseError = null;
  try { if (text) result = JSON.parse(text.split('\n').at(-1)); } catch (e) { parseError = e.message; }
  const report = { schema_version: 1, attempt: attempts + 1, exit_code: exitCode, stopped, spawn_error: spawnError,
    wall_seconds: (performance.now() - start) / 1000, resources_before: before, resources_after: resources(false), parse_error: parseError, result };
  fs.writeFileSync(path.join(here, `results/laya-${mode}.json`), JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify({ mode, attempt: report.attempt, exit_code: exitCode, stopped, wall_seconds: report.wall_seconds,
    project_bytes: report.resources_after.project_bytes, peak_rss_bytes: result?.peak_rss_bytes, load_seconds: result?.load_seconds,
    records: result?.records?.map(r => ({ case_id: r.case_id, model_called: r.model_called, decision: r.decision,
      selected_procedure: r.selected_procedure, reason: r.reason, inference_seconds: r.inference_seconds, raw_model_output: r.raw_model_output })) }));
  assert.equal(exitCode, 0, `worker failed; inspect ${path.basename(prefix)}-stderr.txt in ignored local storage`);
  assert.ok(result?.status === 'completed' && !stopped && !parseError && result.peak_rss_bytes <= 8 * GiB && result.worker_seconds < 300);
  return report;
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await run(process.argv[2]);
