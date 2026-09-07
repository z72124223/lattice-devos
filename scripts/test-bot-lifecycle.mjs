// Real PostgreSQL acceptance using synthetic native receipts. No native effects.
// Creates and retains an isolated test database on the configured local service.
import { spawn, spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {readFileSync,writeFileSync} from 'node:fs';
import {homedir} from 'node:os';
import assert from 'node:assert/strict';
import { loadLatticeRuntimeConfiguration, closedChildEnvironment } from '../apps/lattice-control/src/lattice-runtime-health.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const binary = resolve(root, 'target/release/lattice-runtime.exe');
const { environment } = await loadLatticeRuntimeConfiguration();
const env = closedChildEnvironment(environment);
const runId = randomUUID().replaceAll('-', '');
const marker=JSON.parse(readFileSync(resolve(homedir(),'AppData/Local/LATTICE/bot-lifecycle-postgres/v1/identity.json'),'utf8'));
assert.equal(marker.initialized,true); assert.equal(marker.host,'127.0.0.1');
assert.notEqual(String(marker.port),environment.LATTICE_TASK019_PORT);
const port = String(marker.port);
const old = randomUUID(), next = randomUUID(), project = 'bot-lifecycle-fixture';
const hash = 'a'.repeat(64);
let state, checks = 0;
function cli(request, install = false, selectedPort = port) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(binary, [install ? 'bot-lifecycle-install' : 'bot-lifecycle', '--postgres-host', '127.0.0.1', '--postgres-port', selectedPort, '--postgres-run-id', runId], { env, windowsHide: true, stdio: ['pipe','pipe','pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', x => stdout += x); child.stderr.on('data', x => stderr += x);
    child.on('error', reject);
    child.on('close', code => resolvePromise({code, stdout, stderr, value: code === 0 ? JSON.parse(stdout) : null}));
    child.stdin.on('error', () => {});
    child.stdin.end(install ? '' : JSON.stringify(request));
  });
}
const receipt = (tool, target = next) => ({tool, target_thread_id: target, target_host_id:'local', result_digest:hash, readback_digest:hash, evidence_ref:'fixture:synthetic-only', success:true, readback_verified:true, old_pending_count:0});
const manifest = (work = state.work_ids) => ({work_ids:work,pending_decision_ids:[],pending_input_ids:[],in_flight_ids:[],...Object.fromEntries(['requirements','authorization','git_state','recovery','evidence','next_steps','rules'].map(k=>[`${k}_digest`,hash]))});
const request = (action, body, overrides = {}) => ({action,request_id:randomUUID(),project_id:project,role_id:'reports-fixture',expected_revision:state?.revision??0,expected_generation:state?.generation??0,owner_thread_id:state?.owner_thread_id??old,owner_host_id:'local',body,...overrides});
async function read() { const r=await cli({action:'read',project_id:project,role_id:'reports-fixture'}); assert.equal(r.code,0,r.stderr); return r.value.current; }
async function apply(action, body, overrides) { const p=request(action,body,overrides), r=await cli(p); assert.equal(r.code,0,r.stderr); state=r.value.current; assert.deepEqual(await read(),state); checks++; return {p,r}; }
async function rejected(p, code) { const before=await read(), r=await cli(p); assert.equal(r.code,2,r.stdout); assert.ok(r.stderr.includes(code),r.stderr); assert.deepEqual(await read(),before); checks++; }
async function step(name, tool, target = next) { await apply('reserve-step',{step:name,operation_key:name,input_digest:hash}); await apply('native-step',{step:name,operation_key:name,receipt:receipt(tool,target)}); }
const installed=await cli(null,true); assert.equal(installed.code,0,installed.stderr);
const shared=await cli(null,true,environment.LATTICE_TASK019_PORT); assert.equal(shared.code,2); assert.ok(shared.stderr.includes('ISOLATED_CLUSTER_REQUIRED')); checks++;
const denied=spawnSync(resolve(marker.pgBin,'psql.exe'),['-X','-h','127.0.0.1','-p',port,'-U','lattice_runtime_login','-d',installed.value.database,'-v','ON_ERROR_STOP=1'],{env:{...env,PGPASSWORD:environment.LATTICE_TASK019_PASSWORD},input:"BEGIN; SET ROLE lattice_runtime; UPDATE bot_lifecycle.roles SET state='{}'::jsonb; ROLLBACK;",windowsHide:true,encoding:'utf8'});
assert.notEqual(denied.status,0); assert.ok(denied.stderr.includes('permission denied for table roles'),denied.stderr); checks++;
assert.equal((await cli(null,true)).value.status,'VERIFIED'); checks++;
assert.equal(await read(),null); checks++;
await apply('register',{work_ids:['work-a','work-b'],policy_digest:hash,rules_digest:hash,binding_receipt:receipt('read_thread',old)});
await rejected(request('checkpoint',{manifest:manifest()},{expected_revision:0}),'REVISION_CONFLICT');
await rejected(request('unknown-action',{}),'INPUT_REJECTED');
await rejected({...request('checkpoint',{manifest:manifest()}),extra:'ignored-field'},'INPUT_REJECTED');
await rejected({...request('checkpoint',{manifest:manifest()}),extra:'x'.repeat(70000)},'INPUT_REJECTED');
await rejected(request('commit',{manifest_digest:hash}),'UNVERIFIED_SWITCH');
await rejected(request('checkpoint',{manifest:manifest(['work-a'])}),'WORK_SET_MISMATCH');
await rejected(request('checkpoint',{manifest:{...manifest(),rules_digest:'b'.repeat(64)}}),'RULES_MISMATCH');
await apply('prepare',{handoff_id:'handoff-one',manifest:manifest()});
await rejected({action:'assert-owner',project_id:project,role_id:'reports-fixture',expected_generation:1,owner_thread_id:old,owner_host_id:'local'},'ADMISSION_PAUSED');
const reserved=await apply('reserve-step',{step:'successor_created',operation_key:'create-one',input_digest:hash});
const replay=await cli(reserved.p); assert.equal(replay.value.status,'REPLAYED'); assert.deepEqual(await read(),state); checks++;
await rejected({...reserved.p,body:{...reserved.p.body,input_digest:'b'.repeat(64)}},'IDEMPOTENCY_CONFLICT');
await rejected(request('reserve-step',reserved.p.body),'STEP_ALREADY_RESERVED');
await rejected(request('abort',{reason_digest:hash,candidate_disposition_receipt:null}),'ABORT_UNSAFE');
await rejected(request('native-step',{step:'successor_created',operation_key:'create-one',receipt:{...receipt('create_thread'),readback_verified:false}}),'NATIVE_EVIDENCE_REJECTED');
await apply('native-step',{step:'successor_created',operation_key:'create-one',receipt:receipt('create_thread')});
await rejected(request('ack',{manifest_digest:state.manifest_digest,work_ids:['work-a'],verification_receipt:receipt('read_thread')}),'ACK_REJECTED');
await rejected(request('ack',{manifest_digest:state.manifest_digest,work_ids:state.work_ids,verification_receipt:receipt('read_thread',old)}),'ACK_REJECTED');
await apply('ack',{manifest_digest:state.manifest_digest,work_ids:state.work_ids,verification_receipt:receipt('read_thread')});
await apply('add-work',{work_ids:['work-c']}); assert.equal(state.phase,'PREPARING'); assert.equal(state.ack,null); checks++;
await rejected(request('commit',{manifest_digest:hash}),'UNVERIFIED_SWITCH');
await apply('checkpoint',{manifest:manifest()});
await apply('ack',{manifest_digest:state.manifest_digest,work_ids:state.work_ids,verification_receipt:receipt('read_thread')});
const competing=[request('commit',{manifest_digest:state.manifest_digest}),request('commit',{manifest_digest:state.manifest_digest})];
const outcomes=await Promise.all(competing.map(p=>cli(p))); assert.equal(outcomes.filter(r=>r.code===0).length,1); assert.equal(outcomes.filter(r=>r.code===2).length,1); state=await read(); assert.equal(state.generation,2); assert.equal(state.owner_thread_id,next); checks++;
const winning=competing[outcomes.findIndex(r=>r.code===0)]; assert.equal((await cli(winning)).value.status,'REPLAYED'); assert.deepEqual(await read(),state); checks++;
await rejected(request('add-work',{work_ids:['stale-work']},{expected_generation:1,owner_thread_id:old}),'STALE_OWNER');
await rejected(request('finish',{}),'MIGRATION_INCOMPLETE');
await rejected(request('abort',{reason_digest:hash,candidate_disposition_receipt:receipt('set_thread_archived')}),'ABORT_UNSAFE');
await rejected(request('reserve-step',{step:'old_archived',operation_key:'archive',input_digest:hash}),'ARCHIVE_NOT_READY');
await step('routing_updated','hq-routing-update');
await rejected(request('finish',{}),'MIGRATION_INCOMPLETE');
await step('schedule_updated','schedule-not-configured');
await apply('checkpoint',{manifest:{...manifest(),pending_input_ids:['late-input']}});
await rejected(request('reserve-step',{step:'old_archived',operation_key:'archive',input_digest:hash}),'ARCHIVE_NOT_READY');
await apply('checkpoint',{manifest:manifest()});
await step('old_archived','set_thread_archived',old);
await rejected(request('add-work',{work_ids:['too-late']}),'ARCHIVE_NOT_READY');
await rejected(request('checkpoint',{manifest:{...manifest(),pending_input_ids:['too-late']}}),'ARCHIVE_NOT_READY');
await apply('finish',{}); assert.equal(state.phase,'ACTIVE'); assert.deepEqual(state.work_ids,['work-a','work-b','work-c']); checks++;
await apply('prepare',{handoff_id:'handoff-abort-empty',manifest:manifest()});
await apply('abort',{reason_digest:hash,candidate_disposition_receipt:null}); assert.equal(state.generation,2); assert.equal(state.owner_thread_id,next); checks++;
await apply('prepare',{handoff_id:'handoff-abort-candidate',manifest:manifest()});
await apply('reserve-step',{step:'successor_created',operation_key:'candidate-abort',input_digest:hash});
await apply('native-step',{step:'successor_created',operation_key:'candidate-abort',receipt:receipt('create_thread',old)});
await rejected(request('abort',{reason_digest:hash,candidate_disposition_receipt:receipt('set_thread_archived',next)}),'ABORT_UNSAFE');
await apply('abort',{reason_digest:hash,candidate_disposition_receipt:receipt('set_thread_archived',old)}); assert.equal(state.generation,2); assert.equal(state.owner_thread_id,next); checks++;
const evidence={status:'PASS',checks,runId,port:Number(port),database:installed.value.database,scope:'real-postgresql-synthetic-native-receipts',retained:true,finalState:state.phase,generation:state.generation,verifiedAt:new Date().toISOString()};
writeFileSync(resolve(marker.dataDirectory,'..',`acceptance-${runId}.json`),JSON.stringify(evidence,null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify(evidence));
