import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { ROOT, PROJECT, TASK, PARENT } from './capture.mjs';
import { loadSnapshot, validateSnapshot, auditSources, installedProjection, installedAppearance, compareObservations, verifyCapturedBytes } from './acceptance.mjs';

const fixture = JSON.parse(await readFile(join(ROOT, 'fixture.json'), 'utf8'));
const baseline = await loadSnapshot(join(ROOT, fixture.directory));
const clone = () => structuredClone(baseline);
const target = (bundle, kind = 'tree') => bundle.work.work_snapshot[kind].nodes.find((n) => n.id === TASK);
const parent = (bundle) => bundle.work.work_snapshot.tree.nodes.find((n) => n.id === PARENT);
const validate = (bundle) => validateSnapshot(bundle.work, bundle.detail, bundle.sources);
const rejects = (mutate, code) => {
  const bundle = clone(); mutate(bundle);
  const result = validate(bundle);
  assert.equal(result.passed, false, JSON.stringify(result));
  assert.ok(result.failures.some((f) => f.code === code), JSON.stringify(result.failures));
};

// Alternate states derive from the same real snapshot and run the saved installed
// pure projection. They are TEST VARIANTS, never claims about live completion.
function stateVariant(state) {
  const bundle = clone(), d = bundle.detail;
  d.task.ledger.status = 'SUBMITTED'; d.task.ledger.task_state = 'DRAFT'; d.task.ledger.result_digest = null;
  d.product.observations = [];
  d.product.claims = d.product.claims.filter((c) => c.task_ref === TASK && c.phase === 'EXECUTION').slice(0, 1);
  const c = d.product.claims[0];
  assert.ok(c, '真實快照必須具有本工作的 execution claim');
  c.archived = false; c.pending_questions = []; c.pending_questions_count = 0;
  c.turn_status = 'TURN_BOUND'; c.turn_id = 'offline-test-turn';
  if (state === 'starting') { c.turn_status = 'DISPATCH_STARTED'; c.turn_id = null; }
  if (state === 'codex_done' || state === 'verified' || state.startsWith('archived')) c.turn_status = 'TURN_COMPLETED';
  if (state === 'failed') c.turn_status = 'TURN_FAILED';
  if (state === 'waiting_approval') c.pending_questions_count = 1, delete c.pending_questions;
  if (state === 'draft') d.product.claims = [];
  if (state === 'verified' || state === 'archived_complete') {
    d.task.ledger.status = 'COMPLETED'; d.task.ledger.task_state = 'COMPLETED';
    d.task.ledger.result_digest = 'a'.repeat(64); // clearly synthetic formal-result reference
  }
  if (state.startsWith('archived')) c.archived = true;
  Object.assign(d, installedProjection(bundle.sources, d.task, d.product));
  for (const kind of ['tree', 'graph']) {
    const node = target(bundle, kind); node.status = d.status;
    delete node.completion_verified; // installed tree/graph currently omit this field
    if (kind === 'tree') node.progress = d.progress.trim();
  }
  return bundle;
}

test('保存的原始 HTTP 快照通過來源、固定身份、父子及正式完成核對', () => {
  const result = validate(baseline);
  assert.equal(result.passed, true, JSON.stringify(result.failures));
  assert.equal(result.observed.task_ref, TASK);
  assert.equal(result.observed.parent_ref, PARENT);
  assert.equal(result.observed.formally_completed, baseline.detail.task.ledger.status === 'COMPLETED' && Boolean(baseline.detail.task.ledger.result_digest));
});

test('安裝來源查核及封存欄位限制均有記錄', () => {
  const result = auditSources(baseline.sources);
  assert.equal(result.passed, true, JSON.stringify(result));
  assert.ok(result.findings.some((x) => x.code === 'ARCHIVED_COMPLETION_FIELD_OMITTED'));
});

for (const [state, label] of [['draft', '待開始'], ['starting', '進行中'], ['running', '進行中'],
  ['waiting_approval', '等你決定'], ['failed', '執行失敗'], ['codex_done', '等待驗收'], ['verified', '已完成'],
  ['archived_incomplete', '已封存'], ['archived_complete', '已封存']]) {
  test(`真實快照狀態變體 ${state} 合法，列表來源顯示 ${label}`, () => {
    const bundle = stateVariant(state), result = validate(bundle);
    assert.equal(result.passed, true, JSON.stringify(result.failures));
    assert.equal(installedAppearance(bundle.sources, target(bundle)).label, label);
    assert.equal(result.observed.formally_completed, ['verified', 'archived_complete'].includes(state));
  });
}

test('AI 回覆「已完成」及回合結束仍是等待驗收，不能代替正式成果', () => {
  const bundle = stateVariant('codex_done');
  bundle.detail.product.observations.push({ claim_id: bundle.detail.product.claims[0].claim_id,
    observed_at: '2026-09-06T06:00:00Z', kind: 'PROGRESS', summary: '全部已完成，驗收通過。' });
  Object.assign(bundle.detail, installedProjection(bundle.sources, bundle.detail.task, bundle.detail.product));
  assert.equal(validate(bundle).passed, true);
  assert.equal(validate(bundle).observed.formally_completed, false);
  assert.equal(bundle.detail.status, 'codex_done');
});

for (const [name, mutate] of [
  ['context', (b) => b.work.context.project_id = 'wrong-project'],
  ['snapshot', (b) => b.work.work_snapshot.project_id = 'wrong-project'],
  ['detail', (b) => b.detail.project_id = 'wrong-project'],
  ['ledger', (b) => b.detail.task.ledger.project_id = 'wrong-project'],
  ['project registry', (b) => b.detail.project.id = 'wrong-project'],
  ['metadata', (b) => b.detail.metadata.project_id = 'wrong-project'],
]) test(`拒絕錯誤專案：${name}`, () => rejects(mutate, 'PROJECT_MISMATCH'));

test('拒絕非 PostgreSQL 正式來源', () => rejects((b) => b.work.work_snapshot.source.authority = 'AI_REPLY', 'SOURCE_AUTHORITY'));
test('拒絕停用正式工作來源', () => rejects((b) => b.work.formal_work_enabled = false, 'FORMAL_WORK_DISABLED'));
test('拒絕被替換的工作固定身份', () => rejects((b) => b.detail.task.ledger.task_ref = 'b'.repeat(64), 'TASK_IDENTITY'));
test('拒絕被示意工作標題替換的節點', () => rejects((b) => target(b).title = '示意工作', 'WORK_DESCRIPTION'));
test('拒絕遺失父工作', () => rejects((b) => b.work.work_snapshot.tree.nodes = b.work.work_snapshot.tree.nodes.filter((n) => n.id !== PARENT), 'PARENT_MISSING'));
test('拒絕父工作反向 children 遺失本工作', () => rejects((b) => parent(b).children = [], 'RELATION_BIDIRECTIONAL'));
test('拒絕子工作的 parent_id 不一致', () => rejects((b) => target(b).parent_id = null, 'EXPECTED_PARENT'));
test('拒絕正式 metadata 父身份不一致', () => rejects((b) => b.detail.metadata.parent_ref = null, 'EXPECTED_PARENT'));
test('拒絕重複父子連結', () => rejects((b) => parent(b).children.push(TASK), 'RELATION_BIDIRECTIONAL'));
test('拒絕重複工作身份', () => rejects((b) => b.work.work_snapshot.tree.nodes.push(structuredClone(target(b))), 'DUPLICATE_ID'));
test('拒絕循環父子關係', () => rejects((b) => { parent(b).parent_id = TASK; target(b).children.push(PARENT); }, 'RELATION_CYCLE'));
test('拒絕 tree/graph revision 不一致', () => rejects((b) => b.work.work_snapshot.graph.revision = 'b'.repeat(64), 'SNAPSHOT_IDENTITY'));
test('拒絕跨端點狀態不一致，保留非原子快照限制', () => rejects((b) => {
  target(b, 'graph').status = b.detail.status === 'running' ? 'codex_done' : 'running';
}, 'CROSS_ENDPOINT_STATUS'));
test('拒絕缺少 result_digest 卻宣稱正式完成', () => {
  const bundle = stateVariant('verified'); bundle.detail.task.ledger.result_digest = null;
  const result = validate(bundle);
  assert.equal(result.passed, false);
  assert.ok(result.failures.some((x) => x.code === 'COMPLETION_WITHOUT_RESULT'));
});
test('拒絕未有 COMPLETED ledger 卻宣稱完成', () => {
  const bundle = stateVariant('verified'); bundle.detail.task.ledger.status = 'SUBMITTED';
  assert.ok(validate(bundle).failures.some((x) => x.code === 'COMPLETION_WITHOUT_RESULT'));
});
test('拒絕只有列表 status=verified 的錯誤完成顯示', () => {
  const bundle = stateVariant('codex_done'); target(bundle).status = 'verified';
  assert.ok(validate(bundle).failures.some((x) => x.code === 'COMPLETION_WITHOUT_RESULT'));
});
test('拒絕只有 completion_verified 旗標的錯誤完成宣稱', () => {
  const bundle = stateVariant('codex_done'); bundle.detail.completion_verified = true;
  assert.ok(validate(bundle).failures.some((x) => x.code === 'COMPLETION_WITHOUT_RESULT'));
});
test('拒絕 malformed JSON shape，回傳可核對失敗而非誤通過', () => {
  assert.equal(validateSnapshot({}, {}, baseline.sources).passed, false);
});
test('來源完成條件遭弱化時查核失敗', () => {
  const sources = structuredClone(baseline.sources);
  sources.formal = sources.formal.replace('task.ledger.status === "COMPLETED" && Boolean(task.ledger.result_digest)', 'true');
  assert.equal(auditSources(sources).passed, false);
});
test('兩份真實快照的狀態更新比較只報觀測事實', async () => {
  const earlier = await loadSnapshot(join(ROOT, 'snapshots/2026-09-06T04-54-32.037Z-25b57920'), { requireAllSources: false });
  const result = compareObservations(earlier, baseline);
  assert.equal(result.identity_stable, true);
  assert.equal(result.forward_in_time, true);
  assert.equal(typeof result.progress_changed, 'boolean');
  assert.equal(result.state_transition_observed, earlier.detail.status !== baseline.detail.status);
});
test('已保存快照與測試未改動 Runtime，也不依賴即時網路', () => {
  assert.equal(baseline.manifest.boundaries.runtime_writes, false);
  assert.equal(baseline.manifest.project_id, PROJECT);
  for (const response of Object.values(baseline.manifest.responses)) assert.equal(response.method, 'GET');
});
test('驗收函式不改動輸入的真實快照', () => {
  const bundle = clone(), before = JSON.stringify(bundle);
  validate(bundle); auditSources(bundle.sources);
  assert.equal(JSON.stringify(bundle), before);
});
test('原始回應被修改或截斷時，雜湊核對拒絕接受', async () => {
  const record = baseline.manifest.responses.work;
  const bytes = await readFile(join(baseline.directory, record.file));
  assert.doesNotThrow(() => verifyCapturedBytes(record, bytes));
  const changed = Buffer.from(bytes); changed[10] ^= 1;
  assert.throws(() => verifyCapturedBytes(record, changed), /CAPTURE_HASH/u);
  assert.throws(() => verifyCapturedBytes(record, bytes.subarray(1)), /CAPTURE_HASH/u);
});
