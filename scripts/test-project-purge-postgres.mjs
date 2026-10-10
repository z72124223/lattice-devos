import assert from 'node:assert/strict';
import { createHash, randomBytes, randomUUID } from 'node:crypto';
import { readFileSync, writeFileSync, mkdirSync, existsSync, renameSync } from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const args = Object.fromEntries(process.argv.slice(2).reduce((pairs, token, i, all) => {
  if (i % 2 === 0) pairs.push([token.replace(/^--/, ''), all[i + 1]]);
  return pairs;
}, []));
const port = Number(args.port), runRoot = path.resolve(args['run-root']);
assert.ok(['main', 'interleaved', 'survivor-reference', 'epoch', 'epoch-reference', 'coordinator', 'coordinator-absent', 'inventory', 'bot-inventory', 'bot-purge', 'graph-ownership', 'upgrade', 'streaming', 'streaming-large'].includes(args.scenario));
if (['upgrade','streaming','streaming-large'].includes(args.scenario)) assert.equal(typeof args['legacy-binary'], 'string');
const marker = JSON.parse(readFileSync(path.join(runRoot, 'fixture-owner.json'), 'utf8'));
assert.equal(marker.kind, 'LATTICE_PROJECT_PURGE_SYNTHETIC_FIXTURE');
assert.equal(marker.port, port);
assert.ok(Number.isInteger(port) && port > 0 && port < 65536 && ![4317, 5432, 55432, 58743, 64272].includes(port));
assert.equal(path.dirname(path.resolve(marker.cluster)), runRoot);
assert.ok(path.basename(runRoot).match(/^[0-9a-f]{32}$/));
const password = randomBytes(24).toString('hex');
const baseEnv = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(LATTICE_|PG|DATABASE_URL$)/i.test(key)));
const evidence = { schema: 'lattice.project-purge-live-scenarios.v1', status: 'RUNNING', port, checks: [], databases: [] };
evidence.binaries = Object.fromEntries(['binary', 'seed-binary', 'runtime-binary', ...(args['legacy-binary'] ? ['legacy-binary'] : []), ...(args['epoch-binary'] ? ['epoch-binary'] : []), ...(args['lifecycle-binary'] ? ['lifecycle-binary'] : [])].map(key => [key, { path: path.resolve(args[key]), sha256: createHash('sha256').update(readFileSync(args[key])).digest('hex') }]));
const redact = value => String(value ?? '').replaceAll(password, '[FIXTURE_PASSWORD]');
let sequence = 0;
const hash = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
const sourceNames = ['project-client.mjs', 'project-purge-client.mjs', 'project-purge.mjs', 'project-purge-report.mjs', 'project-purge-finalize.mjs', 'project-purge-files.mjs', 'project-purge-sqlite.mjs', 'project-purge-sqlite-swap.mjs', 'project-purge-code-graph.mjs', 'code-graph.mjs', 'code-graph-model.mjs', 'lattice-runtime-health.mjs', 'store.mjs', 'database-path.mjs', 'decision-project-ownership.mjs'];
const sourceHashes = () => Object.fromEntries(sourceNames.map(name => {
  const file = fileURLToPath(new URL(`../apps/lattice-control/src/${name}`, import.meta.url));
  return [name, createHash('sha256').update(readFileSync(file)).digest('hex')];
}));
evidence.sourceHashes = sourceHashes();
function command(executable, argv, env, input, label, expectedSuccess = true) {
  const startedAt = performance.now();
  const result = spawnSync(executable, argv, { env, input, encoding: 'utf8', windowsHide: true, timeout: 180000, maxBuffer: 8 * 1024 * 1024 });
  const record = { label, elapsedMs: Math.round(performance.now()-startedAt), exitCode: result.status, error: result.error?.code ?? null, stdout: redact(result.stdout), stderr: redact(result.stderr) };
  writeFileSync(path.join(runRoot, `${String(++sequence).padStart(2, '0')}-${label}.json`), JSON.stringify(record, null, 2) + '\n');
  if (expectedSuccess) assert.equal(result.status, 0, `${label}: ${record.stderr || record.error}`);
  return record;
}
function check(name, detail = {}) { evidence.checks.push({ name, passed: true, ...detail }); }
function sql(env, statement, expectedSuccess = true) {
  const result=command(args.psql, ['-X', '-A', '-t', '-h', '127.0.0.1', '-p', String(port), '-U', 'runtime_bootstrap', '-d', `lattice_task019_${env.LATTICE_TASK019_RUN_ID.slice(0, 8)}_base`, '-v', 'ON_ERROR_STOP=1', '-f', '-'], { ...env, PGCLIENTENCODING: 'UTF8' }, statement, 'sql', expectedSuccess);
  return expectedSuccess?result.stdout.trim():result;
}
function native(env, request, expectedSuccess = true, binary = args.binary) {
  const label = binary === args.binary ? 'native' : 'legacy-native';
  const result = command(binary, [], env, JSON.stringify({ schema: 'lattice.project-purge.request.v1', ...request }), `${label}-${request.action}`, expectedSuccess);
  return { ...result, value: result.exitCode === 0 ? JSON.parse(result.stdout) : null };
}
function rows(env, { includeMaintenance = false } = {}) {
  const excluded = includeMaintenance ? "'information_schema'" : "'information_schema','project_purge'";
  const tables = JSON.parse(sql(env, `SELECT coalesce(json_agg(json_build_array(n.nspname,c.relname) ORDER BY n.nspname,c.relname),'[]') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='r' AND n.nspname NOT LIKE 'pg_%' AND n.nspname NOT IN (${excluded})`));
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
function establish(order, { install = true } = {}) {
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
    LATTICE_PURGE_FIXTURE_ROOT: root, LATTICE_PURGE_FIXTURE_ONLY: '1', LATTICE_REGISTRY_ANCHOR_ROOT: path.join(root, 'anchors') };
  command(args['runtime-binary'], ['--postgres-initialize'], env, undefined, `${order}-initialize`);
  command(args['runtime-binary'], ['--postgres-bootstrap'], env, undefined, `${order}-bootstrap`);
  // Product bootstrap establishes the requested synthetic authority. This newly
  // created fixture has no running executor; enter its maintenance state before
  // installing the extension. The production purge itself never does this.
  assert.equal(sql(env, 'SELECT count(*) FROM control.project_registry_commands'), '0');
  const stopped = sql(env, "UPDATE control.runtime_admission SET admission_mode='STOPPED',daemon_instance_id=NULL,daemon_epoch=NULL,authority_revision=0,observation_digest=NULL,authority_head_digest=NULL WHERE singleton AND admission_mode='ACTIVE' AND daemon_instance_id='task050-fresh-process' AND daemon_epoch=50 AND authority_revision=50 AND observation_digest=decode(repeat('a',64),'hex') AND authority_head_digest=decode(repeat('b',64),'hex') RETURNING admission_mode");
  assert.match(stopped, /^STOPPED\r?\nUPDATE 1$/);
  const seeded = JSON.parse(command(args['seed-binary'], ['target-first', 'survivor-reference'].includes(order) ? [order] : [], env, undefined, `${order}-seed`).stdout);
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
  if (install) assert.equal(native(env, { action: 'install', authorization: 'INSTALL_PURGE_MAINTENANCE' }).value.status, 'INSTALLED');
  return { env, seeded, root };
}

try {
  if(['streaming','streaming-large'].includes(args.scenario)) {
    for(const large of [args.scenario==='streaming-large']) {
      const {env,seeded}=establish(large?'streaming-large':'streaming-small');
      env.LATTICE_DELIVERY_GIT_EXE=command('where.exe',['git.exe'],baseEnv,undefined,'fixture-git-path').stdout.trim().split(/\r?\n/)[0];
      command(args['epoch-binary'],['graph-install'],env,undefined,'graph-install');
      const graph=JSON.parse(command(args['epoch-binary'],[large?'graph-seed-large':'graph-seed-compat'],env,undefined,'graph-seed-streaming').stdout).records;
      const survivor=graph.find(row=>row.source==='survivor');
      const request={projectId:seeded.targetProjectId,operationId:large?'stream-large':'stream-small'};
      const old=native(env,{action:'preview',...request},!large,args['legacy-binary']);
      if(large) {
        assert.equal(old.exitCode,2); assert.match(old.stderr,/PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED/);
        const size=JSON.parse(sql(env,"SELECT json_build_object('count',count(*),'bytes',sum(octet_length(to_jsonb(p)::text))) FROM memory.codebase_memory_records p"));
        assert.equal(size.count,110000); assert.ok(size.bytes>64*1024*1024);
        check('valid-native-graph-fixture-exceeds-both-old-total-bounds',size);
      }
      const preview=native(env,{action:'preview',...request}).value;
      assert.equal(preview.status,'READY',JSON.stringify(preview.blockers));
      if(!large) {
        assert.deepEqual(preview,old.value);
        check('native-old-and-streaming-small-graph-preview-commitments-are-identical');
      }
      const analysis=sql(env,`SELECT encode(analysis_digest,'hex') FROM memory.codebase_memory_analyses WHERE configuration_digest=decode('${survivor.configuration}','hex')`);
      assert.match(analysis,/^[a-f0-9]{64}$/);
      const survivorDigest=()=>sql(env,`SELECT md5(string_agg(to_jsonb(p)::text,E'\\n' ORDER BY to_jsonb(p)::text COLLATE "C")) FROM memory.codebase_memory_records p WHERE analysis_digest=decode('${analysis}','hex')`);
      const before=survivorDigest();
      if(large) {
        const last=sql(env,`SELECT subject FROM memory.codebase_memory_records WHERE analysis_digest=decode('${analysis}','hex') AND ordinal=55000`);
        assert.match(last,/^[a-z0-9 ]+$/);
        sql(env,`UPDATE memory.codebase_memory_records SET subject=subject||' ${seeded.targetProjectId}' WHERE analysis_digest=decode('${analysis}','hex') AND ordinal=55000`);
        const cross=native(env,{action:'preview',...request}).value;
        assert.ok(cross.blockers.some(row=>row.code==='CROSS_SCOPE_OR_UNSUPPORTED_REFERENCE'&&row.table==='memory.codebase_memory_records'));
        sql(env,`UPDATE memory.codebase_memory_records SET subject='${last}' WHERE analysis_digest=decode('${analysis}','hex') AND ordinal=55000`);
        assert.equal(survivorDigest(),before);
        check('large-stream-still-scans-surviving-rows-for-target-references');
      }
      const apply={action:'apply',...request,expectedScopeDigest:preview.scopeDigest,authorization:'ERASE_PROJECT_DATA'};
      const result=native(env,apply,true,large?args.binary:args['legacy-binary']).value;
      assert.equal(result.status,'PURGED');
      assert.equal(result.counts['memory.codebase_memory_records'],large?55000:8);
      assert.equal(survivorDigest(),before);
      assert.deepEqual(native(env,{action:'status',...request}).value,result);
      assert.deepEqual(native(env,apply).value,result);
      const readback=JSON.parse(command(args['epoch-binary'],['graph-verify-survivor'],env,undefined,'stream-survivor-readback').stdout);
      assert.equal(readback.records[0].receipt,survivor.receipt);
      assert.equal(readback.records[0].reflection,survivor.reflection);
      check(large?'large-stream-purge-preserves-survivor-bytes-and-fresh-runtime-receipt':'stream-reader-replays-pre-streaming-purge-receipt-without-changing-digests');
      if(!large) {
        const second={projectId:seeded.survivorProjectId,operationId:'stream-reverse-compat'};
        const legacy=native(env,{action:'preview',...second},true,args['legacy-binary']).value;
        assert.deepEqual(native(env,{action:'preview',...second}).value,legacy);
        const action={action:'apply',...second,expectedScopeDigest:legacy.scopeDigest,authorization:'ERASE_PROJECT_DATA'};
        const written=native(env,action).value;
        assert.equal(written.status,'PURGED');
        assert.deepEqual(native(env,{action:'status',...second},true,args['legacy-binary']).value,written);
        assert.deepEqual(native(env,action,true,args['legacy-binary']).value,written);
        check('legacy-reader-replays-streaming-writer-receipt-and-old-plan-is-still-accepted');
      }
    }
  }
  if(args.scenario==='graph-ownership') {
    const {env,seeded,root}=establish('graph-ownership');
    env.LATTICE_DELIVERY_GIT_EXE=command('where.exe',['git.exe'],baseEnv,undefined,'fixture-git-path').stdout.trim().split(/\r?\n/)[0];
    command(args['epoch-binary'],['graph-install'],env,undefined,'graph-install');
    const records=JSON.parse(command(args['epoch-binary'],['graph-seed'],env,undefined,'graph-seed').stdout).records;
    const targetGraph=records.find(record=>record.source==='target'), survivorGraph=records.find(record=>record.source==='survivor');
    const before=rows(env);
    const graphRows=()=>rows(env).filter(([table])=>table.startsWith('memory.'));
    const beforeGraph=graphRows();
    const request={projectId:seeded.targetProjectId,operationId:'fixture-graph-ownership'};
    const preview=native(env,{action:'preview',...request}).value;
    assert.equal(preview.status,'READY',JSON.stringify(preview.blockers));
    for(const table of ['analyses','records','retrieval_audits','receipts','reflections']) assert.equal(preview.counts['memory.codebase_memory_'+table],1);
    assert.equal(preview.graphSourceProof.targetAnalyses,1);
    assert.equal(preview.graphSourceProof.survivorAnalyses,1);
    assert.equal(preview.graphSourceProof.unattributableAnalyses,0);
    assert.deepEqual(rows(env),before);
    check('real-memory-rows-under-shared-project-key-classified-by-runtime-source-digest');
    const changedGit=path.join(root,'different-git.exe');
    writeFileSync(changedGit,'synthetic changed Git bytes; never executed');
    const unknown=native({...env,LATTICE_DELIVERY_GIT_EXE:changedGit},{action:'preview',...request}).value;
    assert.equal(unknown.status,'BLOCKED');
    assert.ok(unknown.blockers.some(item=>item.code==='GRAPH_ANALYSIS_OWNERSHIP_UNATTRIBUTABLE'));
    assert.deepEqual(graphRows(),beforeGraph);
    check('unmatched-historical-runtime-configuration-blocks-without-claiming-unrelated');
    assert.match(targetGraph.reflection,/^[a-f0-9]{64}$/);
    sql(env,`UPDATE memory.codebase_memory_reflections SET project_id='purge-survivor' WHERE reflection_receipt_digest=decode('${targetGraph.reflection}','hex')`);
    const crossBefore=rows(env);
    const cross=native(env,{action:'preview',...request}).value;
    assert.equal(cross.status,'BLOCKED');
    assert.ok(cross.blockers.some(item=>item.code==='CROSS_SCOPE_OR_UNSUPPORTED_REFERENCE'&&item.table==='memory.codebase_memory_reflections'));
    const refused=native(env,{action:'apply',...request,expectedScopeDigest:cross.scopeDigest,authorization:'ERASE_PROJECT_DATA'}).value;
    assert.equal(refused.status,'BLOCKED'); assert.deepEqual(rows(env),crossBefore);
    sql(env,`UPDATE memory.codebase_memory_reflections SET project_id='task032-delivery' WHERE reflection_receipt_digest=decode('${targetGraph.reflection}','hex')`);
    assert.deepEqual(rows(env),before);
    check('cross-project-reflection-reference-blocks-before-erasure-and-retains-every-row');
    const applied=native(env,{action:'apply',...request,expectedScopeDigest:preview.scopeDigest,authorization:'ERASE_PROJECT_DATA'}).value;
    assert.equal(applied.status,'PURGED',JSON.stringify(applied));
    const after=graphRows();
    const targetNeedles=[targetGraph.receipt,targetGraph.configuration];
    assert.equal(selectRows(after,targetNeedles).length,0);
    const survivorBefore=selectRows(beforeGraph,[survivorGraph.receipt,survivorGraph.configuration]);
    assert.deepEqual(selectRows(after,[survivorGraph.receipt,survivorGraph.configuration]),survivorBefore);
    for(const table of ['analyses','records','retrieval_audits','receipts','reflections']) {
      const retained=after.find(([name])=>name==='memory.codebase_memory_'+table)[1];
      assert.equal(retained.length,1);
      assert.ok(beforeGraph.find(([name])=>name==='memory.codebase_memory_'+table)[1].some(row=>JSON.stringify(row)===JSON.stringify(retained[0])));
    }
    const survivor=JSON.parse(command(args['epoch-binary'],['graph-verify-survivor'],env,undefined,'graph-survivor-readback').stdout);
    assert.equal(survivor.records[0].receipt,survivorGraph.receipt);
    assert.equal(survivor.records[0].reflection,survivorGraph.reflection);
    assert.equal(native(env,{action:'status',...request}).value.status,'PURGED');
    check('real-graph-child-closure-erased-and-fresh-process-survivor-receipt-replays-unchanged');
  }
  if (args.scenario === 'bot-purge') {
    const {env,seeded,root}=establish('bot-purge');
    const botPort=Number(args['bot-port']), botRun=randomUUID().replaceAll('-','');
    const botMarker=JSON.parse(readFileSync(path.join(runRoot,'bot-fixture-owner.json'),'utf8'));
    assert.equal(botMarker.port,botPort); assert.equal(path.dirname(botMarker.path),runRoot);
    assert.ok(botPort!==port&&![4317,5432,55432,58743,64272].includes(botPort));
    const botDb=`lattice_bot_lifecycle_${botRun}`;
    const botSql=(database,statement,success=true,user='runtime_bootstrap')=>command(args.psql,
      ['-X','-A','-t','-h','127.0.0.1','-p',String(botPort),'-U',user,'-d',database,'-v','ON_ERROR_STOP=1','-f','-'],
      {...env,PGCLIENTENCODING:'UTF8'},statement,'bot-purge-sql',success);
    botSql('postgres',`CREATE ROLE lattice_migrator NOLOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; CREATE ROLE lattice_runtime NOLOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
      CREATE ROLE lattice_migrator_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '${password}'; CREATE ROLE lattice_runtime_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '${password}';
      GRANT lattice_migrator TO lattice_migrator_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; GRANT lattice_runtime TO lattice_runtime_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE;`);
    const botService={port:botPort,runId:botRun,systemIdentifier:botSql('postgres','SELECT system_identifier::text FROM pg_control_system()').stdout.trim()};
    const botArgs=['--postgres-host','127.0.0.1','--postgres-port',String(botPort),'--postgres-run-id',botRun];
    command(args['lifecycle-binary'],['bot-lifecycle-install',...botArgs],env,undefined,'bot-base-install');
    native(env,{action:'install-bot-ownership',authorization:'INSTALL_BOT_PROJECT_OWNERSHIP',botService});
    const owner='11111111-2222-3333-4444-555555555555';
    const registration=project=>({action:'register',request_id:'fixture-register',project_id:project,role_id:'fixture-role',expected_revision:0,expected_generation:0,owner_thread_id:owner,owner_host_id:'fixture',
      body:{work_ids:[],policy_digest:'a'.repeat(64),rules_digest:'b'.repeat(64),binding_receipt:{tool:'read_thread',target_thread_id:owner,target_host_id:'fixture',result_digest:'c'.repeat(64),readback_digest:'d'.repeat(64),evidence_ref:'synthetic-fixture:bot-purge',success:true,readback_verified:true,old_pending_count:0}}});
    const botCall=(value,success=true)=>command(args['lifecycle-binary'],['bot-lifecycle',...botArgs],env,JSON.stringify(value),'bot-lifecycle-call',success);
    const missing=botCall(registration('not-in-registry'),false);
    assert.notEqual(missing.exitCode,0);assert.match(missing.stderr,/REGISTRY_PROJECT_MISSING/);
    const originals=[seeded.targetProjectId,seeded.survivorProjectId].map(project=>JSON.parse(botCall(registration(project)).stdout));
    const botRows=()=>JSON.parse(botSql(botDb,"SELECT json_build_array((SELECT coalesce(json_agg(r ORDER BY r::text),'[]') FROM (SELECT to_jsonb(p) r FROM bot_lifecycle.roles p) x),(SELECT coalesce(json_agg(r ORDER BY r::text),'[]') FROM (SELECT to_jsonb(p) r FROM bot_lifecycle.events p) x),(SELECT coalesce(json_agg(r ORDER BY r::text),'[]') FROM (SELECT to_jsonb(p) r FROM bot_project_ownership.bindings p) x),(SELECT coalesce(json_agg(r ORDER BY r::text),'[]') FROM (SELECT to_jsonb(p) r FROM bot_project_ownership.retired p) x))").stdout);
    const before=botRows();
    assert.equal(before[2].length,2);
    check('bot-registration-requires-current-registry-project-and-atomically-persists-binding');
    const req={botService,projectId:seeded.targetProjectId,operationId:'fixture-bot-purge'};
    const preview=native(env,{action:'preview-bot',...req}).value;
    assert.equal(preview.status,'READY',JSON.stringify(preview));assert.equal(preview.roleCount,1);assert.equal(preview.eventCount,1);
    assert.deepEqual(botRows(),before);
    const boundaries=[{roleId:'fixture-role',revision:1,generation:1,boundary:{source:'codex.read_thread',observed_at:new Date().toISOString(),thread_id:owner,host_id:'fixture',latest_turn_id:'22222222-2222-3333-4444-555555555555',thread_updated_at:1,status:'idle',latest_turn_status:'completed',pending_input_count:0,in_flight_count:0,readback_digest:'e'.repeat(64),evidence_ref:'synthetic-fixture:read-thread'}}];
    for(const mutation of ['running','stale-generation','stale-time']) {
      const bad=structuredClone(boundaries);
      if(mutation==='running')bad[0].boundary.status='running';
      if(mutation==='stale-generation')bad[0].generation=0;
      if(mutation==='stale-time')bad[0].boundary.observed_at='2020-01-01T00:00:00.000Z';
      const rejected=native(env,{action:'apply-bot',...req,expectedBotScopeDigest:preview.scopeDigest,authorization:'ERASE_PROJECT_DATA',botBoundaries:bad},false);
      assert.notEqual(rejected.exitCode,0);assert.match(rejected.stderr,/NATIVE_BOUNDARY_REJECTED/);assert.deepEqual(botRows(),before);
    }
    check('running-stale-owner-and-stale-native-readback-block-before-any-bot-erasure');
    botSql(botDb,'ALTER TABLE bot_lifecycle.roles DISABLE TRIGGER project_retirement_v1');
    assert.match(native(env,{action:'preview-bot',...req},false).stderr,/OWNERSHIP_SCHEMA_REJECTED/);
    botSql(botDb,'ALTER TABLE bot_lifecycle.roles ENABLE TRIGGER project_retirement_v1');
    command(args['lifecycle-binary'],['bot-lifecycle-install',...botArgs],env,undefined,'old-installer-retains-new-trigger');
    assert.equal(native(env,{action:'preview-bot',...req}).value.scopeDigest,preview.scopeDigest);
    assert.deepEqual(botRows(),before);
    check('disabled-retirement-trigger-rejected-and-existing-installer-preserves-protection');
    const {LatticeStore}=await import('../apps/lattice-control/src/store.mjs');
    const sqlitePath=path.join(root,'control.sqlite'), store=new LatticeStore(sqlitePath);
    for (const kind of ['target','survivor']) {
      store.database.prepare('INSERT INTO projects(id,name,root_path,created_at,updated_at) VALUES(?,?,?,?,?)')
        .run(seeded[`${kind}ProjectId`],`Synthetic ${kind}`,seeded[`${kind}CanonicalPath`],'2026-10-09T00:00:00Z','2026-10-09T00:00:00Z');
    }
    const decide=(decisionOwner,scope,clientRequestId,supersedesDecisionId)=> {
      const state=store.decisionStateIdentity();
      return store.recordDecision({owner:decisionOwner,scope,subject:'synthetic',content:'Synthetic decision',rationale:'Synthetic verification',
        source:{kind:'user_confirmation',reference:'thread:synthetic-bot-fixture/turn:1'},clientRequestId,
        expectedRevision:state.revision,expectedDigest:state.digest,...(supersedesDecisionId?{supersedesDecisionId}:{})});
    };
    const aDecision=decide({kind:'PROJECT',projectId:seeded.targetProjectId},'scope-a','fixture-decision-a1');
    decide({kind:'PROJECT',projectId:seeded.targetProjectId},'scope-a','fixture-decision-a2',aDecision.decision.id);
    decide({kind:'PROJECT',projectId:seeded.survivorProjectId},'scope-b','fixture-decision-b');
    decide({kind:'GLOBAL'},'global','fixture-decision-global');
    const sqliteSurvivors=store.database.prepare("SELECT rowid,* FROM decisions WHERE scope<>'scope-a' ORDER BY rowid").all();
    store.close();
    const configPath=path.join(root,'purge-config.json'),planPath=path.join(root,'purge-plan.json'),boundaryPath=path.join(root,'native-boundaries.json');
    writeFileSync(configPath,JSON.stringify({projectId:seeded.targetProjectId,operationId:req.operationId,nativeBinary:path.resolve(args.binary),databasePath:sqlitePath,statePath:path.join(root,'progress.json'),codeGraphCacheDirectory:path.join(root,'control-graph'),runtimeGraphWorkDirectory:path.join(root,'runtime-graph'),botService,botBoundaryPath:boundaryPath}));
    const cliPath=fileURLToPath(new URL('../apps/lattice-control/src/project-purge-client.mjs',import.meta.url));
    const cli=(action,flags)=>JSON.parse(command(process.execPath,[cliPath,action,...flags],env,undefined,`bot-purge-cli-${action}`).stdout);
    const plan=cli('inventory',['--input',configPath,'--plan',planPath]);assert.equal(plan.status,'READY',JSON.stringify(plan.blockers));assert.equal(plan.bot.scopeDigest,preview.scopeDigest);
    boundaries[0].boundary.observed_at=new Date().toISOString();writeFileSync(boundaryPath,JSON.stringify(boundaries));
    const applied=cli('apply',['--plan',planPath,'--confirm',plan.digest,'--maintenance-offline','--bot-boundaries',boundaryPath]);
    assert.equal(applied.status,'SCOPED_PURGED');assert.equal(applied.bot.status,'PURGED');
    assert.equal(applied.report.remaining.some(item=>item.kind==='botLifecycle'),false);
    assert.equal(applied.report.remaining.some(item=>item.kind==='controlDecisions'),false);
    assert.equal(plan.sqlite.counts.decisions,2);assert.equal(plan.sqlite.strategy,'REBUILD_SURVIVORS_V1');
    const rebuilt=new LatticeStore(sqlitePath);
    try {
      assert.deepEqual(rebuilt.database.prepare('SELECT rowid,* FROM decisions ORDER BY rowid').all(),sqliteSurvivors);
      assert.equal(rebuilt.decisionStateIdentity().revision,2);
      assert.equal(rebuilt.database.prepare('SELECT count(*) AS count FROM decision_request_tombstones').get().count,2);
      assert.equal(rebuilt.database.prepare('SELECT count(*) AS count FROM decision_purge_receipts').get().count,1);
    } finally { rebuilt.close(); }
    check('real-cli-erases-complete-owned-decision-lineage-and-preserves-survivor-and-global-rowids');
    const after=botRows();
    for(let i=0;i<3;i++)assert.deepEqual(after[i],before[i].filter(row=>row.project_id===seeded.survivorProjectId));
    assert.equal(after[3].length,1);assert.deepEqual(Object.keys(after[3][0]),['pair_digest']);
    const bRead=JSON.parse(botCall({action:'read',project_id:seeded.survivorProjectId,role_id:'fixture-role'}).stdout);
    assert.deepEqual(bRead.current,originals[1].current);
    assert.equal(existsSync(seeded.targetCanonicalPath),false);assert.equal(existsSync(seeded.survivorCanonicalPath),true);
    assert.equal(sql(env,"SELECT count(*) FROM control.project_registry_projects WHERE project_id='purge-target'"),'0');
    check('real-two-cluster-cli-purges-bot-before-registry-and-preserves-survivor-state-and-receipts');
    const old=botSql(botDb,`SET ROLE lattice_runtime; BEGIN ISOLATION LEVEL SERIALIZABLE; SELECT bot_lifecycle.apply_v1('${JSON.stringify(registration(seeded.targetProjectId))}'::jsonb); COMMIT;`,false,'lattice_runtime_login');
    assert.notEqual(old.exitCode,0);assert.match(old.stderr,/PROJECT_RETIRED/);assert.deepEqual(botRows(),after);
    assert.equal(native(env,{action:'status-bot',...req,expectedBotScopeDigest:preview.scopeDigest}).value.status,'PURGED');
    assert.equal(cli('resume',['--plan',planPath,'--confirm',plan.digest,'--maintenance-offline']).status,'SCOPED_PURGED');
    assert.deepEqual(botRows(),after);
    check('old-apply-function-cannot-recreate-retired-pair-and-exact-retry-works-after-registry-erasure');
    const finalized=cli('finalize',['--plan',planPath,'--confirm',plan.digest,'--maintenance-offline']);
    assert.equal(finalized.status,'LOGICAL_SCOPE_COMPLETE');assert.equal(finalized.latticeScopeComplete,true);assert.equal(finalized.complete,false);
    for(const file of [configPath,planPath,boundaryPath,path.join(root,'progress.json')])assert.equal(existsSync(file),false);
    assert.equal(JSON.stringify(finalized).includes(seeded.targetProjectId),false);assert.equal(JSON.stringify(finalized).includes(root),false);
    assert.deepEqual(cli('verify-finalization',['--finalization',plan.maintenance.finalizationPath]),finalized);
    assert.deepEqual(cli('resume-finalize',['--finalization',plan.maintenance.finalizationPath,'--confirm',plan.digest,'--maintenance-offline']),finalized);
    check('cli-finalizes-planned-maintenance-files-and-retains-only-content-free-attestation');
  }
  if (args.scenario === 'bot-inventory') {
    const runId=randomUUID().replaceAll('-',''), root=path.join(runRoot,'bot-inventory');
    mkdirSync(root); writeFileSync(path.join(root,'project-purge-fixture.marker'),runId+'\n');
    const env={...baseEnv,LATTICE_TASK019_PORT:String(port),LATTICE_TASK019_RUN_ID:runId,LATTICE_TASK019_PASSWORD:password,LATTICE_PURGE_FIXTURE_ONLY:'1',LATTICE_PURGE_FIXTURE_ROOT:root};
    const botSql=(database,statement)=>command(args.psql,['-X','-A','-t','-h','127.0.0.1','-p',String(port),'-U','runtime_bootstrap','-d',database,'-v','ON_ERROR_STOP=1','-f','-'],{...env,PGCLIENTENCODING:'UTF8'},statement,'bot-sql').stdout.trim();
    botSql('postgres',`CREATE ROLE lattice_migrator NOLOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; CREATE ROLE lattice_runtime NOLOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
      CREATE ROLE lattice_migrator_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '${password}'; CREATE ROLE lattice_runtime_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '${password}';
      GRANT lattice_migrator TO lattice_migrator_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; GRANT lattice_runtime TO lattice_runtime_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE;`);
    const botService={port,runId,systemIdentifier:botSql('postgres','SELECT system_identifier::text FROM pg_control_system()')};
    const request={action:'inspect-bot',projectId:'purge-target',botService};
    const inspect=()=>native(env,request).value;
    assert.equal(inspect().discovery,'VERIFIED_ABSENT');
    check('native-bot-inventory-proves-configured-database-absent');
    command(args['epoch-binary'],['bot-install'],env,undefined,'bot-install');
    const empty=inspect();
    assert.equal(empty.discovery,'VERIFIED_EMPTY',JSON.stringify(empty));
    assert.deepEqual(empty.counts,{roles:0,events:0});
    check('native-bot-inventory-verifies-exact-installed-empty-schema');
    command(args['epoch-binary'],['bot-register'],env,undefined,'bot-register');
    const snapshot=()=>botSql(`lattice_bot_lifecycle_${runId}`,"SELECT json_build_array((SELECT json_agg(r ORDER BY r::text) FROM (SELECT to_jsonb(p) r FROM bot_lifecycle.roles p) x),(SELECT json_agg(r ORDER BY r::text) FROM (SELECT to_jsonb(p) r FROM bot_lifecycle.events p) x))");
    const before=snapshot();
    const populated=inspect();
    assert.equal(populated.discovery,'OBSERVED',JSON.stringify(populated));
    assert.deepEqual(populated.counts,{roles:2,events:2});
    assert.deepEqual(populated.exactKeyMatches,{roles:1,events:1});
    assert.equal(populated.identityBinding,'TEXT_KEY_ONLY_REGISTRY_BINDING_NOT_PROVEN');
    assert.equal(populated.distinctProjectKeys,2);
    assert.deepEqual(inspect(),populated);
    assert.equal(snapshot(),before);
    check('real-bot-records-counted-without-mutation-or-invented-registry-ownership');
    const missed=native(env,{...request,projectId:'purge-survivor'}).value;
    assert.equal(missed.discovery,'OBSERVED');
    assert.deepEqual(missed.exactKeyMatches,{roles:0,events:0});
    assert.equal(missed.identityBinding,'TEXT_KEY_ONLY_REGISTRY_BINDING_NOT_PROVEN');
    check('different-bot-project-key-does-not-falsely-prove-absence');
    const wrong=native(env,{...request,botService:{...botService,systemIdentifier:'1'}},false);
    assert.notEqual(wrong.exitCode,0); assert.match(wrong.stderr,/CLUSTER_IDENTITY_REJECTED/);
    assert.equal(snapshot(),before);
    check('wrong-dedicated-service-identity-rejected-without-mutation');
  }
  if (args.scenario === 'upgrade') {
  assert.notEqual(evidence.binaries.binary.sha256, evidence.binaries['legacy-binary'].sha256);
  const { env, seeded } = establish('upgrade');
  const request = { projectId: seeded.targetProjectId, operationId: 'fixture-receipt-upgrade' };
  const beforePreview = rows(env, { includeMaintenance: true });
  const legacyPreview = native(env, { action: 'preview', ...request }, true, args['legacy-binary']).value;
  const candidatePreview = native(env, { action: 'preview', ...request }).value;
  assert.equal(legacyPreview.status, 'READY');
  assert.equal(candidatePreview.status, 'READY');
  assert.notEqual(legacyPreview.scopeDigest, candidatePreview.scopeDigest);
  assert.deepEqual(rows(env, { includeMaintenance: true }), beforePreview);
  check('real-legacy-and-candidate-binaries-produce-distinct-scope-digests-for-same-live-fixture');
  const survivorNeedles = ['survivorProjectId', 'survivorStreamId', 'survivorTaskRef', 'survivorCanonicalPath'].map(key => seeded[key]);
  const targetNeedles = ['targetProjectId', 'targetStreamId', 'targetTaskRef', 'targetCanonicalPath'].map(key => seeded[key]);
  const survivorBefore = hash(selectRows(beforePreview, survivorNeedles));
  const legacyApply = { action: 'apply', ...request, expectedScopeDigest: legacyPreview.scopeDigest, authorization: 'ERASE_PROJECT_DATA' };
  const originalReceipt = native(env, legacyApply, true, args['legacy-binary']).value;
  assert.equal(originalReceipt.status, 'PURGED');
  assert.equal(originalReceipt.scopeDigest, legacyPreview.scopeDigest);
  const afterLegacy = rows(env, { includeMaintenance: true });
  assert.equal(selectRows(rows(env), targetNeedles).length, 0);
  assert.equal(hash(selectRows(afterLegacy, survivorNeedles)), survivorBefore);
  assert.equal(sql(env, "SELECT count(*) FROM project_purge.receipts WHERE operation_id='fixture-receipt-upgrade'"), '1');
  check('real-legacy-binary-creates-committed-purge-receipt-and-preserves-survivor');
  const status = native(env, { action: 'status', ...request }).value;
  const resumed = native(env, legacyApply).value;
  assert.deepEqual(status, originalReceipt);
  assert.deepEqual(resumed, originalReceipt);
  assert.deepEqual(rows(env, { includeMaintenance: true }), afterLegacy);
  check('candidate-status-and-idempotent-apply-replay-original-legacy-receipt-without-mutation');
  const rebound = native(env, { ...legacyApply, expectedScopeDigest: candidatePreview.scopeDigest }, false);
  assert.notEqual(rebound.exitCode, 0);
  assert.match(rebound.stderr, /PROJECT_PURGE_IDEMPOTENCY_CONFLICT/);
  assert.deepEqual(rows(env, { includeMaintenance: true }), afterLegacy);
  check('upgrade-cannot-rebind-committed-legacy-operation-to-new-scope-digest');
  const replay = JSON.parse(command(args['seed-binary'], ['verify-survivor'], env, undefined, 'upgrade-survivor-replay').stdout);
  assert.equal(replay.status, 'VERIFIED');
  assert.equal(replay.survivorTaskRef, seeded.survivorTaskRef);
  check('fresh-process-survivor-replay-after-legacy-receipt-upgrade');
  }

  if (args.scenario === 'inventory') {
  const { env, seeded } = establish('inventory', { install: false });
  const request = { projectId: seeded.targetProjectId, operationId: 'fixture-inventory' };
  const maintenanceCatalog = () => JSON.parse(sql(env, `SELECT json_build_object(
    'namespace', (SELECT to_jsonb(n) FROM pg_namespace n WHERE n.nspname='project_purge'),
    'relations', (SELECT coalesce(json_agg(to_jsonb(c) ORDER BY c.oid),'[]') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='project_purge'),
    'constraints', (SELECT coalesce(json_agg(to_jsonb(c) ORDER BY c.oid),'[]') FROM pg_constraint c JOIN pg_namespace n ON n.oid=c.connamespace WHERE n.nspname='project_purge'),
    'routines', (SELECT coalesce(json_agg(to_jsonb(p) ORDER BY p.oid),'[]') FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='project_purge'))`));
  const snapshot = () => ({ rows: rows(env, { includeMaintenance: true }), maintenance: maintenanceCatalog() });
  const assertCounts = preview => {
    assert.equal(preview.inventoryMode, 'READ_ONLY_SNAPSHOT');
    assert.equal(preview.project.id, seeded.targetProjectId);
    assert.equal(preview.project.canonicalPath, seeded.targetCanonicalPath);
    assert.ok(preview.counts['control.task_ledger_streams'] >= 1);
    assert.ok(preview.counts['control.project_registry_projects'] >= 1);
    assert.match(preview.scopeDigest, /^[a-f0-9]{64}$/);
  };
  const beforeUninstalled = snapshot();
  assert.equal(beforeUninstalled.maintenance.namespace, null);
  const stoppedPreview = native(env, { action: 'preview', ...request }).value;
  assertCounts(stoppedPreview);
  assert.equal(stoppedPreview.status, 'BLOCKED');
  assert.equal(stoppedPreview.maintenanceStopped, true);
  assert.equal(stoppedPreview.maintenanceExtensionInstalled, false);
  assert.ok(stoppedPreview.blockers.some(item => item.code === 'MAINTENANCE_EXTENSION_REQUIRED'));
  assert.ok(!stoppedPreview.blockers.some(item => item.code === 'MAINTENANCE_OFFLINE_REQUIRED'));
  const uninstalledApply = native(env, { action: 'apply', ...request, expectedScopeDigest: stoppedPreview.scopeDigest, authorization: 'ERASE_PROJECT_DATA' }, false);
  assert.notEqual(uninstalledApply.exitCode, 0);
  assert.equal(uninstalledApply.stderr.trim(), 'PROJECT_PURGE_EXTENSION_REJECTED');
  assert.deepEqual(snapshot(), beforeUninstalled);
  check('uninstalled-stopped-preview-inventories-real-project-and-apply-cannot-mutate');

  // Only this owned synthetic cluster changes admission. The production purge
  // must observe ACTIVE and reject maintenance writes without changing it.
  const active = sql(env, "UPDATE control.runtime_admission SET admission_mode='ACTIVE',daemon_instance_id='task050-fresh-process',daemon_epoch=50,authority_revision=50,observation_digest=decode(repeat('a',64),'hex'),authority_head_digest=decode(repeat('b',64),'hex') WHERE singleton AND admission_mode='STOPPED' AND daemon_instance_id IS NULL AND daemon_epoch IS NULL AND authority_revision=0 AND observation_digest IS NULL AND authority_head_digest IS NULL RETURNING admission_mode");
  assert.match(active, /^ACTIVE\r?\nUPDATE 1$/);
  const beforeActive = snapshot();
  const activePreview = native(env, { action: 'preview', ...request }).value;
  assertCounts(activePreview);
  assert.equal(activePreview.status, 'BLOCKED');
  assert.equal(activePreview.maintenanceStopped, false);
  assert.equal(activePreview.maintenanceExtensionInstalled, false);
  assert.ok(activePreview.blockers.some(item => item.code === 'MAINTENANCE_EXTENSION_REQUIRED'));
  assert.ok(activePreview.blockers.some(item => item.code === 'MAINTENANCE_OFFLINE_REQUIRED'));
  assert.deepEqual(activePreview.counts, stoppedPreview.counts);
  assert.deepEqual(snapshot(), beforeActive);
  check('active-uninstalled-preview-is-read-only-and-reports-both-maintenance-blockers');
  for (const attempted of [
    { action: 'apply', ...request, expectedScopeDigest: activePreview.scopeDigest, authorization: 'ERASE_PROJECT_DATA' },
    { action: 'install', authorization: 'INSTALL_PURGE_MAINTENANCE' },
  ]) {
    const rejected = native(env, attempted, false);
    assert.notEqual(rejected.exitCode, 0);
    assert.match(rejected.stderr, /PROJECT_PURGE_MAINTENANCE_(PROFILE_REQUIRED|OFFLINE_REQUIRED|EXTENSION_REQUIRED)/);
  }
  assert.deepEqual(snapshot(), beforeActive);
  assert.equal(sql(env, "SELECT admission_mode FROM control.runtime_admission WHERE singleton"), 'ACTIVE');
  check('active-apply-and-install-reject-with-all-rows-and-maintenance-catalog-unchanged');

  const stopped = sql(env, "UPDATE control.runtime_admission SET admission_mode='STOPPED',daemon_instance_id=NULL,daemon_epoch=NULL,authority_revision=0,observation_digest=NULL,authority_head_digest=NULL WHERE singleton AND admission_mode='ACTIVE' AND daemon_instance_id='task050-fresh-process' AND daemon_epoch=50 AND authority_revision=50 AND observation_digest=decode(repeat('a',64),'hex') AND authority_head_digest=decode(repeat('b',64),'hex') RETURNING admission_mode");
  assert.match(stopped, /^STOPPED\r?\nUPDATE 1$/);
  assert.equal(native(env, { action: 'install', authorization: 'INSTALL_PURGE_MAINTENANCE' }).value.status, 'INSTALLED');
  const beforeReady = snapshot();
  assert.ok(beforeReady.maintenance.namespace);
  const ready = native(env, { action: 'preview', ...request }).value;
  assertCounts(ready);
  assert.equal(ready.status, 'READY', JSON.stringify(ready.blockers));
  assert.equal(ready.maintenanceStopped, true);
  assert.equal(ready.maintenanceExtensionInstalled, true);
  assert.deepEqual(ready.blockers, []);
  assert.deepEqual(ready.counts, activePreview.counts);
  assert.deepEqual(snapshot(), beforeReady);
  check('fresh-stopped-installed-preview-is-ready-and-still-read-only');
  for (const preview of [stoppedPreview, activePreview]) {
    assert.notEqual(preview.scopeDigest, ready.scopeDigest);
    const stale = native(env, { action: 'apply', ...request, expectedScopeDigest: preview.scopeDigest, authorization: 'ERASE_PROJECT_DATA' }, false);
    assert.notEqual(stale.exitCode, 0);
    assert.match(stale.stderr, /PROJECT_PURGE_STALE_SCOPE/);
  }
  assert.deepEqual(snapshot(), beforeReady);
  assert.equal(sql(env, 'SELECT count(*) FROM project_purge.receipts'), '0');
  check('blocked-online-or-uninstalled-digests-cannot-authorize-later-maintenance-apply');
  }

  if (['epoch','epoch-reference'].includes(args.scenario)) {
    for (const order of [args.scenario==='epoch'?'target-first':'survivor-reference']) {
      const {env,seeded}=establish(order);
      const request={projectId:seeded.targetProjectId,operationId:`epoch-${order}`,registryPolicy:'MINIMAL_ATTESTATION'};
      assert.equal(native(env,{action:'install-epoch',authorization:'INSTALL_REGISTRY_EPOCH_MAINTENANCE'}).value.status,'INSTALLED');
      const vectors=[['register-alpha','811300be1d2eebe93b7f5a4cdcc1a06aeb5033f9b48d8630a3557456ebae8fc1'],['命令-台灣','4129561f037081570f776d844308446a06f5fecdbc7cae04feaec81f479a4c43'],['quote"slash\\line\nend','028fcad801490b3ad55228ac222462d20e7b4c6cc602a92e0364d9ec2b01d458']];
      for(const [id,commitment] of vectors) assert.equal(sql(env,`SELECT registry_epoch.command_key_v1($fixture$${id}$fixture$)`),commitment);
      assert.equal(sql(env,"SELECT registry_epoch.command_key_v1(U&'e\\0301')=registry_epoch.command_key_v1('é')"),'t');
      check(`${order}-sql-command-commitments-match-pure-unicode-and-escaping-vectors`);
      const original=rows(env);
      const originalRetry=JSON.parse(command(args['epoch-binary'],['retry-survivor'],env,undefined,`${order}-original-survivor-retry`).stdout);
      const originalCommands=original.find(([table])=>table==='control.project_registry_commands')[1];
      const survivorCommand=originalCommands.find(row=>row.command_id==='fixture-register-purge-survivor');
      const survivorProjects=original.find(([table])=>table==='control.project_registry_projects')[1].filter(row=>row.project_id===seeded.survivorProjectId);
      const preview=native(env,{action:'preview',...request}).value;
      assert.equal(preview.status,'READY',JSON.stringify(preview.blockers));
      assert.equal(preview.history.assurance,'ATTESTED_FROM_SEAL');
      assert.equal(preview.history.redactedSurvivorCommands.length,order==='survivor-reference'?1:0);
      assert.deepEqual(rows(env),original);
      check(`${order}-epoch-preview-binds-explicit-survivor-redactions-without-writing`);
      for(const [change,restore] of [
        ['GRANT SELECT ON registry_epoch.current_seal TO lattice_runtime','REVOKE SELECT ON registry_epoch.current_seal FROM lattice_runtime'],
        ['ALTER FUNCTION registry_epoch.read_v1() VOLATILE','ALTER FUNCTION registry_epoch.read_v1() STABLE'],
      ]) {
        sql(env,change);assert.notEqual(native(env,{action:'preview',...request},false).exitCode,0);sql(env,restore);
      }
      check(`${order}-expanded-table-grant-and-altered-read-function-rejected`);
      const anchorRoot=preview.protectedRoots.find(root=>path.dirname(root)===env.LATTICE_REGISTRY_ANCHOR_ROOT);
      assert.ok(anchorRoot); assert.match(path.basename(anchorRoot),/^[a-f0-9]{64}$/);
      const anchorPath=path.join(anchorRoot,'registry-epoch.anchor.json');
      const canonical=value=>Array.isArray(value)?value.map(canonical):value!==null&&typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(key=>[key,canonical(value[key])])):value;
      const operationDigest=createHash('sha256').update(`${request.operationId}\n${request.projectId}\n${preview.scopeDigest}`).digest('hex');
      const pending={schema:'lattice.registry-epoch.anchor.v1',dbIdentityDigest:path.basename(anchorRoot),status:'pending',previous:null,next:{epoch:1,sealDigest:preview.history.sealDigest},operationDigest};
      mkdirSync(anchorRoot,{recursive:true});writeFileSync(anchorPath,JSON.stringify(canonical(pending)));
      assert.notEqual(command(args['epoch-binary'],['retry-survivor'],env,undefined,`${order}-pending-reader`,false).exitCode,0);
      assert.notEqual(native(env,{action:'preview',...request},false).exitCode,0);
      assert.notEqual(native(env,{action:'apply',...request,operationId:'wrong-operation',expectedScopeDigest:preview.scopeDigest,authorization:'ERASE_PROJECT_DATA'},false).exitCode,0);
      assert.deepEqual(rows(env),original);
      assert.equal(native(env,{action:'preview',...request,expectedScopeDigest:preview.scopeDigest}).value.scopeDigest,preview.scopeDigest);
      check(`${order}-precommit-pending-state-blocks-runtime-and-wrong-operation-but-resumes-exact-source`);
      const applied=native(env,{action:'apply',...request,expectedScopeDigest:preview.scopeDigest,authorization:'ERASE_PROJECT_DATA'}).value;
      assert.equal(applied.status,'PURGED');
      const after=rows(env);
      const targetNeedles=[seeded.targetProjectId,seeded.targetCanonicalPath,seeded.targetTaskRef,seeded.targetStreamId];
      assert.deepEqual(selectRows(after,targetNeedles),[]);
      assert.deepEqual(after.find(([table])=>table==='control.project_registry_projects')[1],survivorProjects);
      const seal=after.find(([table])=>table==='registry_epoch.current_seal')[1][0];
      assert.deepEqual(seal.payload.commandRows,[survivorCommand]);
      assert.equal(seal.epoch,1);
      assert.equal(sql(env,'SELECT count(*) FROM control.project_registry_commands'),'0');
      for(const [id,code] of [['fixture-register-purge-target','REGISTRY_COMMAND_REDACTED'],['fixture-register-purge-survivor','REGISTRY_ARCHIVED_COMMAND_REPLAY_REQUIRED']]) {
        const originalRow=originalCommands.find(row=>row.command_id===id);
        const probe=sql(env,`INSERT INTO control.project_registry_commands SELECT * FROM jsonb_populate_record(NULL::control.project_registry_commands,'${JSON.stringify(originalRow).replaceAll("'","''")}'::jsonb)`,false);
        assert.notEqual(probe.exitCode,0);assert.ok(probe.stderr.includes(code),probe.stderr);
      }
      check(`${order}-sql-insert-guard-rejects-erased-and-archived-command-ids`);
      const inspected=JSON.parse(command(args['epoch-binary'],['retry-survivor'],env,undefined,`${order}-epoch-runtime-readback`).stdout);
      assert.deepEqual(inspected,originalRetry);
      assert.equal(JSON.parse(command(args['epoch-binary'],['redacted'],env,undefined,`${order}-redacted-id`).stdout).status,'REDACTED_ID_REJECTED');
      check(`${order}-fresh-runtime-verifies-anchored-baseline-and-original-survivor-row`);
      assert.equal(sql(env,`SELECT count(*) FROM project_purge.receipts WHERE operation_id='${request.operationId}'`),'0');
      writeFileSync(anchorPath,JSON.stringify(canonical(pending)));
      assert.notEqual(command(args['epoch-binary'],['retry-survivor'],env,undefined,`${order}-committed-pending-reader`,false).exitCode,0);
      assert.deepEqual(native(env,{action:'status',...request}).value,applied);
      assert.equal(JSON.parse(readFileSync(anchorPath,'utf8')).status,'active');
      assert.deepEqual(native(env,{action:'apply',...request,expectedScopeDigest:preview.scopeDigest,authorization:'ERASE_PROJECT_DATA'}).value,applied);
      assert.deepEqual(rows(env),after);
      check(`${order}-committed-pending-state-activates-only-from-matching-receipt-and-retry-is-idempotent`);
      renameSync(anchorPath,`${anchorPath}.fixture-hidden`);
      assert.notEqual(native(env,{action:'preview',projectId:seeded.survivorProjectId,operationId:'missing-anchor',registryPolicy:'MINIMAL_ATTESTATION'},false).exitCode,0);
      assert.equal(existsSync(anchorPath),false);
      renameSync(`${anchorPath}.fixture-hidden`,anchorPath);
      check(`${order}-missing-host-anchor-is-never-rebuilt-from-database`);
      sql(env,"UPDATE registry_epoch.current_seal SET seal_digest=repeat('f',64)");
      const tampered=native(env,{action:'preview',projectId:seeded.survivorProjectId,operationId:'tampered',registryPolicy:'MINIMAL_ATTESTATION'},false);
      assert.notEqual(tampered.exitCode,0);
      sql(env,`UPDATE registry_epoch.current_seal SET seal_digest='${applied.registrySealDigest}'`);
      check(`${order}-database-only-seal-substitution-rejected`);
      const secondRequest={projectId:seeded.survivorProjectId,operationId:`second-${order}`,registryPolicy:'MINIMAL_ATTESTATION'};
      const second=native(env,{action:'preview',...secondRequest}).value;
      assert.equal(second.status,'READY',JSON.stringify(second.blockers));
      const secondApplied=native(env,{action:'apply',...secondRequest,expectedScopeDigest:second.scopeDigest,authorization:'ERASE_PROJECT_DATA'}).value;
      assert.equal(secondApplied.history.epoch,2);
      assert.equal(sql(env,'SELECT count(*) FROM control.project_registry_projects'),'0');
      assert.equal(sql(env,"SELECT count(*) FROM registry_epoch.used_commands WHERE disposition='ARCHIVED'"),'0');
      assert.equal(sql(env,"SELECT count(*) FROM registry_epoch.used_commands WHERE disposition='REDACTED'"),order==='survivor-reference'?'3':'2');
      assert.deepEqual(selectRows(rows(env),[seeded.survivorProjectId,seeded.survivorCanonicalPath]),[]);
      check(`${order}-second-epoch-erases-prior-baseline-content-and-carries-command-tombstones`);
      assert.equal(JSON.parse(command(args['epoch-binary'],['redacted'],env,undefined,`${order}-second-redacted-id`).stdout).status,'REDACTED_ID_REJECTED');
      assert.deepEqual(JSON.parse(command(args['epoch-binary'],['new-tail'],env,undefined,`${order}-new-tail`).stdout),{status:'NEW_TAIL_VERIFIED',epoch:2,ordinal:1});
      check(`${order}-redacted-id-blocks-changed-payload-across-epochs-and-new-id-reuses-released-identity`);
    }
  }

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

  if (args.scenario === 'survivor-reference') {
  const { env, seeded } = establish('survivor-reference');
  const request = { projectId: seeded.targetProjectId, operationId: 'fixture-survivor-reference' };
  assert.equal(sql(env, "SELECT denial_existing_project_id FROM control.project_registry_commands WHERE command_id='fixture-duplicate-survivor'"), seeded.targetProjectId);
  const before = rows(env, { includeMaintenance: true });
  const preview = native(env, { action: 'preview', ...request }).value;
  assert.equal(preview.status, 'BLOCKED');
  assert.ok(preview.blockers.some(item => item.code === 'REGISTRY_INTERLEAVED_HISTORY_REQUIRES_MIGRATION'
    && item.detail === 'REGISTRY_COMPACTION_TRUST_ROOT_REQUIRED'));
  for (const table of ['control.project_registry_commands', 'control.project_registry_observations']) {
    const blocker = preview.blockers.find(item => item.code === 'REGISTRY_SURVIVOR_REFERENCE' && item.table === table);
    assert.equal(blocker?.count, 1);
    assert.deepEqual(Object.keys(blocker).sort(), ['code', 'count', 'table']);
  }
  assert.deepEqual(rows(env, { includeMaintenance: true }), before);
  check('real-registry-denied-command-retains-target-id-and-owned-observation-and-preview-reports-counts-only');
  const applied = native(env, { action: 'apply', ...request, expectedScopeDigest: preview.scopeDigest, authorization: 'ERASE_PROJECT_DATA' }).value;
  assert.equal(applied.status, 'BLOCKED');
  assert.deepEqual(rows(env, { includeMaintenance: true }), before);
  assert.equal(sql(env, 'SELECT count(*) FROM project_purge.receipts'), '0');
  check('survivor-reference-apply-rejects-with-all-rows-and-receipts-unchanged');
  const replay = JSON.parse(command(args['seed-binary'], ['verify-survivor'], env, undefined, 'reference-survivor-replay').stdout);
  assert.equal(replay.status, 'VERIFIED');
  check('fresh-process-registry-replay-preserves-the-original-cross-project-denial');
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
      localStore.createInstallationReceipt({ projectId: id, component: 'synthetic', sourceCommitSha: 'a'.repeat(40), artifactPath: path.join(root, 'synthetic.exe'), artifactSha256: 'b'.repeat(64) });
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
  const plan = await previewProjectPurge({ projectId: coordinated.seeded.targetProjectId, nativeBinary: path.resolve(args.binary), databasePath: sqlitePath, statePath: path.join(coordinated.root, 'purge-state.json'), codeGraphCacheDirectory: path.join(coordinated.root, 'code-graphs'), operationId: 'fixture-coordinator', botService:null });
  assert.equal(plan.status, 'READY', JSON.stringify(plan.blockers));
  assert.equal(plan.sqlite.strategy, 'REBUILD_SURVIVORS_V1');
  writeFileSync(path.join(runRoot, 'coordinator-plan.json'), JSON.stringify(plan, null, 2) + '\n');
  const appliedCoordinator = await applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true });
  assert.equal(appliedCoordinator.status, 'SCOPED_PURGED');
  assert.equal(existsSync(coordinated.seeded.targetCanonicalPath), false);
  assert.equal(hash(readFileSync(survivorFile)), survivorFileBefore);
  assert.equal(hash(selectRows(rows(coordinated.env), coordinatorSurvivors)), survivorPgBefore);
  const verifiedCoordinator = await statusProjectPurge(plan);
  assert.equal(verifiedCoordinator.status, 'SCOPED_PURGED');
  assert.equal(verifiedCoordinator.externalCleanup, 'NOT_VERIFIED');
  const rebuiltStore = new LatticeStore(sqlitePath);
  try {
    assert.equal(rebuiltStore.countInstallationReceipts(), 1);
    assert.equal(rebuiltStore.listInstallationReceipts()[0].project_id, coordinated.seeded.survivorProjectId);
    assert.throws(() => rebuiltStore.database.exec('DELETE FROM installation_receipts'), /append-only/);
  } finally { rebuiltStore.close(); }
  check('real-postgres-coordinator-rebuilds-owned-installation-receipts-with-unchanged-protection');
  writeFileSync(path.join(runRoot, 'coordinator-result.json'), JSON.stringify(verifiedCoordinator, null, 2) + '\n');
  check('real-coordinator-pg-sqlite-files-readback-and-survivor-preserved', { survivorPgBefore, survivorFileBefore });
  }

  if (args.scenario === 'coordinator-absent') {
  const coordinated = establish('coordinator-absent');
  const codeGraphCacheDirectory = path.join(coordinated.root, 'code-graphs');
  const graphCache = (projectId, sourceRoot) => {
    const graph = { schema_version: 'lattice.control.code-graph.v1', source: 'GRAPHIFY', authority: 'DERIVED',
      commit: '1'.repeat(40), nodes: [], edges: [], project_id: projectId, source_root: sourceRoot };
    const key = createHash('sha256').update(`${projectId}\0${sourceRoot.toLocaleLowerCase()}`).digest('hex');
    const root = path.join(codeGraphCacheDirectory, key);
    mkdirSync(path.join(root, 'runs'), { recursive: true });
    writeFileSync(path.join(root, 'graph.json'), JSON.stringify({ cache_digest: hash(graph), ...graph }));
    writeFileSync(path.join(root, 'runs', 'analysis.txt'), `Synthetic derived content for ${projectId}`);
    return root;
  };
  const targetCaches = [coordinated.seeded.targetCanonicalPath, path.join(coordinated.root, 'other-checkout')]
    .map(root => graphCache(coordinated.seeded.targetProjectId, root));
  const survivorCache = graphCache(coordinated.seeded.survivorProjectId, coordinated.seeded.survivorCanonicalPath);
  const survivorCacheBefore = hash(readFileSync(path.join(survivorCache, 'graph.json')));
  const runtimeGraphWorkDirectory=path.join(coordinated.root,'runtime-graph');
  const runtimeRoots={};
  for(const kind of ['target','survivor']) {
    const preview=native(coordinated.env,{action:'preview',projectId:coordinated.seeded[kind+'ProjectId'],operationId:'fixture-source-key-'+kind}).value;
    assert.equal(preview.runtimeGraphSource.binding,'REGISTRY_CANONICAL_PATH');
    assert.equal(preview.relatedStores.runtimeGraph.discovery,'VERIFIED_EMPTY');
    const graphRoot=path.join(runtimeGraphWorkDirectory,'sources',preview.runtimeGraphSource.sourceKey);
    const snapshot=path.join(graphRoot,'1'.repeat(40),'snapshots','synthetic');
    mkdirSync(path.join(snapshot,'.git'),{recursive:true});
    writeFileSync(path.join(snapshot,'source.txt'),'synthetic-'+kind+'-graph-source');
    runtimeRoots[kind]={graphRoot,snapshot};
  }
  const { LatticeStore } = await import('../apps/lattice-control/src/store.mjs');
  const { DatabaseSync } = await import('node:sqlite');
  const sqlitePath = path.join(coordinated.root, 'control-fixture.sqlite');
  const localStore = new LatticeStore(sqlitePath);
  const timestamp = '2026-10-10T00:00:00Z';
  try {
    const id = coordinated.seeded.survivorProjectId, root = coordinated.seeded.survivorCanonicalPath;
    localStore.database.prepare('INSERT INTO projects(id,name,root_path,created_at,updated_at) VALUES(?,?,?,?,?)')
      .run(id, 'Synthetic survivor', root, timestamp, timestamp);
    localStore.createWorkItem({ projectId: id, title: 'Retained work', objective: 'Preserve this synthetic survivor' });
    localStore.database.prepare('INSERT INTO project_registration_details(project_id,canonical_path,registered_at,refreshed_at) VALUES (?,?,?,?)')
      .run(id, root, timestamp, timestamp);
    // Registration claims have no project FK. Simulate a catalog entry already
    // removed while both its PostgreSQL truth and this path claim remain.
    for (const kind of ['target', 'survivor']) {
      localStore.database.prepare('INSERT INTO project_registration_claims(canonical_path,generation,claimed_at) VALUES (?,1,?)')
        .run(coordinated.seeded[`${kind}CanonicalPath`], timestamp);
    }
    assert.equal(localStore.getProject(coordinated.seeded.targetProjectId), null);
  } finally { localStore.close(); }
  const sqliteRows = () => {
    const db = new DatabaseSync(sqlitePath, { readOnly: true });
    try {
      assert.equal(db.prepare('PRAGMA foreign_key_check').all().length, 0);
      return db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all().map(({ name }) => {
        assert.match(name, /^[a-z_]+$/);
        return [name, db.prepare(`SELECT * FROM "${name}"`).all().sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b), 'en'))];
      });
    } finally { db.close(); }
  };
  const cliPath = fileURLToPath(new URL('../apps/lattice-control/src/project-client.mjs', import.meta.url));
  const configPath = path.join(coordinated.root, 'purge-config.json'), planPath = path.join(coordinated.root, 'purge-plan.json');
  writeFileSync(configPath, JSON.stringify({
    projectId: coordinated.seeded.targetProjectId, operationId: 'fixture-coordinator-absent',
    nativeBinary: path.resolve(args.binary), databasePath: sqlitePath, statePath: path.join(coordinated.root, 'purge-state.json'),
    codeGraphCacheDirectory, runtimeGraphWorkDirectory, botService:null,
    externalResources: [{ kind: 'codex', reference: 'thread:synthetic-unverified-reference', source: 'synthetic-fixture' }],
  }, null, 2) + '\n');
  const cli = (alias, action, flags, expectedCode = 0) => {
    const result = command(process.execPath, [cliPath, alias, action, ...flags], coordinated.env, undefined, `cli-${alias}-${action}`, expectedCode === 0);
    assert.equal(result.exitCode, expectedCode);
    return JSON.parse((expectedCode === 0 ? result.stdout : result.stderr).trim());
  };
  const survivorFile = path.join(coordinated.seeded.survivorCanonicalPath, 'synthetic.txt');
  const survivorFileBefore = hash(readFileSync(survivorFile));
  const postgresBefore = rows(coordinated.env), sqliteBefore = sqliteRows();
  const targetNeedles = ['targetProjectId', 'targetStreamId', 'targetTaskRef', 'targetCanonicalPath'].map(key => coordinated.seeded[key]);
  const survivorNeedles = ['survivorProjectId', 'survivorStreamId', 'survivorTaskRef', 'survivorCanonicalPath'].map(key => coordinated.seeded[key]);
  const survivorPgBefore = hash(selectRows(postgresBefore, survivorNeedles));
  const survivorSqliteBefore = hash(sqliteBefore.map(([table, values]) => [table,
    table === 'project_registration_claims' ? values.filter(row => row.canonical_path !== coordinated.seeded.targetCanonicalPath) : values]));
  const inventory = cli('delete', 'inventory', ['--input', configPath, '--plan', planPath]);
  const plan = JSON.parse(readFileSync(planPath, 'utf8'));
  assert.equal(inventory.status, 'READY', JSON.stringify(inventory.blockers));
  assert.equal(plan.sqlite.catalogState, 'ABSENT');
  assert.equal(plan.sqlite.authoritativeProject.projectId, coordinated.seeded.targetProjectId);
  assert.equal(plan.sqlite.authoritativeProject.canonicalPath, coordinated.seeded.targetCanonicalPath);
  assert.equal(plan.sqlite.authoritativeProject.scopeDigest, plan.postgres.scopeDigest);
  assert.equal(plan.sqlite.counts.project_registration_claims, 1);
  assert.equal(plan.codeGraph.discovery, 'COMPLETE');
  assert.deepEqual([...plan.codeGraph.roots].sort(), [...targetCaches].sort());
  assert.equal(hash(rows(coordinated.env)), hash(postgresBefore));
  assert.equal(hash(sqliteRows()), hash(sqliteBefore));
  assert.ok(existsSync(coordinated.seeded.targetCanonicalPath));
  check('standard-cli-inventory-binds-absent-catalog-to-live-postgres-without-mutation');

  const resumed = cli('purge', 'resume', ['--plan', planPath, '--confirm', plan.digest, '--maintenance-offline']);
  assert.equal(resumed.status, 'SCOPED_PURGED');
  assert.equal(resumed.report.overallStatus, 'PARTIAL');
  assert.equal(resumed.report.complete, false);
  assert.equal(resumed.report.stages.find(stage => stage.kind === 'sqlite').initialCatalog, 'ABSENT');
  assert.equal(existsSync(coordinated.seeded.targetCanonicalPath), false);
  assert.equal(hash(readFileSync(survivorFile)), survivorFileBefore);
  assert.ok(targetCaches.every(root => !existsSync(root)));
  assert.equal(existsSync(runtimeRoots.target.graphRoot),false);
  assert.equal(readFileSync(path.join(runtimeRoots.survivor.snapshot,'source.txt'),'utf8'),'synthetic-survivor-graph-source');
  assert.equal(resumed.runtimeGraph.complete,true);
  check('native-runtime-source-hash-binds-cache-cleanup-and-survivor-remains-byte-identical');
  assert.equal(hash(readFileSync(path.join(survivorCache, 'graph.json'))), survivorCacheBefore);
  assert.equal(resumed.report.stages.find(stage => stage.kind === 'controlCodeGraph').status, 'VERIFIED_ABSENT');
  check('standard-cli-clears-multiple-owned-control-caches-and-preserves-survivor-cache', { survivorCacheBefore });
  const postgresAfter = rows(coordinated.env), sqliteAfter = sqliteRows();
  assert.equal(selectRows(postgresAfter, targetNeedles).length, 0);
  assert.equal(hash(selectRows(postgresAfter, survivorNeedles)), survivorPgBefore);
  assert.equal(hash(sqliteAfter), survivorSqliteBefore);
  assert.equal(sqliteAfter.find(([name]) => name === 'project_registration_claims')[1].length, 1);
  check('standard-cli-resume-clears-live-postgres-orphan-claim-and-files-with-survivors-unchanged', {
    survivorPgBefore, survivorSqliteBefore, survivorFileBefore,
  });

  const verified = cli('delete', 'verify', ['--plan', planPath], 2);
  assert.equal(verified.status, 'SCOPED_PURGED');
  assert.equal(verified.report.overallStatus, 'PARTIAL');
  assert.equal(verified.report.complete, false);
  assert.equal(verified.externalCleanup, 'NOT_VERIFIED');
  assert.equal(verified.report.external.find(group => group.kind === 'codex').resources[0].status, 'NOT_VERIFIED');
  assert.ok(verified.report.remaining.length > 0);
  assert.ok(verified.report.stages.every(stage => ['VERIFIED_ERASED', 'VERIFIED_ABSENT'].includes(stage.status)));
  writeFileSync(path.join(runRoot, 'coordinator-absent-result.json'), JSON.stringify(verified, null, 2) + '\n');
  check('standard-cli-verify-exits-two-with-partial-report-and-unverified-external-scope');

  const replay = JSON.parse(command(args['seed-binary'], ['verify-survivor'], coordinated.env, undefined, 'absent-survivor-replay').stdout);
  assert.equal(replay.status, 'VERIFIED');
  assert.equal(replay.survivorTaskRef, coordinated.seeded.survivorTaskRef);
  check('fresh-process-survivor-registry-and-task-replay-after-absent-catalog-purge');
  const retried = cli('delete', 'resume', ['--plan', planPath, '--confirm', plan.digest, '--maintenance-offline']);
  assert.equal(retried.status, 'SCOPED_PURGED');
  assert.equal(retried.report.overallStatus, 'PARTIAL');
  assert.equal(hash(rows(coordinated.env)), hash(postgresAfter));
  assert.equal(hash(sqliteRows()), hash(sqliteAfter));
  assert.equal(hash(readFileSync(survivorFile)), survivorFileBefore);
  check('standard-cli-resume-retry-preserves-verified-survivor-state');
  assert.deepEqual(sourceHashes(), evidence.sourceHashes);
  evidence.capturedNodeSourcesUnchanged = true;
  check('same-node-source-hashes-through-entire-cli-acceptance');
  }
  assert.deepEqual(sourceHashes(), evidence.sourceHashes);
  evidence.capturedNodeSourcesUnchanged = true;
  evidence.status = 'PASS';
} catch (error) {
  evidence.status = 'FAIL'; evidence.error = redact(error.stack); process.exitCode = 1;
} finally {
  evidence.finishedAt = new Date().toISOString();
  writeFileSync(path.join(runRoot, 'scenario-results.json'), JSON.stringify(evidence, null, 2) + '\n');
  console.log(JSON.stringify({ status: evidence.status, checks: evidence.checks, evidence: path.join(runRoot, 'scenario-results.json'), error: evidence.error }));
}
