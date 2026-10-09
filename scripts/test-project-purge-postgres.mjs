import assert from 'node:assert/strict';
import { createHash, randomBytes, randomUUID } from 'node:crypto';
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const args = Object.fromEntries(process.argv.slice(2).reduce((pairs, token, i, all) => {
  if (i % 2 === 0) pairs.push([token.replace(/^--/, ''), all[i + 1]]);
  return pairs;
}, []));
const port = Number(args.port), runRoot = path.resolve(args['run-root']);
assert.ok(['main', 'interleaved', 'coordinator'].includes(args.scenario));
const marker = JSON.parse(readFileSync(path.join(runRoot, 'fixture-owner.json'), 'utf8'));
assert.equal(marker.kind, 'LATTICE_PROJECT_PURGE_SYNTHETIC_FIXTURE');
assert.equal(marker.port, port);
assert.ok(Number.isInteger(port) && port > 0 && port < 65536 && ![4317, 5432, 55432, 58743, 64272].includes(port));
assert.equal(path.dirname(path.resolve(marker.cluster)), runRoot);
assert.ok(path.basename(runRoot).match(/^[0-9a-f]{32}$/));
const password = randomBytes(24).toString('hex');
const baseEnv = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(LATTICE_|PG|DATABASE_URL$)/i.test(key)));
const evidence = { schema: 'lattice.project-purge-live-scenarios.v1', status: 'RUNNING', port, checks: [], databases: [] };
evidence.binaries = Object.fromEntries(['binary', 'seed-binary', 'runtime-binary'].map(key => [key, { path: path.resolve(args[key]), sha256: createHash('sha256').update(readFileSync(args[key])).digest('hex') }]));
const redact = value => String(value ?? '').replaceAll(password, '[FIXTURE_PASSWORD]');
let sequence = 0;
const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
function command(executable, argv, env, input, label, expectedSuccess = true) {
  const result = spawnSync(executable, argv, { env, input, encoding: 'utf8', windowsHide: true, timeout: 180000, maxBuffer: 8 * 1024 * 1024 });
  const record = { label, exitCode: result.status, error: result.error?.code ?? null, stdout: redact(result.stdout), stderr: redact(result.stderr) };
  writeFileSync(path.join(runRoot, `${String(++sequence).padStart(2, '0')}-${label}.json`), JSON.stringify(record, null, 2) + '\n');
  if (expectedSuccess) assert.equal(result.status, 0, `${label}: ${record.stderr || record.error}`);
  return record;
}
function check(name, detail = {}) { evidence.checks.push({ name, passed: true, ...detail }); }
function sql(env, statement) {
  return command(args.psql, ['-X', '-A', '-t', '-h', '127.0.0.1', '-p', String(port), '-U', 'runtime_bootstrap', '-d', `lattice_task019_${env.LATTICE_TASK019_RUN_ID.slice(0, 8)}_base`, '-v', 'ON_ERROR_STOP=1', '-c', statement], env, undefined, 'sql').stdout.trim();
}
function native(env, request, expectedSuccess = true) {
  const result = command(args.binary, [], env, JSON.stringify({ schema: 'lattice.project-purge.request.v1', ...request }), `native-${request.action}`, expectedSuccess);
  return { ...result, value: result.exitCode === 0 ? JSON.parse(result.stdout) : null };
}
function rows(env) {
  const tables = JSON.parse(sql(env, `SELECT coalesce(json_agg(json_build_array(n.nspname,c.relname) ORDER BY n.nspname,c.relname),'[]') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='r' AND n.nspname NOT LIKE 'pg_%' AND n.nspname NOT IN ('information_schema','project_purge')`));
  return tables.map(([schema, table]) => {
    assert.match(schema, /^[a-z_][a-z0-9_]*$/); assert.match(table, /^[a-z_][a-z0-9_]*$/);
    return [`${schema}.${table}`, JSON.parse(sql(env, `SELECT coalesce(json_agg(row_value ORDER BY row_value::text COLLATE "C"),'[]') FROM (SELECT to_jsonb(p) row_value FROM ONLY "${schema}"."${table}" p) r`))];
  });
}
function selectRows(snapshot, needles) {
  const matches = value => typeof value === 'string' ? needles.some(needle => value.includes(needle))
    : value !== null && typeof value === 'object' && Object.entries(value).some(([key, child]) => matches(key) || matches(child));
  return snapshot.map(([table, values]) => [table, values.filter(matches)]).filter(([, values]) => values.length);
}
function establish(order) {
  const runId = randomUUID().replaceAll('-', '');
  const root = path.join(runRoot, order);
  for (const name of ['project-a', 'project-b']) {
    mkdirSync(path.join(root, name), { recursive: true });
    writeFileSync(path.join(root, name, 'synthetic.txt'), `Synthetic ${name}; fixture ${runId}\n`);
  }
  writeFileSync(path.join(root, 'project-purge-fixture.marker'), runId + '\n');
  const env = { ...baseEnv, LATTICE_TASK019_HOST: '127.0.0.1', LATTICE_TASK019_PORT: String(port), LATTICE_TASK019_RUN_ID: runId, LATTICE_TASK019_PASSWORD: password,
    LATTICE_RUNTIME_INTEGRATION: 'CORE_ONLY', LATTICE_FULL_CHAIN_RUN_MODE: 'RESUME_EXISTING', LATTICE_DELIVERY_CODEX_MODE: 'OFFICIAL_CODEX_APP_SERVER', LATTICE_MANAGED_FOREMAN_MODE: 'DISABLED',
    LATTICE_STORE_DAEMON_INSTANCE_ID: 'task050-fresh-process', LATTICE_STORE_DAEMON_EPOCH: '50', LATTICE_STORE_AUTHORITY_REVISION: '50', LATTICE_STORE_OBSERVATION_DIGEST: 'a'.repeat(64), LATTICE_STORE_AUTHORITY_HEAD_DIGEST: 'b'.repeat(64),
    LATTICE_PURGE_FIXTURE_ROOT: root, LATTICE_PURGE_FIXTURE_ONLY: '1' };
  command(args['runtime-binary'], ['--postgres-initialize'], env, undefined, `${order}-initialize`);
  command(args['runtime-binary'], ['--postgres-bootstrap'], env, undefined, `${order}-bootstrap`);
  // Product bootstrap establishes the requested synthetic authority. This newly
  // created fixture has no running executor; enter its maintenance state before
  // installing the extension. The production purge itself never does this.
  assert.equal(sql(env, 'SELECT count(*) FROM control.project_registry_commands'), '0');
  const stopped = sql(env, "UPDATE control.runtime_admission SET admission_mode='STOPPED',daemon_instance_id=NULL,daemon_epoch=NULL,authority_revision=0,observation_digest=NULL,authority_head_digest=NULL WHERE singleton AND admission_mode='ACTIVE' AND daemon_instance_id='task050-fresh-process' AND daemon_epoch=50 AND authority_revision=50 AND observation_digest=decode(repeat('a',64),'hex') AND authority_head_digest=decode(repeat('b',64),'hex') RETURNING admission_mode");
  assert.match(stopped, /^STOPPED\r?\nUPDATE 1$/);
  const seeded = JSON.parse(command(args['seed-binary'], order === 'target-first' ? ['target-first'] : [], env, undefined, `${order}-seed`).stdout);
  seeded.targetTaskRef = seeded.records.find(x => x.projectId === seeded.targetProjectId).taskRef;
  seeded.survivorTaskRef = seeded.records.find(x => x.projectId === seeded.survivorProjectId).taskRef;
  seeded.targetCanonicalPath = path.join(root, 'project-a');
  seeded.survivorCanonicalPath = path.join(root, 'project-b');
  for (const kind of ['target', 'survivor']) {
    const id = seeded[`${kind}ProjectId`]; assert.match(id, /^[a-zA-Z0-9_.:-]+$/);
    seeded[`${kind}StreamId`] = sql(env, `SELECT encode(stream_id,'hex') FROM control.task_ledger_streams WHERE project_id='${id}'`);
    assert.match(seeded[`${kind}StreamId`], /^[0-9a-f]{64}$/);
  }
  evidence.databases.push({ runId, order, seeded });
  assert.equal(native(env, { action: 'install', authorization: 'INSTALL_PURGE_MAINTENANCE' }).value.status, 'INSTALLED');
  return { env, seeded, root };
}

try {
  if (args.scenario === 'main') {
  const { env, seeded } = establish('survivor-first');
  assert.equal(typeof seeded.targetProjectId, 'string');
  assert.equal(typeof seeded.survivorProjectId, 'string');
  const target = seeded.targetProjectId, survivor = seeded.survivorProjectId;
  const replayBefore = JSON.parse(command(args['seed-binary'], ['verify-survivor'], env, undefined, 'survivor-replay-before').stdout);
  const operationId = 'fixture-main';
  const request = { projectId: target, operationId };
  const initial = rows(env);
  const beforePreview = hash(initial);
  const preview = native(env, { action: 'preview', ...request }).value;
  assert.equal(preview.status, 'READY', JSON.stringify(preview.blockers));
  assert.equal(hash(rows(env)), beforePreview);
  check('preview-is-read-only');
  assert.ok(preview.counts['control.task_ledger_streams'] >= 1);
  check('target-has-real-task-ledger-data');

  assert.match(target, /^[a-zA-Z0-9_.:-]+$/); assert.match(survivor, /^[a-zA-Z0-9_.:-]+$/);
  assert.match(seeded.targetTaskRef, /^[a-f0-9]{64}$/); assert.match(seeded.survivorTaskRef, /^[a-f0-9]{64}$/);
  sql(env, `INSERT INTO control_product.work_metadata(task_ref,project_id,title,success_criteria,priority,revision,request_id,request_digest) VALUES('${seeded.targetTaskRef}','${target}','Changed synthetic title','Synthetic fixture',1,1,'fixture-metadata-target',repeat('a',64)),('${seeded.survivorTaskRef}','${survivor}','Survivor synthetic title','Synthetic fixture',1,1,'fixture-metadata-survivor',repeat('b',64))`);
  const beforeStale = hash(rows(env));
  const stale = native(env, { action: 'apply', ...request, expectedScopeDigest: preview.scopeDigest, authorization: 'ERASE_PROJECT_DATA' }, false);
  assert.notEqual(stale.exitCode, 0); assert.match(stale.stderr, /PROJECT_PURGE_STALE_SCOPE/);
  assert.equal(hash(rows(env)), beforeStale);
  check('stale-preview-rejected-with-no-mutation');

  sql(env, `UPDATE control_product.work_metadata SET title='Reference to ${target}' WHERE project_id='${survivor}'`);
  const beforeCrossScope = hash(rows(env));
  const crossPreview = native(env, { action: 'preview', ...request }).value;
  assert.equal(crossPreview.status, 'BLOCKED');
  assert.ok(crossPreview.blockers.some(x => x.code === 'CROSS_SCOPE_OR_UNSUPPORTED_REFERENCE'));
  const crossApply = native(env, { action: 'apply', ...request, expectedScopeDigest: crossPreview.scopeDigest, authorization: 'ERASE_PROJECT_DATA' }).value;
  assert.equal(crossApply.status, 'BLOCKED');
  assert.equal(hash(rows(env)), beforeCrossScope);
  check('cross-scope-reference-blocks-preview-and-apply');
  sql(env, `UPDATE control_product.work_metadata SET title='Survivor synthetic title' WHERE project_id='${survivor}'`);

  const targetRootSql = seeded.targetCanonicalPath.replaceAll("'", "''");
  sql(env, `UPDATE control_product.work_metadata SET title='${targetRootSql}' WHERE project_id='${survivor}'`);
  const pathReferenceBefore = hash(rows(env));
  const pathReference = native(env, { action: 'preview', ...request }).value;
  assert.equal(pathReference.status, 'BLOCKED');
  assert.ok(pathReference.blockers.some(x => x.code === 'CROSS_SCOPE_OR_UNSUPPORTED_REFERENCE'));
  assert.equal(native(env, { action: 'apply', ...request, expectedScopeDigest: pathReference.scopeDigest, authorization: 'ERASE_PROJECT_DATA' }).value.status, 'BLOCKED');
  assert.equal(hash(rows(env)), pathReferenceBefore);
  check('windows-canonical-path-reference-fails-closed');
  sql(env, `UPDATE control_product.work_metadata SET title='Survivor synthetic title' WHERE project_id='${survivor}'`);

  sql(env, `INSERT INTO control_product.conversation_claims(claim_id,task_ref,project_id,phase,request_digest,prompt,model,worktree_path) VALUES('fixture-shared-root','${seeded.survivorTaskRef}','${survivor}','EXECUTION',repeat('c',64),'Synthetic shared-root probe','fixture','${targetRootSql}')`);
  const sharedBefore = hash(rows(env));
  const shared = native(env, { action: 'preview', ...request }).value;
  assert.equal(shared.status, 'BLOCKED');
  assert.ok(shared.blockers.some(x => x.code === 'SHARED_OR_OVERLAPPING_FILESYSTEM_ROOT'));
  assert.equal(native(env, { action: 'apply', ...request, expectedScopeDigest: shared.scopeDigest, authorization: 'ERASE_PROJECT_DATA' }).value.status, 'BLOCKED');
  assert.equal(hash(rows(env)), sharedBefore);
  check('another-project-shared-root-blocks-erasure');
  sql(env, "DELETE FROM control_product.conversation_claims WHERE claim_id='fixture-shared-root'");

  const reviewed = native(env, { action: 'preview', ...request }).value;
  assert.equal(reviewed.status, 'READY');
  const beforeApply = rows(env);
  const survivorNeedles = [survivor, seeded.survivorStreamId, seeded.survivorTaskRef, seeded.survivorCanonicalPath].filter(Boolean);
  const targetNeedles = [target, seeded.targetStreamId, seeded.targetTaskRef, seeded.targetCanonicalPath].filter(Boolean);
  const survivorRows = selectRows(beforeApply, survivorNeedles);
  assert.ok(survivorRows.length > 0);
  const survivorBefore = hash(survivorRows);
  const applyRequest = { action: 'apply', ...request, expectedScopeDigest: reviewed.scopeDigest, authorization: 'ERASE_PROJECT_DATA' };
  const applied = native(env, applyRequest).value;
  assert.equal(applied.status, 'PURGED');
  const afterApply = rows(env);
  assert.equal(selectRows(afterApply, targetNeedles).length, 0);
  assert.equal(hash(selectRows(afterApply, survivorNeedles)), survivorBefore);
  check('target-identities-erased-and-survivor-rows-byte-equivalent', { survivorBefore, survivorAfter: hash(selectRows(afterApply, survivorNeedles)) });
  const replayAfter = JSON.parse(command(args['seed-binary'], ['verify-survivor'], env, undefined, 'survivor-replay-after').stdout);
  assert.equal(replayAfter.status, 'VERIFIED');
  assert.equal(replayAfter.survivorTaskRef, replayBefore.survivorTaskRef);
  assert.equal(replayAfter.survivorLedgerHead, replayBefore.survivorLedgerHead);
  check('fresh-process-registry-and-survivor-ledger-replay', { replayBefore, replayAfter });
  assert.deepEqual(native(env, { action: 'status', operationId, projectId: target }).value, applied);
  assert.deepEqual(native(env, applyRequest).value, applied);
  assert.equal(hash(rows(env)), hash(afterApply));
  check('committed-receipt-readback-and-idempotent-retry');
  const mismatch = native(env, { ...applyRequest, projectId: survivor }, false);
  assert.notEqual(mismatch.exitCode, 0); assert.match(mismatch.stderr, /PROJECT_PURGE_IDEMPOTENCY_CONFLICT/);
  assert.equal(hash(rows(env)), hash(afterApply));
  check('operation-cannot-be-rebound-to-another-project');
  sql(env, `UPDATE control_product.work_metadata SET title='Changed after erasure' WHERE project_id='${survivor}'`);
  const readbackDrift = hash(rows(env));
  for (const req of [{ action: 'status', operationId, projectId: target }, applyRequest]) {
    const changed = native(env, req, false);
    assert.notEqual(changed.exitCode, 0); assert.match(changed.stderr, /PROJECT_PURGE_READBACK_CHANGED/);
  }
  assert.equal(hash(rows(env)), readbackDrift);
  check('postcommit-drift-invalidates-receipt-and-retry');
  }

  if (args.scenario === 'interleaved') {
  const interleaved = establish('target-first');
  const interleavedRequest = { projectId: interleaved.seeded.targetProjectId, operationId: 'fixture-interleaved' };
  const interleavedBaseline = native(interleaved.env, { action: 'preview', ...interleavedRequest }).value;
  assert.equal(interleavedBaseline.status, 'BLOCKED');
  assert.ok(interleavedBaseline.blockers.some(x => x.code === 'REGISTRY_INTERLEAVED_HISTORY_REQUIRES_MIGRATION'));
  sql(interleaved.env, 'ALTER TABLE project_purge.receipts ADD CONSTRAINT fixture_extra_check CHECK(true) NOT VALID');
  const malformedProfile = native(interleaved.env, { action: 'preview', ...interleavedRequest }, false);
  assert.notEqual(malformedProfile.exitCode, 0);
  assert.match(malformedProfile.stderr, /PROJECT_PURGE_MAINTENANCE_PROFILE_REQUIRED/);
  assert.equal(sql(interleaved.env, 'SELECT count(*) FROM control.project_registry_projects'), '2');
  check('extra-not-valid-constraint-rejected-by-exact-profile');
  sql(interleaved.env, 'ALTER TABLE project_purge.receipts DROP CONSTRAINT fixture_extra_check');
  for (const role of ['lattice_runtime', 'PUBLIC']) {
    sql(interleaved.env, `GRANT SELECT ON project_purge.receipts TO ${role}`);
    const expandedGrant = native(interleaved.env, { action: 'preview', ...interleavedRequest }, false);
    assert.notEqual(expandedGrant.exitCode, 0);
    assert.match(expandedGrant.stderr, /PROJECT_PURGE_MAINTENANCE_PROFILE_REQUIRED/);
    sql(interleaved.env, `REVOKE SELECT ON project_purge.receipts FROM ${role}`);
    assert.equal(native(interleaved.env, { action: 'preview', ...interleavedRequest }).exitCode, 0);
    check(`receipt-select-grant-to-${role}-rejected-and-revocation-recovers-profile`);
  }
  const interleavedBefore = hash(rows(interleaved.env));
  const blocked = native(interleaved.env, { action: 'preview', ...interleavedRequest }).value;
  assert.equal(blocked.status, 'BLOCKED');
  assert.ok(blocked.blockers.some(x => x.code === 'REGISTRY_INTERLEAVED_HISTORY_REQUIRES_MIGRATION'));
  assert.equal(native(interleaved.env, { action: 'apply', ...interleavedRequest, expectedScopeDigest: blocked.scopeDigest, authorization: 'ERASE_PROJECT_DATA' }).value.status, 'BLOCKED');
  assert.equal(hash(rows(interleaved.env)), interleavedBefore);
  check('interleaved-history-fails-closed-with-no-mutation');
  }

  if (args.scenario === 'coordinator') {
  const coordinated = establish('coordinator');
  const { LatticeStore } = await import('../apps/lattice-control/src/store.mjs');
  const { previewProjectPurge, applyProjectPurge, statusProjectPurge } = await import('../apps/lattice-control/src/project-purge.mjs');
  const sqlitePath = path.join(coordinated.root, 'control-fixture.sqlite');
  const localStore = new LatticeStore(sqlitePath);
  try {
    for (const kind of ['target', 'survivor']) {
      const id = coordinated.seeded[`${kind}ProjectId`], root = coordinated.seeded[`${kind}CanonicalPath`];
      localStore.database.prepare('INSERT INTO projects(id,name,root_path,created_at,updated_at) VALUES(?,?,?,?,?)').run(id, `Synthetic ${kind}`, root, '2026-10-09T00:00:00Z', '2026-10-09T00:00:00Z');
    }
  } finally { localStore.close(); }
  // These values exist only in this isolated harness process; the product native
  // subprocess receives the same owned fixture connection as the preceding tests.
  for (const [key, value] of Object.entries(coordinated.env)) if (key.startsWith('LATTICE_')) process.env[key] = value;
  const survivorFile = path.join(coordinated.seeded.survivorCanonicalPath, 'synthetic.txt');
  const survivorFileBefore = hash(readFileSync(survivorFile));
  const coordinatorBefore = rows(coordinated.env);
  const coordinatorSurvivors = [coordinated.seeded.survivorProjectId, coordinated.seeded.survivorStreamId, coordinated.seeded.survivorTaskRef];
  const survivorPgBefore = hash(selectRows(coordinatorBefore, coordinatorSurvivors));
  const plan = await previewProjectPurge({ projectId: coordinated.seeded.targetProjectId, nativeBinary: path.resolve(args.binary), databasePath: sqlitePath, statePath: path.join(coordinated.root, 'purge-state.json'), operationId: 'fixture-coordinator' });
  assert.equal(plan.status, 'READY', JSON.stringify(plan.blockers));
  writeFileSync(path.join(runRoot, 'coordinator-plan.json'), JSON.stringify(plan, null, 2) + '\n');
  const appliedCoordinator = await applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true });
  assert.equal(appliedCoordinator.status, 'SCOPED_PURGED');
  assert.equal(existsSync(coordinated.seeded.targetCanonicalPath), false);
  assert.equal(hash(readFileSync(survivorFile)), survivorFileBefore);
  assert.equal(hash(selectRows(rows(coordinated.env), coordinatorSurvivors)), survivorPgBefore);
  const verifiedCoordinator = await statusProjectPurge(plan);
  assert.equal(verifiedCoordinator.status, 'SCOPED_PURGED');
  assert.equal(verifiedCoordinator.externalCleanup, 'NOT_VERIFIED');
  writeFileSync(path.join(runRoot, 'coordinator-result.json'), JSON.stringify(verifiedCoordinator, null, 2) + '\n');
  check('real-coordinator-pg-sqlite-files-readback-and-survivor-preserved', { survivorPgBefore, survivorFileBefore });
  }
  evidence.status = 'PASS';
} catch (error) {
  evidence.status = 'FAIL'; evidence.error = redact(error.stack); process.exitCode = 1;
} finally {
  evidence.finishedAt = new Date().toISOString();
  writeFileSync(path.join(runRoot, 'scenario-results.json'), JSON.stringify(evidence, null, 2) + '\n');
  console.log(JSON.stringify({ status: evidence.status, checks: evidence.checks, evidence: path.join(runRoot, 'scenario-results.json'), error: evidence.error }));
}
