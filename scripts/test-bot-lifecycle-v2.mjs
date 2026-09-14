// Real isolated PostgreSQL fixture; all native identities/receipts are synthetic.
// No native Codex effects and no shared production lifecycle role mutations.
import {spawn, spawnSync} from 'node:child_process';
import {randomUUID,createHash} from 'node:crypto';
import {readFileSync,writeFileSync} from 'node:fs';
import {resolve,isAbsolute} from 'node:path';
import {homedir} from 'node:os';
import assert from 'node:assert/strict';
import {loadLatticeRuntimeConfiguration,closedChildEnvironment} from '../apps/lattice-control/src/lattice-runtime-health.mjs';
const binary=process.env.LATTICE_BOT_LIFECYCLE_TEST_BINARY;
assert.ok(binary&&isAbsolute(binary),'explicit candidate binary required');
const {environment}=await loadLatticeRuntimeConfiguration(), env=closedChildEnvironment(environment);
const marker=JSON.parse(readFileSync(resolve(homedir(),'AppData/Local/LATTICE/bot-lifecycle-postgres/v1/identity.json')));
assert.equal(marker.initialized,true);assert.equal(marker.host,'127.0.0.1');
assert.notEqual(String(marker.port),environment.LATTICE_TASK019_PORT);
const runId=randomUUID().replaceAll('-',''),port=String(marker.port),project='control-v2-fixture';
const old=randomUUID(),next=randomUUID(),executor=randomUUID(),observer=randomUUID(),turn=randomUUID(),nextTurn=randomUUID();
const hash='a'.repeat(64),newHash='b'.repeat(64),checks=[];
let state;
const receipt=(tool,target=old)=>({tool,target_thread_id:target,target_host_id:'local',result_digest:hash,readback_digest:hash,evidence_ref:'fixture:synthetic-only',success:true,readback_verified:true,old_pending_count:0});
async function cli(p,command='bot-lifecycle',selectedRunId=runId) {
 return new Promise((done,fail)=>{
 const child=spawn(binary,[command,'--postgres-host','127.0.0.1','--postgres-port',port,'--postgres-run-id',selectedRunId],{env,windowsHide:true,stdio:['pipe','pipe','pipe']});
 let stdout='',stderr='';child.stdout.on('data',x=>stdout+=x);child.stderr.on('data',x=>stderr+=x);child.on('error',fail);
 child.on('close',code=>done({code,stderr,stdout,value:code===0?JSON.parse(stdout):null}));child.stdin.on('error',()=>{});child.stdin.end(p?JSON.stringify(p):'');
 });
}
const base=(action,body,overrides={})=>({action,request_id:randomUUID(),project_id:project,role_id:'control',expected_revision:state?.revision??0,expected_generation:state?.generation??0,owner_thread_id:state?.owner_thread_id??old,owner_host_id:'local',body,...overrides});
async function read(role='control'){const r=await cli({action:'read',project_id:project,role_id:role});assert.equal(r.code,0,r.stderr);return r.value.current;}
async function apply(p,command='bot-lifecycle',label=p.action){const r=await cli(p,command);assert.equal(r.code,0,r.stderr);state=await read();assert.deepEqual(r.value.current,state);checks.push(label);return r;}
async function reject(p,error,command='bot-lifecycle'){const before=await read(),r=await cli(p,command);assert.equal(r.code,2,r.stdout);assert.ok(r.stderr.includes(error),r.stderr);assert.deepEqual(await read(),before);checks.push('reject '+error);return r;}
const installed=await cli(null,'bot-lifecycle-install');assert.equal(installed.code,0,installed.stderr);
function sql(query,database=installed.value.database) {
 const r=spawnSync(resolve(marker.pgBin,'psql.exe'),['-X','-qAt','-w','-h','127.0.0.1','-p',port,'-U','lattice_migrator_login','-d',database,'-v','ON_ERROR_STOP=1'],{env:{...env,PGPASSWORD:environment.LATTICE_TASK019_PASSWORD},input:'SET ROLE lattice_migrator; '+query,encoding:'utf8',windowsHide:true});
 assert.equal(r.status,0,r.stderr);return r.stdout.trim();
}
const partialRun=randomUUID().replaceAll('-',''),partial=await cli(null,'bot-lifecycle-install',partialRun);
assert.equal(partial.code,0,partial.stderr);
sql("CREATE FUNCTION bot_lifecycle.unexpected_partial() RETURNS integer LANGUAGE sql AS 'SELECT 1';",partial.value.database);
const partialRead=await cli({action:'read',project_id:project,role_id:'control'},'bot-lifecycle',partialRun);
assert.equal(partialRead.code,2);assert.ok(partialRead.stderr.includes('SCHEMA_REJECTED'));checks.push('partial or unexpected catalog fails closed');
const work=Array.from({length:14},(_,i)=>'retained-work-'+i).sort();
await apply(base('register',{work_ids:work,policy_digest:hash,rules_digest:hash,binding_receipt:receipt('read_thread')}),'bot-lifecycle','register control v1');
for(const [role,id] of [['lattice_maintenance',executor],['reports',observer]]){
const r=await cli(base('register',{work_ids:['legacy-work'],policy_digest:hash,rules_digest:hash,binding_receipt:receipt('read_thread',id)},{role_id:role,expected_revision:0,expected_generation:0,owner_thread_id:id}));assert.equal(r.code,0,r.stderr);
}
const legacy=await read('reports'),original=structuredClone(state);
const historical=sql("SELECT jsonb_agg(to_jsonb(e) ORDER BY project_id,role_id,request_id)::text FROM bot_lifecycle.events e;");
const migration=base('migrate-contract',{from_version:1,to_version:2,expected_policy_digest:hash,expected_rules_digest:hash,policy_digest:newHash,rules_digest:newHash,approval_digest:hash,authorization_receipt:receipt('read_thread')});
await reject({...migration,body:{...migration.body,expected_rules_digest:'c'.repeat(64)}},'CONTRACT_MIGRATION_REJECTED','bot-lifecycle-migrate');
assert.equal((await cli(null,'bot-lifecycle-install')).code,0);checks.push('failed migration rolls back extension to verified v1');
await reject({...migration,extra:true},'INPUT_REJECTED','bot-lifecycle-migrate');
await reject({...migration,expected_revision:0},'REVISION_CONFLICT','bot-lifecycle-migrate');
const migrated=await apply(migration,'bot-lifecycle-migrate');
const verifiedInstallation=await cli(null,'bot-lifecycle-install');
assert.equal(verifiedInstallation.value.schemaVersion,2);assert.equal(verifiedInstallation.value.status,'VERIFIED');checks.push('explicit verifier reports actual installed schema version');
assert.equal(state.generation,original.generation);assert.equal(state.owner_thread_id,original.owner_thread_id);assert.deepEqual(state.work_ids,work);assert.equal(state.revision,2);assert.equal(state.contract_version,2);
assert.deepEqual(await read('reports'),legacy);checks.push('migration preserves owner generation work and legacy role');
const historyAfter=JSON.parse(sql("SELECT jsonb_agg(to_jsonb(e) ORDER BY project_id,role_id,request_id)::text FROM bot_lifecycle.events e;"));
for(const event of JSON.parse(historical))assert.deepEqual(historyAfter.find(e=>e.project_id===event.project_id&&e.role_id===event.role_id&&e.request_id===event.request_id),event);
checks.push('all pre-migration event bytes semantically unchanged');
assert.equal((await cli(migration,'bot-lifecycle-migrate')).value.status,'REPLAYED');
await reject({...migration,body:{...migration.body,approval_digest:newHash}},'IDEMPOTENCY_CONFLICT','bot-lifecycle-migrate');
await reject(base('migrate-contract',migration.body),'CONTRACT_MIGRATION_REJECTED','bot-lifecycle-migrate');
const legacyWrite=await cli(base('add-work',{work_ids:['legacy-new']},{role_id:'reports',expected_revision:legacy.revision,expected_generation:legacy.generation,owner_thread_id:observer}));assert.equal(legacyWrite.code,0,legacyWrite.stderr);
const legacyGuard=await cli({action:'assert-owner',project_id:project,role_id:'reports',expected_generation:1,owner_thread_id:observer,owner_host_id:'local'});assert.equal(legacyGuard.value.status,'OWNER_CURRENT');checks.push('v1 role mutation and ordinary guard work after schema migration');
const direct=spawnSync(resolve(marker.pgBin,'psql.exe'),['-X','-h','127.0.0.1','-p',port,'-U','lattice_runtime_login','-d',installed.value.database,'-v','ON_ERROR_STOP=1'],{env:{...env,PGPASSWORD:environment.LATTICE_TASK019_PASSWORD},input:"SET ROLE lattice_runtime; SELECT bot_lifecycle.apply_v1('{}'::jsonb);",encoding:'utf8',windowsHide:true});
assert.notEqual(direct.status,0);assert.ok(direct.stderr.includes('permission denied for function apply_v1'),direct.stderr);checks.push('runtime cannot bypass v2 via direct apply_v1');
const handoff='same-original-work-id',controlActor=()=>({role_id:'control',thread_id:state.owner_thread_id,host_id:'local',generation:state.generation});
const executorActor={role_id:'lattice_maintenance',thread_id:executor,host_id:'local',generation:1};
let oldUpdatedAt=1788825600;
const boundary=(id=old,latest=turn)=>({source:'codex.read_thread',observed_at:new Date().toISOString(),thread_id:id,host_id:'local',latest_turn_id:latest,thread_updated_at:oldUpdatedAt,status:'idle',latest_turn_status:'completed',pending_input_count:0,in_flight_count:0,readback_digest:hash,evidence_ref:'fixture:synthetic-only'});
const grantBody=()=>({executor_role_id:'lattice_maintenance',executor_thread_id:executor,executor_host_id:'local',executor_generation:1,checkpoint_turn_id:turn,expires_at:new Date(Date.now()+3600000).toISOString(),authorization_receipt:receipt('read_thread')});
const v2=(action,body,actor=executorActor,native={old:boundary(),new:null})=>({...base(action,body),actor,handoff_id:handoff,native});
const grant=()=>v2('authorize-executor',grantBody(),controlActor(),{old:null,new:null});
const manifest=()=>({work_ids:state.work_ids,pending_decision_ids:[],pending_input_ids:[],in_flight_ids:[],...Object.fromEntries(['requirements','authorization','git_state','recovery','evidence','next_steps'].map(k=>[k+'_digest',hash])),rules_digest:newHash});
await reject(base('add-work',{work_ids:['bypass']}),'INPUT_REJECTED');
await reject(v2('prepare',{handoff_id:handoff,manifest:manifest()}),'EXECUTOR_GRANT_REJECTED');
await reject({...grant(),actor:{role_id:'reports',thread_id:observer,host_id:'local',generation:1}},'ACTOR_REJECTED');
await reject({...grant(),body:{...grantBody(),expires_at:new Date(Date.now()-1000).toISOString()}},'EXECUTOR_GRANT_REJECTED');
await reject({...grant(),body:{...grantBody(),executor_generation:99}},'ACTOR_REJECTED');
await apply({...grant(),body:{...grantBody(),expires_at:new Date(Date.now()+2000).toISOString()}});
await new Promise(resolve=>setTimeout(resolve,2200));
await reject(v2('add-work',{work_ids:['expired']}),'EXECUTOR_GRANT_REJECTED');
await apply(v2('revoke-executor',{reason_digest:hash},controlActor(),{old:null,new:null}));
await apply(grant());
await reject(grant(),'EXECUTOR_GRANT_REJECTED');
await reject(v2('add-work',{work_ids:['extra']},{...executorActor,generation:2}),'ACTOR_REJECTED');
await reject({...v2('add-work',{work_ids:['extra']}),handoff_id:'other-handoff'},'EXECUTOR_GRANT_REJECTED');
await reject(v2('reserve-step',{step:'routing_updated',operation_key:'no',input_digest:hash}),'EXECUTOR_GRANT_REJECTED');
for(const change of [{status:'active'},{status:'unknown'},{latest_turn_status:'inProgress'},{pending_input_count:null},{in_flight_count:1},{latest_turn_id:randomUUID()},{observed_at:new Date(Date.now()-301000).toISOString()}]){
await reject(v2('prepare',{handoff_id:handoff,manifest:manifest()},executorActor,{old:{...boundary(),...change},new:null}),'NATIVE_BOUNDARY_REJECTED');
}
await apply(v2('revoke-executor',{reason_digest:hash},controlActor(),{old:null,new:null}));
await reject(v2('add-work',{work_ids:['extra']}),'EXECUTOR_GRANT_REJECTED');
await apply(grant());
// Hold a real executor-authorized transaction open. Its actor row must remain
// locked until commit, preventing concurrent actor changes from overtaking CAS.
const heldRequest=v2('add-work',{work_ids:[]});
const session=spawn(resolve(marker.pgBin,'psql.exe'),['-X','-qAt','-w','-h','127.0.0.1','-p',port,'-U','lattice_runtime_login','-d',installed.value.database,'-v','ON_ERROR_STOP=1'],{env:{...env,PGPASSWORD:environment.LATTICE_TASK019_PASSWORD},windowsHide:true,stdio:['pipe','pipe','pipe']});
let heldOut='',heldErr='';
const heldDone=new Promise(resolve=>session.on('close',code=>resolve(code)));
session.stderr.on('data',x=>heldErr+=x);
const heldReady=new Promise((resolve,reject)=>{
const timer=setTimeout(()=>reject(Error('actor lock marker timeout')),10000);
session.stdout.on('data',x=>{heldOut+=x;if(heldOut.includes('ACTOR_LOCK_HELD')){clearTimeout(timer);resolve();}});
});
session.stdin.write("BEGIN ISOLATION LEVEL SERIALIZABLE; SET ROLE lattice_runtime; SELECT bot_lifecycle.apply_v2('"+JSON.stringify(heldRequest).replaceAll("'","''")+"'); SELECT pg_backend_pid(); SELECT 'ACTOR_LOCK_HELD';\n");
await heldReady;
const actorState=await read('lattice_maintenance');
let actorChangeDone=false;
const actorChange=cli(base('add-work',{work_ids:['actor-change']},{role_id:'lattice_maintenance',expected_revision:actorState.revision,expected_generation:actorState.generation,owner_thread_id:executor})).then(r=>{actorChangeDone=true;return r;});
let blocked=false;
for(let i=0;i<30;i++){
 if(sql("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND cardinality(pg_blocking_pids(pid))>0;")!=='0'){blocked=true;break;}
 await new Promise(resolve=>setTimeout(resolve,50));
}
assert.equal(blocked,true,'actor replacement must block behind delegated transaction');assert.equal(actorChangeDone,false);
session.stdin.end('COMMIT;\n');assert.equal(await heldDone,0,heldErr);
const actorChanged=await actorChange;assert.equal(actorChanged.code,0,actorChanged.stderr);state=await read();
checks.push('actor row ownership held through delegated transaction commit');
await apply(v2('add-work',{work_ids:['new-retained-work']}));
assert.equal(state.work_ids.length,15);
await reject(v2('prepare',{handoff_id:handoff,manifest:{...manifest(),work_ids:work}}),'WORK_SET_MISMATCH');
await reject(v2('prepare',{handoff_id:handoff,manifest:{...manifest(),pending_input_ids:['unknown']}}),'NATIVE_BOUNDARY_REJECTED');
await apply(v2('prepare',{handoff_id:handoff,manifest:manifest()}));
await apply(v2('abort',{reason_digest:hash,candidate_disposition_receipt:null},controlActor(),{old:null,new:null}));
assert.equal(state.phase,'ACTIVE');assert.equal(state.contract_version,2);assert.deepEqual(state.contract_migration,migration.body);assert.equal(state.executor_grant.status,'REVOKED');
checks.push('pre-CAS abort retains contract and revokes executor');
await apply(grant());
await apply(v2('prepare',{handoff_id:handoff,manifest:manifest()}));
const ordinary=()=>({action:'assert-owner',project_id:project,role_id:'control',expected_generation:state.generation,owner_thread_id:state.owner_thread_id,owner_host_id:'local'});
await reject(ordinary(),'ADMISSION_PAUSED');
await reject(v2('revoke-executor',{reason_digest:hash},controlActor(),{old:null,new:null}),'EXECUTOR_GRANT_REJECTED');
const reserved=v2('reserve-step',{step:'successor_created',operation_key:'create-one',input_digest:hash});
await apply(reserved);
assert.equal((await cli(reserved)).value.status,'REPLAYED');checks.push('exact step replay');
await reject({...reserved,body:{...reserved.body,input_digest:newHash}},'IDEMPOTENCY_CONFLICT');
await reject(v2('reserve-step',reserved.body),'STEP_ALREADY_RESERVED');
await reject(v2('abort',{reason_digest:hash,candidate_disposition_receipt:null},controlActor(),{old:null,new:null}),'ABORT_UNSAFE');
await reject(v2('native-step',{step:'successor_created',operation_key:'create-one',receipt:{...receipt('create_thread',next),readback_verified:false}}),'NATIVE_EVIDENCE_REJECTED');
await apply(v2('native-step',{step:'successor_created',operation_key:'create-one',receipt:receipt('create_thread',next)}));
const ack=()=>v2('ack',{manifest_digest:state.manifest_digest,work_ids:state.work_ids,verification_receipt:receipt('read_thread',next)},executorActor,{old:boundary(),new:boundary(next,nextTurn)});
await reject({...ack(),native:{old:boundary(),new:{...boundary(next,nextTurn),status:'active'}}},'NATIVE_BOUNDARY_REJECTED');
await apply(ack());
await reject(v2('commit',{manifest_digest:state.manifest_digest},executorActor,{old:{...boundary(),thread_updated_at:1788825601},new:boundary(next,nextTurn)}),'NATIVE_BOUNDARY_REJECTED');
await reject(v2('commit',{manifest_digest:state.manifest_digest},executorActor,{old:boundary(),new:boundary(next,randomUUID())}),'NATIVE_BOUNDARY_REJECTED');
const commit=()=>v2('commit',{manifest_digest:state.manifest_digest},executorActor,{old:boundary(),new:boundary(next,nextTurn)});
const requests=[commit(),commit()],outcomes=await Promise.all(requests.map(p=>cli(p)));
assert.equal(outcomes.filter(r=>r.code===0).length,1,JSON.stringify(outcomes));assert.equal(outcomes.filter(r=>r.code===2).length,1);state=await read();
assert.equal(state.generation,2);assert.equal(state.owner_thread_id,next);assert.equal(state.executor_grant.status,'CONSUMED');checks.push('competing CAS exactly one owner and consumed grant');
const winning=requests[outcomes.findIndex(r=>r.code===0)];assert.equal((await cli(winning)).value.status,'REPLAYED');checks.push('consumed grant historical commit replay only');
await reject(v2('finish',{}),'ACTOR_REJECTED');
await reject({...v2('finish',{},controlActor()),owner_thread_id:old,expected_generation:1},'STALE_OWNER');
await reject(ordinary(),'ADMISSION_PAUSED');
const handoffGuard={...ordinary(),action:'assert-handoff-owner',handoff_id:handoff};
assert.equal((await cli(handoffGuard)).value.status,'HANDOFF_OWNER_CURRENT');checks.push('new owner MIGRATING-only guard');
await reject({...handoffGuard,handoff_id:'other'},'ADMISSION_PAUSED');
await reject(v2('add-work',{work_ids:['not-general-work']},controlActor()),'ACTOR_REJECTED');
await reject(v2('finish',{},controlActor()),'MIGRATION_INCOMPLETE');
await reject(v2('reserve-step',{step:'old_archived',operation_key:'archive',input_digest:hash},controlActor()),'ARCHIVE_NOT_READY');
async function step(name,tool,target){
await apply(v2('reserve-step',{step:name,operation_key:name,input_digest:hash},controlActor()));
const native={old:{...boundary(),...(name==='old_archived'?{status:'notLoaded'}:{})},new:null};
await apply(v2('native-step',{step:name,operation_key:name,receipt:receipt(tool,target)},controlActor(),native));
}
await step('routing_updated','hq-routing-update',next);
await reject(v2('finish',{},controlActor()),'MIGRATION_INCOMPLETE');
await step('schedule_updated','schedule-not-configured',next);
await reject(v2('reserve-step',{step:'old_archived',operation_key:'old_archived',input_digest:hash},controlActor(),{old:{...boundary(),pending_input_count:1},new:null}),'NATIVE_BOUNDARY_REJECTED');
let archiveRecoveryRequest,archiveSqlSha256;
if(process.env.LATTICE_BOT_LIFECYCLE_ARCHIVE_RECOVERY==='1'){
 await apply(v2('reserve-step',{step:'old_archived',operation_key:'archive-recovery',input_digest:hash},controlActor()));
 const originalBoundary=structuredClone(state.handoff_boundary),retained=structuredClone(state),post={...boundary(),status:'notLoaded',thread_updated_at:oldUpdatedAt-3600};
 const body={manifest_digest:state.manifest_digest,anchor_request_id:winning.request_id,pre_native_digest:hash,archive_operation_key:'archive-recovery',archive_receipt:receipt('set_thread_archived',old),history_proof:{pre_turn_digest:hash,post_turn_digest:hash,rollout_sha256:hash,evidence_digest:hash,archived:true,archived_at:oldUpdatedAt+10,durable_updated_at:post.thread_updated_at,latest_turn_id:turn,new_input_count:0,in_flight_count:0}};
 const recovery=()=>v2('reconcile-archived-boundary',body,controlActor(),{old:post,new:null});
 await reject({...recovery(),actor:executorActor},'ACTOR_REJECTED','bot-lifecycle-reconcile-archive');
 await reject({...recovery(),expected_revision:0},'REVISION_CONFLICT','bot-lifecycle-reconcile-archive');
 for(const change of [{latest_turn_id:randomUUID()},{status:'active'},{status:'idle'},{pending_input_count:1},{in_flight_count:1},{thread_updated_at:oldUpdatedAt+1},{observed_at:new Date(Date.now()-301000).toISOString()}]){
  await reject({...recovery(),native:{old:{...post,...change},new:null}},'NATIVE_BOUNDARY_REJECTED','bot-lifecycle-reconcile-archive');
 }
 for(const change of [{post_turn_digest:newHash},{archived:false},{new_input_count:1},{in_flight_count:1},{durable_updated_at:oldUpdatedAt},{latest_turn_id:randomUUID()},{archived_at:oldUpdatedAt-1}]){
  await reject({...recovery(),body:{...body,history_proof:{...body.history_proof,...change}}},'ARCHIVE_RECONCILE_REJECTED','bot-lifecycle-reconcile-archive');
 }
 for(const change of [{manifest_digest:newHash},{anchor_request_id:'missing-anchor'},{pre_native_digest:newHash},{archive_operation_key:'other-operation'}]){
  await reject({...recovery(),body:{...body,...change}},'ARCHIVE_RECONCILE_REJECTED','bot-lifecycle-reconcile-archive');
 }
 assert.equal((await cli(null,'bot-lifecycle-install')).value.schemaVersion,2);checks.push('failed reconcile rolls back additive function installation');
 archiveRecoveryRequest=recovery();archiveSqlSha256=(await apply(archiveRecoveryRequest,'bot-lifecycle-reconcile-archive')).value.archive_sql_sha256;
 for(const key of ['owner_thread_id','generation','phase','work_ids','steps','manifest','manifest_digest','executor_grant','ack'])assert.deepEqual(state[key],retained[key]);
 assert.deepEqual(state.archive_reconciliation.original_boundary,originalBoundary);assert.equal(state.handoff_boundary.thread_updated_at,post.thread_updated_at);oldUpdatedAt=post.thread_updated_at;
 assert.equal((await cli(archiveRecoveryRequest,'bot-lifecycle-reconcile-archive')).value.status,'REPLAYED');
 await reject({...archiveRecoveryRequest,body:{...body,pre_native_digest:newHash}},'IDEMPOTENCY_CONFLICT','bot-lifecycle-reconcile-archive');
 await reject(v2('finish',{},controlActor()),'MIGRATION_INCOMPLETE');
 await apply(v2('native-step',{step:'old_archived',operation_key:'archive-recovery',receipt:receipt('set_thread_archived',old)},controlActor(),{old:{...boundary(),status:'notLoaded'},new:null}));
 checks.push('archive reconcile preserves original boundary and only resumes reserved recording');
}else await step('old_archived','set_thread_archived',old);
for(const change of [{status:'active'},{status:'unknown'},{latest_turn_status:'inProgress'},{pending_input_count:null},{in_flight_count:1},{latest_turn_id:randomUUID()},{thread_updated_at:1788825601}]){
await reject(v2('finish',{},controlActor(),{old:{...boundary(),status:'notLoaded',...change},new:null}),'NATIVE_BOUNDARY_REJECTED');
}
await apply(v2('finish',{},controlActor(),{old:{...boundary(),status:'notLoaded'},new:null}));
checks.push('post-archive notLoaded retains completed source identity through finish');
assert.equal((await cli(ordinary())).value.status,'OWNER_CURRENT');
await reject(handoffGuard,'ADMISSION_PAUSED');
assert.equal(state.phase,'ACTIVE');assert.equal(state.work_ids.length,15);assert.ok(work.every(w=>state.work_ids.includes(w)));
assert.equal(state.contract_version,2);assert.deepEqual(state.contract_migration,migration.body);assert.equal(state.executor_grant.status,'CONSUMED');
if(archiveRecoveryRequest){const before=structuredClone(state);assert.equal((await cli(archiveRecoveryRequest,'bot-lifecycle-reconcile-archive')).value.status,'REPLAYED');assert.deepEqual(await read(),before);checks.push('historical reconcile replay cannot restore migration or authority');}
const final=structuredClone(state);assert.equal((await cli(migration,'bot-lifecycle-migrate')).value.status,'REPLAYED');assert.deepEqual(await read(),final);checks.push('migration historical replay preserves completed owner');
const evidence={status:'PASS',scope:'real-isolated-postgresql-synthetic-native-receipts',runId,port:Number(port),database:installed.value.database,partialFixtureDatabase:partial.value.database,retained:true,binary, binarySha256:createHash('sha256').update(readFileSync(binary)).digest('hex'),v1SqlSha256:migrated.value.v1_sql_sha256,v2SqlSha256:migrated.value.v2_sql_sha256,archiveSqlSha256,checks,finalState:state,verifiedAt:new Date().toISOString()};
const evidencePath=resolve(marker.dataDirectory,'..','v2-acceptance-'+runId+'.json');writeFileSync(evidencePath,JSON.stringify(evidence,null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify({status:'PASS',checks:checks.length,runId,evidencePath,binarySha256:evidence.binarySha256,v2SqlSha256:evidence.v2SqlSha256}));
