import { readFile, mkdir } from 'node:fs/promises';
import { join, resolve, relative, isAbsolute, basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { runInNewContext } from 'node:vm';
import { randomUUID } from 'node:crypto';
import { ROOT, PROJECT, TASK, PARENT, VERSION, SOURCE_ROOT, SOURCE_FILES, ENDPOINTS, capture, sha256, saveJson } from './capture.mjs';

const HASH = /^[a-f0-9]{64}$/u;
const ensure = (condition, message) => { if (!condition) throw new Error(message); };
const timestamp = (value) => typeof value === 'string' && Number.isFinite(Date.parse(value));
const sameSet = (a, b) => a.length === b.length && new Set(a).size === a.length && a.every((x) => b.includes(x));

export function verifyCapturedBytes(record, bytes) {
  ensure(record.bytes === bytes.length && record.sha256 === sha256(bytes), `CAPTURE_HASH: ${record.file}`);
}

export async function loadSnapshot(directory, { requireAllSources = true } = {}) {
  const manifest = JSON.parse(await readFile(join(directory, 'manifest.json'), 'utf8'));
  ensure(manifest.schema_version === 'work-acceptance.capture.v1' && manifest.capture_succeeded === true, 'CAPTURE_INCOMPLETE');
  ensure(manifest.project_id === PROJECT && manifest.task_ref === TASK && manifest.parent_ref === PARENT, 'CAPTURE_IDENTITY');
  ensure(manifest.source_version === VERSION, 'SOURCE_VERSION');
  ensure(timestamp(manifest.started_at) && timestamp(manifest.finished_at)
    && Date.parse(manifest.finished_at) >= Date.parse(manifest.started_at), 'CAPTURE_TIME');
  const readVerified = async (record, file) => {
    ensure(record?.file === file && basename(file) === file, 'CAPTURE_FILE');
    const bytes = await readFile(join(directory, file));
    verifyCapturedBytes(record, bytes);
    return bytes.toString('utf8');
  };
  const bundle = { directory, manifest, sources: {} };
  for (const [name, url] of Object.entries(ENDPOINTS)) {
    const record = manifest.responses?.[name];
    ensure(record?.url === url && record.method === 'GET' && record.status === 200, `CAPTURE_HTTP: ${name}`);
    ensure(timestamp(record.requested_at) && timestamp(record.received_at)
      && Date.parse(record.received_at) >= Date.parse(record.requested_at), `CAPTURE_HTTP_TIME: ${name}`);
    ensure(/^application\/json(?:;|$)/u.test(record.content_type), `CAPTURE_CONTENT_TYPE: ${name}`);
    bundle[name] = JSON.parse(await readVerified(record, `${name}.response.json`));
  }
  for (const [name, file] of Object.entries(SOURCE_FILES)) {
    const record = manifest.sources?.[name];
    if (!record && !requireAllSources) continue;
    ensure(record?.path === `${SOURCE_ROOT}/${file}` && timestamp(record.captured_at), `CAPTURE_SOURCE: ${name}`);
    bundle.sources[name] = await readVerified(record, `${name}.source.txt`);
  }
  return bundle;
}

// Execute only extracted pure functions, never import the installed store/client.
// JSON crosses the VM boundary; the context has no process, require, fetch or DOM.
function evaluatePure(code, input) {
  const context = { inputJson: JSON.stringify(input) };
  const result = runInNewContext(`'use strict'; ${code}`, context, {
    timeout: 250, contextCodeGeneration: { strings: false, wasm: false },
  });
  return JSON.parse(result);
}
export function installedProjection(sources, task, product) {
  const priorities = sources.formal.match(/const priorities = \[[^\n]+;/u)?.[0];
  const circuit = sources.recovery?.match(/export const openCircuitSummary = ("[^\n]+");/u)?.[1];
  const start = sources.formal.indexOf('function pendingCount(');
  const end = sources.formal.indexOf('export function projectFormalWork(');
  ensure(priorities && circuit && start >= 0 && end > start, 'SOURCE_PROJECTION_BOUNDARY');
  return evaluatePure(`${priorities}\nconst openCircuitSummary = ${circuit};\n${sources.formal.slice(start, end)}
    const args = JSON.parse(inputJson); JSON.stringify(taskProjection(args.task, args.product));`, { task, product });
}
export function installedAppearance(sources, work) {
  const start = sources.view.indexOf('const palette =');
  const end = sources.view.indexOf('export const workFilters =');
  ensure(start >= 0 && end > start, 'SOURCE_APPEARANCE_BOUNDARY');
  return evaluatePure(`${sources.view.slice(start, end).replaceAll('export function ', 'function ')}
    JSON.stringify(workAppearance(JSON.parse(inputJson)));`, work);
}

export function auditSources(sources) {
  const checks = [], findings = [];
  const check = (id, file, text, meaning) => {
    const offset = sources[file]?.indexOf(text) ?? -1;
    checks.push({ id, passed: offset >= 0, file: SOURCE_FILES[file],
      line: offset < 0 ? null : sources[file].slice(0, offset).split('\n').length, meaning });
  };
  check('postgres_authority', 'formal', 'kind: "POSTGRESQL_CONTROL_PRODUCT", authority: "POSTGRESQL_TASK_LEDGER"', '正式來源宣告為 PostgreSQL Control Product / Task Ledger。');
  check('runtime_snapshot_read', 'formal', 'this.runtime.call("lattice_control_snapshot",', '透過既有 Runtime 讀取正式快照。');
  check('project_guard', 'formal', 'page.project?.id !== projectId', '拒絕錯誤專案。');
  check('stable_identity', 'formal', 'id: task.task_ref, project_id: task.ledger.project_id', '節點使用固定 task_ref 與 Ledger project_id。');
  check('formal_completion', 'formal', 'task.ledger.status === "COMPLETED" && Boolean(task.ledger.result_digest)', '正式完成必須同時具備 COMPLETED 與 result_digest。');
  check('reply_not_completion', 'formal', 'execution?.turn_status === "TURN_COMPLETED") status = "codex_done"', 'AI 回合完成僅映射 codex_done。');
  check('metadata_parent', 'formal', 'parent_work_item_id: row.metadata?.parent_ref ?? null', '父工作由正式 metadata.parent_ref 投影。');
  check('parent_forward', 'store', 'parentByChild.set(relation.work_item_id, relation.parent_work_item_id)', '建立 child 到 parent 關係。');
  check('parent_reverse', 'store', 'childrenByParent.get(relation.parent_work_item_id).push(relation.work_item_id)', '同時建立 parent 到 children 反向關係。');
  check('orphan_guard', 'store', 'CONTROL_WORK_ORPHAN_REJECTED', '拒絕缺少父節點。');
  check('cycle_guard', 'store', 'CONTROL_WORK_HIERARCHY_CYCLE_REJECTED', '拒絕循環階層。');
  check('completion_label', 'view', "if (work.completion_verified === true) return palette.complete", '正式完成旗標優先顯示已完成。');
  check('ai_reply_label', 'view', "if (work.status === 'codex_done') return palette.review", 'codex_done 顯示等待驗收。');
  check('cache_15s', 'formal', 'cacheMs = 15000', '專案列表快取預設 15 秒；與 detail GET 非同一原子快照。');
  check('render_identity', 'view', '`${context.project_id}:${data?.revision}:${data?.digest}:${message}`', '專案／revision／digest／訊息變化時重新顯示。');
  check('render_on_change', 'view', 'if (key !== lastKey) { lastKey = key; render(); }', '相同資料不重繪。');
  // A source observation is not a claim that an actual browser timer ran.
  const poll = sources.index?.match(/setInterval\(\(\)\s*=>\s*void poll\(\),\s*2_?000\)/u);
  checks.push({ id: 'poll_2s_source', passed: Boolean(poll), file: SOURCE_FILES.index,
    line: poll ? sources.index.slice(0, poll.index).split('\n').length : null, meaning: '入口來源設定 2 秒輪詢；本次未操作瀏覽器。' });
  const begin = sources.store?.indexOf('const treeNodes = nodes.map(') ?? -1;
  const end = sources.store?.indexOf('const treeProjection =', begin) ?? -1;
  if (begin >= 0 && end > begin && !sources.store.slice(begin, end).includes('completion_verified')) {
    findings.push({ code: 'ARCHIVED_COMPLETION_FIELD_OMITTED', file: SOURCE_FILES.store,
      line: sources.store.slice(0, begin).split('\n').length,
      detail: 'tree/graph 輸出省略 completion_verified；完成後封存可能只顯示「已封存」。正式完成仍需查 detail Ledger，本次不修改產品。' });
  }
  return { passed: checks.every((c) => c.passed), version: VERSION, checks, findings,
    scope: '保存之已安裝來源靜態查核及純函式離線執行；未直接 SQL 查詢或完整 UI 操作。' };
}

export function validateSnapshot(work, detail, sources) {
  const checks = [], failures = [];
  const check = (code, condition, message) => {
    const row = { code, passed: Boolean(condition), message }; checks.push(row);
    if (!row.passed) failures.push(row);
  };
  const s = work?.work_snapshot, d = detail;
  check('SCHEMA', s?.schema_version === 'lattice.control.work-snapshot.v1'
    && s?.tree?.schema_version === 'lattice.control.work-tree.v1' && s?.graph?.schema_version === 'lattice.control.work-graph.v1'
    && Array.isArray(s?.tree?.nodes) && Array.isArray(s?.tree?.roots) && Array.isArray(s?.graph?.nodes)
    && d?.task?.ledger && Array.isArray(d?.product?.claims) && Array.isArray(d?.product?.metadata)
    && Array.isArray(d?.product?.observations), '必須具有正式工作快照與詳情資料結構。');
  if (failures.length) return { passed: false, checks, failures, observed: null };
  check('FORMAL_WORK_DISABLED', work.formal_work_enabled === true && work.context?.status === 'ready', '正式工作來源啟用且專案已選定。');
  check('SOURCE_AUTHORITY', s.source?.kind === 'POSTGRESQL_CONTROL_PRODUCT' && s.source?.authority === 'POSTGRESQL_TASK_LEDGER', '回應來源為 PostgreSQL 正式 Task Ledger 投影。');
  const projectFields = [work.context?.project_id, s.project_id, d.project_id, d.project?.id, d.task.ledger.project_id, d.metadata?.project_id];
  const metadata = d.product.metadata.find((row) => row.task_ref === TASK);
  projectFields.push(metadata?.project_id);
  check('PROJECT_MISMATCH', projectFields.every((id) => id === PROJECT), '所有專案欄位均須等於本次固定專案。');
  check('TASK_IDENTITY', [d.id, d.task.task_ref, d.task.ledger.task_ref, d.metadata?.task_ref].every((id) => id === TASK), '詳情、Ledger、metadata 使用相同固定 task_ref。');
  check('LEDGER_IDENTITY', HASH.test(d.task.ledger.ledger_head_digest) && d.ledger_head_digest === d.task.ledger.ledger_head_digest,
    '正式 Ledger head digest 與詳情相同；不代表本次重算整條 Ledger。');
  check('SNAPSHOT_IDENTITY', HASH.test(s.revision) && HASH.test(s.digest)
    && [s.tree, s.graph].every((part) => part.revision === s.revision && part.digest === s.digest), 'tree/graph 共用相同 revision 與 digest。');
  const tree = new Map(s.tree.nodes.map((n) => [n.id, n])), graph = new Map(s.graph.nodes.map((n) => [n.id, n]));
  check('DUPLICATE_ID', tree.size === s.tree.nodes.length && graph.size === s.graph.nodes.length, 'tree 與 graph 的工作身份各自唯一。');
  check('NODE_SET', sameSet([...tree.keys()], [...graph.keys()]), 'tree 與 graph 涵蓋同一組工作。');
  const node = tree.get(TASK), graphNode = graph.get(TASK), parent = tree.get(PARENT);
  check('TASK_MISSING', Boolean(node && graphNode), '本工作的固定身份存在於 tree 與 graph。');
  check('PARENT_MISSING', Boolean(parent && graph.has(PARENT)), '指定父工作存在於 tree 與 graph。');
  check('WORK_DESCRIPTION', node?.title === d.title && graphNode?.title === d.title
    && d.objective === d.task.objective && node?.objective === d.task.objective, '真實工作標題及需求在列表、詳情與正式 task 之間一致。');
  check('EXPECTED_PARENT', node?.parent_id === PARENT && d.metadata?.parent_ref === PARENT && metadata?.parent_ref === PARENT, '本工作節點及正式 metadata 皆指向指定父工作。');
  let bidirectional = true, cycle = false;
  for (const n of tree.values()) {
    if (!Array.isArray(n.children) || new Set(n.children).size !== n.children.length) { bidirectional = false; continue; }
    if (n.parent_id != null && tree.get(n.parent_id)?.children?.filter((id) => id === n.id).length !== 1) bidirectional = false;
    for (const id of n.children) if (!tree.has(id) || tree.get(id).parent_id !== n.id) bidirectional = false;
    const seen = new Set(); let cursor = n;
    while (cursor) {
      if (seen.has(cursor.id)) { cycle = true; break; }
      seen.add(cursor.id); cursor = tree.get(cursor.parent_id);
    }
  }
  check('RELATION_BIDIRECTIONAL', bidirectional, '每個可見父子連結雙向一致且不重複。');
  check('RELATION_CYCLE', !cycle, '父子階層沒有循環。');
  check('TREE_ROOTS', sameSet(s.tree.roots, [...tree.values()].filter((n) => n.parent_id == null).map((n) => n.id)), 'roots 與沒有父工作的節點一致。');
  check('CROSS_ENDPOINT_STATUS', node?.status === d.status && graphNode?.status === d.status
    && [...tree.values()].every((n) => graph.get(n.id)?.status === n.status), '列表與詳情狀態相符；若不符需保留非原子讀取／15 秒快取可能性，不能宣稱通過。');
  const formallyCompleted = d.task.ledger.status === 'COMPLETED' && Boolean(d.task.ledger.result_digest);
  const hasCompletionClaim = [d, node, graphNode].some((n) => n?.status === 'verified' || n?.completion_verified === true);
  check('COMPLETION_WITHOUT_RESULT', !hasCompletionClaim || formallyCompleted, '顯示已完成時，必須存在正式 COMPLETED 與 result_digest，AI 文字或回合完成不足以成立。');
  check('COMPLETION_FLAG', d.completion_verified === formallyCompleted, '詳情完成旗標與正式 Ledger 條件一致。');
  check('RESULT_DIGEST', d.result_digest === d.task.ledger.result_digest
    && (d.result_digest == null || HASH.test(d.result_digest)), '正式結果 digest 相同且格式有效；不代表下載／重新執行正式成果。');
  check('OBSERVATIONS_COMPLETE', d.product.truncation?.observations !== true, '本次詳情 observations 未截斷。');
  try {
    const projected = installedProjection(sources, d.task, d.product);
    const fields = ['status', 'completion_verified', 'result_digest', 'ledger_head_digest', 'progress', 'updated_at'];
    check('INSTALLED_STATUS_PROJECTION', fields.every((field) => projected[field] === d[field]), '離線執行保存之安裝版純投影函式，與詳情的狀態、進度及更新時間相符。');
  } catch (error) { check('INSTALLED_STATUS_PROJECTION', false, error.message); }
  let appearance = null;
  try { if (node) appearance = installedAppearance(sources, node); }
  catch (error) { check('INSTALLED_APPEARANCE', false, error.message); }
  return { passed: failures.length === 0, checks, failures, observed: {
    project_id: PROJECT, task_ref: d.id, parent_ref: node?.parent_id, nodes: tree.size,
    ledger_status: d.task.ledger.status, ledger_task_state: d.task.ledger.task_state,
    projected_status: d.status, work_view_label: appearance?.label, formally_completed: formallyCompleted,
    result_digest: d.result_digest, updated_at: d.updated_at,
    source: s.source, snapshot_revision: s.revision, snapshot_digest: s.digest,
  } };
}

export function compareObservations(earlier, later) {
  const before = earlier.detail, after = later.detail;
  return { earlier_at: earlier.manifest.responses.detail.received_at, later_at: later.manifest.responses.detail.received_at,
    identity_stable: before.id === after.id && after.id === TASK && before.project_id === after.project_id && after.project_id === PROJECT,
    forward_in_time: Date.parse(later.manifest.responses.detail.received_at) > Date.parse(earlier.manifest.responses.detail.received_at),
    state_transition_observed: before.status !== after.status, before_status: before.status, after_status: after.status,
    progress_changed: before.progress !== after.progress, updated_at_changed: before.updated_at !== after.updated_at,
    snapshot_revision_changed: earlier.work.work_snapshot.revision !== later.work.work_snapshot.revision,
    note: '兩次唯讀觀測；進度或 revision 更新不能代替正式完成，也未強制觸發狀態轉換。' };
}

export async function main(args = process.argv.slice(2)) {
  const fixture = JSON.parse(await readFile(join(ROOT, 'fixture.json'), 'utf8'));
  let directory;
  if (args.length === 0) {
    directory = join(ROOT, fixture.directory);
  } else if (args.length === 1 && args[0] === '--capture') directory = await capture();
  else if (args.length === 2 && args[0] === '--snapshot') directory = resolve(args[1]);
  else throw new Error('用法：node work-acceptance/acceptance.mjs [--capture | --snapshot 工作目錄內的快照目錄]');
  const rel = relative(ROOT, directory);
  ensure(rel && !rel.startsWith('..') && !isAbsolute(rel), 'SNAPSHOT_OUTSIDE_ACCEPTANCE_DIRECTORY');
  const bundle = await loadSnapshot(directory);
  const validation = validateSnapshot(bundle.work, bundle.detail, bundle.sources);
  const sourceAudit = auditSources(bundle.sources);
  const earlier = await loadSnapshot(join(ROOT, fixture.earlier_directory), { requireAllSources: false });
  const report = { schema_version: 'work-acceptance.report.v1', checked_at: new Date().toISOString(),
    scope: '本固定工作之唯讀工作樹／正式詳情資料與已安裝來源驗收',
    passed: validation.passed && sourceAudit.passed, snapshot_directory: rel.replaceAll('\\', '/'),
    acquisition: bundle.manifest, validation, source_audit: sourceAudit,
    observations_over_time: compareObservations(earlier, bundle),
    not_verified: ['MCP Runtime health：user rejected MCP tool call，未重試。',
      '未直接連線 PostgreSQL、重算整條 Ledger 或讀取 result_digest 對應成果內容；只核對 API 正式來源宣告及安裝來源路徑。',
      '未實際操作瀏覽器／桌面 App，未測量 UI 輪詢延遲、完成後封存或重新開啟。',
      '未寫入 Runtime、未強制狀態轉換、未手動宣告正式完成；本項通過不代表四核心整體完成。'],
    source_findings: sourceAudit.findings,
  };
  const outputDirectory = join(ROOT, 'runs'); await mkdir(outputDirectory, { recursive: true });
  const output = join(outputDirectory, new Date().toISOString().replaceAll(':', '-') + '-' + randomUUID().slice(0, 8) + '.json');
  await saveJson(output, report);
  console.log(JSON.stringify({ passed: report.passed, report: relative(process.cwd(), output).replaceAll('\\', '/'),
    observed: validation.observed, failures: validation.failures, source_check_failures: sourceAudit.checks.filter((c) => !c.passed),
    source_findings: sourceAudit.findings }, null, 2));
  if (!report.passed) process.exitCode = 1;
  return report;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  main().catch((error) => { console.error(JSON.stringify({ passed: false, error: error.message, snapshot_directory: error.snapshot_directory })); process.exitCode = 1; });
}
