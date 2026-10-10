import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { previewProjectPurge, applyProjectPurge, statusProjectPurge, nativeProjectPurge } from './project-purge.mjs';
import { projectPurgeReport } from './project-purge-report.mjs';
import { finalizeProjectPurge, resumeProjectPurgeFinalization, readProjectPurgeAttestation } from './project-purge-finalize.mjs';

export const projectPurgeUsage = `LATTICE 專案清除（所有專案共用；唯讀盤點、離線清除）
npm run project:purge -- preview --input config.json --plan plan.json
npm run project:purge -- inventory --input config.json --plan plan.json
npm run project:purge -- apply --plan plan.json --confirm DIGEST --maintenance-offline
npm run project:purge -- resume --plan plan.json --confirm DIGEST --maintenance-offline
npm run project:purge -- status --plan plan.json
npm run project:purge -- verify --plan plan.json
npm run project:purge -- finalize --plan plan.json --confirm DIGEST --maintenance-offline
npm run project:purge -- resume-finalize --finalization attestation.json --confirm DIGEST --maintenance-offline
npm run project:purge -- verify-finalization --finalization attestation.json
npm run project:purge -- reconcile-claim --input archive-proof.json --confirm INPUT_SHA256 --maintenance-offline
Bot 清除需要時，在 apply/resume 加上 --bot-boundaries 已於盤點綁定路徑的最新原生證據檔。
preview/inventory 可在線上唯讀盤點；未停機或未安裝維護元件時仍列出範圍與阻擋原因。
停止服務、安裝元件後須重新盤點並確認新摘要。resume 沿用同一計畫，不自動重建或解除鎖。
verify 僅在所有清除範圍均已驗證時回傳 0；局部完成回傳 2，阻擋或錯誤回傳 1。
封存或從名單移除不等於完整刪除；外部紀錄未驗證時不會回報完整清除。`;

function parse(argv) {
  if (argv.length === 0 || (argv.length === 1 && ['help', '--help', '-h'].includes(argv[0]))) return { help: true };
  const [action, ...args] = argv;
  const allowed = {
    preview: ['--input', '--plan'], inventory: ['--input', '--plan'],
    apply: ['--plan', '--confirm', '--maintenance-offline', '--bot-boundaries'], resume: ['--plan', '--confirm', '--maintenance-offline', '--bot-boundaries'],
    status: ['--plan'], verify: ['--plan'],
    finalize: ['--plan', '--confirm', '--maintenance-offline'],
    'resume-finalize': ['--finalization', '--confirm', '--maintenance-offline'],
    'verify-finalization': ['--finalization'],
    'reconcile-claim': ['--input', '--confirm', '--maintenance-offline'],
  };
  if (!Object.hasOwn(allowed, action)) throw new Error('PURGE_CLI_ACTION_INVALID');
  if (args.length === 1 && args[0] === '--help') return { help: true };
  const options = { action };
  for (let index = 0; index < args.length; index++) {
    const key = args[index];
    if (!allowed[action].includes(key) || Object.hasOwn(options, key)) throw new Error('PURGE_CLI_OPTION_INVALID');
    if (key === '--maintenance-offline') options[key] = true;
    else {
      const value = args[++index];
      if (typeof value !== 'string' || !value.trim() || value.startsWith('--')) throw new Error('PURGE_CLI_VALUE_REQUIRED');
      options[key] = value;
    }
  }
  if (action === 'reconcile-claim') {
    if (!options['--input']) throw new Error('PURGE_INPUT_REQUIRED');
  } else if (['resume-finalize','verify-finalization'].includes(action)) {
    if (!options['--finalization']) throw new Error('PURGE_FINALIZATION_PATH_REQUIRED');
  } else if (!options['--plan']) throw new Error('PURGE_PLAN_REQUIRED');
  if (['preview', 'inventory'].includes(action) && !options['--input']) throw new Error('PURGE_INPUT_REQUIRED');
  if (['apply', 'resume', 'finalize', 'resume-finalize', 'reconcile-claim'].includes(action)
    && (!/^[a-f0-9]{64}$/.test(options['--confirm'] ?? '') || !options['--maintenance-offline'])) throw new Error('PURGE_EXACT_CONFIRMATION_REQUIRED');
  return options;
}

async function jsonFile(file, withBytes = false) {
  const bytes = await readFile(file);
  if (bytes.length > 16 * 1024 * 1024) throw new Error('PURGE_INPUT_TOO_LARGE');
  try { const value = JSON.parse(bytes.toString('utf8')); return withBytes ? { value, bytes } : value; } catch { throw new Error('PURGE_INPUT_INVALID_JSON'); }
}

export async function runProjectPurgeCli(argv, { preview = previewProjectPurge, apply = applyProjectPurge, status = statusProjectPurge, reconcile = nativeProjectPurge } = {}) {
  try {
    const options = parse(argv);
    if (options.help) return { output: `${projectPurgeUsage}\n`, exitCode: 0 };
    if (options.action === 'reconcile-claim') {
      const { value, bytes } = await jsonFile(path.resolve(options['--input']), true);
      if (createHash('sha256').update(bytes).digest('hex') !== options['--confirm']) throw new Error('PURGE_INPUT_DIGEST_CHANGED');
      if (!value || Object.keys(value).length !== 2 || typeof value.nativeBinary !== 'string' || !path.isAbsolute(value.nativeBinary)
        || value.request?.schema !== 'lattice.project-purge.request.v1' || value.request?.action !== 'reconcile-archived-claim'
        || value.request?.authorization !== 'RECONCILE_ARCHIVED_CLAIM') throw new Error('PURGE_RECONCILIATION_INPUT_INVALID');
      const result = await reconcile(value.nativeBinary, value.request);
      if (result?.schema !== 'lattice.claim-reconciliation.result.v1' || result.status !== 'RECONCILED'
        || result.projectId !== value.request.projectId || result.claimId !== value.request.claimId
        || result.taskCompletionChanged !== false || result.projectDeletionExecuted !== false) throw new Error('PURGE_RECONCILIATION_RESULT_INVALID');
      return { output: `${JSON.stringify(result, null, 2)}\n`, exitCode: 0 };
    }
    if (['resume-finalize','verify-finalization','finalize'].includes(options.action)) {
      const result = options.action === 'resume-finalize'
        ? await resumeProjectPurgeFinalization(path.resolve(options['--finalization']), { confirmDigest: options['--confirm'], maintenanceOffline: options['--maintenance-offline'] })
        : options.action === 'verify-finalization' ? await readProjectPurgeAttestation(path.resolve(options['--finalization']))
        : await finalizeProjectPurge(await jsonFile(path.resolve(options['--plan'])), { confirmDigest: options['--confirm'], maintenanceOffline: options['--maintenance-offline'], status });
      return { output: `${JSON.stringify(result, null, 2)}\n`, exitCode: 0 };
    }
    const planPath = path.resolve(options['--plan']);
    let result, plan;
    if (['preview', 'inventory'].includes(options.action)) {
      const inputPath = path.resolve(options['--input']);
      const { value: config, bytes } = await jsonFile(inputPath, true);
      if (!config || Array.isArray(config) || Object.keys(config).some(key => !['projectId', 'operationId', 'nativeBinary', 'databasePath', 'statePath', 'protectedRoots', 'externalResources', 'codeGraphCacheDirectory', 'runtimeGraphWorkDirectory', 'botService', 'botBoundaryPath', 'registryPolicy'].includes(key))) throw new Error('PURGE_CONFIG_INVALID');
      plan = await preview({ ...config, protectedRoots: [...(config.protectedRoots ?? []), planPath, inputPath],
        maintenance: { planPath, configPath: inputPath, configDigest: createHash('sha256').update(bytes).digest('hex') } });
      await writeFile(planPath, JSON.stringify(plan, null, 2), { flag: 'wx', mode: 0o600 });
      result = { ...plan, report: projectPurgeReport(plan) };
    } else {
      plan = await jsonFile(planPath);
      if (options['--bot-boundaries'] && plan.maintenance && path.resolve(options['--bot-boundaries']) !== plan.maintenance.boundaryPath) throw new Error('PURGE_BOUNDARY_PATH_NOT_PLANNED');
      result = ['apply', 'resume'].includes(options.action)
        ? await apply(plan, { confirmDigest: options['--confirm'], maintenanceOffline: options['--maintenance-offline'],
          botBoundaries: options['--bot-boundaries'] ? await jsonFile(path.resolve(options['--bot-boundaries'])) : [] })
        : await status(plan);
    }
    const report = result.report ?? projectPurgeReport(plan, result);
    const exitCode = result.status === 'BLOCKED' ? 1 : options.action === 'verify'
      ? (report.complete === true ? 0 : 2)
      : ['apply', 'resume', 'status'].includes(options.action) && result.status !== 'SCOPED_PURGED' ? 2 : 0;
    return { output: `${JSON.stringify({ ...result, report }, null, 2)}\n`, exitCode };
  } catch (error) {
    const code = /^[A-Z][A-Z0-9_]{0,100}$/.test(error.message) ? error.message
      : /^[A-Z][A-Z0-9_]{0,40}$/.test(error.code ?? '') ? error.code : 'PURGE_CLI_FAILED';
    return { output: `${JSON.stringify({ status: 'ERROR', code })}\n`, exitCode: 1 };
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const result = await runProjectPurgeCli(process.argv.slice(2));
  process.stdout.write(result.output); process.exitCode = result.exitCode;
}
