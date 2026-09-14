import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {createInterface} from 'node:readline';
import {DatabaseSync} from 'node:sqlite';
import {homedir} from 'node:os';
import {pathToFileURL,fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
import {loadLatticeRuntimeConfiguration,closedChildEnvironment} from '../apps/lattice-control/src/lattice-runtime-health.mjs';
const canonical=v=>JSON.stringify(v,(_,x)=>x&&!Array.isArray(x)&&typeof x==='object'?Object.fromEntries(Object.entries(x).sort(([a],[b])=>a.localeCompare(b))):x);
export const digest=v=>crypto.createHash('sha256').update(typeof v==='string'||Buffer.isBuffer(v)?v:canonical(v)).digest('hex');
const requireThat=(ok,code)=>{if(!ok)throw Error(code);};
const read=f=>JSON.parse(fs.readFileSync(f,'utf8').replace(/^\uFEFF/,''));
// Decode the native display format only; no shell evaluation or execution.
export function commandVector(display){
 requireThat(typeof display==='string','COMMAND_DISPLAY_REJECTED');
 const values=[];let token='',quote=null,started=false;
 for(let index=0;index<display.length;index++){
  const c=display[index];
  if(c===quote){quote=null;continue;}
  if(quote==="'"){token+=c;continue;}
  if(!quote&&(c==='"'||c==="'")){quote=c;started=true;continue;}
  if(c==='\\'){
   requireThat(index+1<display.length,'COMMAND_DISPLAY_REJECTED');const next=display[index+1];
   if(!quote||'\\"$`\n'.includes(next)){index++;if(next!=='\n')token+=next;}
   else token+=c;
   started=true;continue;
  }
  if(!quote&&/\s/.test(c)){if(started){values.push(token);token='';started=false;}continue;}
  token+=c;started=true;
 }
 requireThat(!quote,'COMMAND_DISPLAY_REJECTED');if(started)values.push(token);
 requireThat(values.length>0,'COMMAND_DISPLAY_REJECTED');return values;
}
export function completedTurnDigests(first,last,history,historyAnchor){
 if(digest(first)===digest(last))return {pre:digest(first),post:digest(last)};
 requireThat(historyAnchor?.status==='READ_ONLY_ELIGIBLE'&&historyAnchor.databaseMutation===false&&historyAnchor.proof?.pre_turn_digest===digest(first)&&historyAnchor.proof.post_turn_digest===digest(first)&&historyAnchor.proof.latest_turn_id===first.id&&historyAnchor.proof.rollout_sha256===history.sha256,'COMPLETED_TURN_CONTENT_CHANGED');
 const enriched=structuredClone(first),records=history.completedItems??[];
 requireThat(first.items.length===last.items.length,'COMPLETED_TURN_CONTENT_CHANGED');
 for(let index=0;index<first.items.length;index++){
  const before=first.items[index],after=last.items[index];
  requireThat(before.id===after.id&&before.type===after.type,'COMPLETED_TURN_CONTENT_CHANGED');
  for(const field of Object.keys(after).filter(k=>!Object.hasOwn(before,k))){
   const matches=records.filter(r=>r.id===before.id&&r.threadId===history.sessionId&&r.turnId===first.id);
   requireThat(matches.length===1,'ARCHIVED_ITEM_PROOF_REJECTED');const r=matches[0];
   if(field==='command'&&before.type==='commandExecution'){
    requireThat(r.type==='CommandExecution'&&r.commandVectorDigest===digest(commandVector(after.command))&&r.cwd===before.cwd,'ARCHIVED_COMMAND_CHANGED');
   }else if(field==='arguments'&&before.type==='mcpToolCall'){
    requireThat(r.type==='McpToolCall'&&r.argumentsDigest===digest(after.arguments)&&r.server===before.server&&r.tool===before.tool,'ARCHIVED_ARGUMENTS_CHANGED');
   }else throw Error('COMPLETED_TURN_CONTENT_CHANGED');
   enriched.items[index][field]=after[field];
  }
 }
 requireThat(digest(enriched)===digest(last),'COMPLETED_TURN_CONTENT_CHANGED');
 return {pre:digest(enriched),post:digest(last)};
}
// The original full native turn is bound to the immutable commit request in DB.
// Metadata-only fallback summaries are deliberately insufficient for recovery.
export function validateProof({current,anchor,pre,post,archive,durable,history,historyAnchor},now=Date.now()){
 const old=current.handoff.old_thread_id,locked=current.handoff_boundary,first=pre.pages[0],last=post.pages[0];
 requireThat(current.contract_version===2&&current.phase==='MIGRATING'&&current.archive_reconciliation?.handoff_id!==current.handoff.handoff_id&&current.steps.old_archived?.status==='RESERVED','RECONCILE_NOT_READY');
 requireThat(anchor.requestDigest===digest(anchor.request)&&anchor.request.action==='commit'&&anchor.request.handoff_id===current.handoff.handoff_id&&anchor.request.body.manifest_digest===current.manifest_digest,'ANCHOR_MISMATCH');
 requireThat(digest(pre)===anchor.request.native.old.readback_digest&&first.thread.id===old&&first.thread.updatedAt===locked.thread_updated_at&&first.turns[0].id===locked.latest_turn_id,'PRE_NATIVE_NOT_ANCHORED');
 requireThat(last.thread.id===old&&last.thread.hostId===current.handoff.old_host_id&&last.thread.status.type==='notLoaded'&&last.turns[0].status==='completed'&&last.turns[0].id===locked.latest_turn_id,'POST_NATIVE_BOUNDARY_CHANGED');
 requireThat(now-Date.parse(post.observedAt)<=300000&&Date.parse(post.observedAt)<=now+5000&&Number.isFinite(Date.parse(post.observedAt)),'POST_NATIVE_STALE');
 requireThat(Number.isSafeInteger(last.thread.updatedAt)&&last.thread.updatedAt>0&&last.thread.updatedAt<locked.thread_updated_at,'NOT_ARCHIVE_METADATA_REGRESSION');
 requireThat(first.turns[0].items?.length>0&&last.turns[0].items?.length>0,'COMPLETED_TURN_CONTENT_CHANGED');
 const turnDigests=completedTurnDigests(first.turns[0],last.turns[0],history,historyAnchor);
 const result=archive.result??archive,native=JSON.parse(result.content?.find(x=>x.type==='text')?.text??'null');
 requireThat(result.isError!==true&&native?.threadId===old&&native.archived===true,'ARCHIVE_RECEIPT_MISMATCH');
 requireThat(durable.id===old&&durable.archived===1&&durable.updated_at===last.thread.updatedAt&&Number.isSafeInteger(durable.archived_at)&&durable.archived_at>=locked.thread_updated_at&&durable.archived_at<=now/1000+5,'ARCHIVE_DURABLE_MISMATCH');
 requireThat(history.sessionId===old&&history.latestStartedTurn===locked.latest_turn_id&&history.latestCompletedTurn===locked.latest_turn_id&&history.afterCompletionEvents===0&&history.pendingActions===0,'ARCHIVED_HISTORY_CHANGED');
 return {pre_turn_digest:turnDigests.pre,post_turn_digest:turnDigests.post,rollout_sha256:history.sha256,archived:true,archived_at:durable.archived_at,durable_updated_at:durable.updated_at,latest_turn_id:locked.latest_turn_id,new_input_count:0,in_flight_count:0};
}
export async function inspectHistory(file){
 const hash=crypto.createHash('sha256');let sessionId,latestStartedTurn,latestCompletedTurn,afterCompletionEvents=0,pendingActions=new Set(),completedItems=[];
 const stream=fs.createReadStream(file);stream.on('data',bytes=>hash.update(bytes));
 for await(const line of createInterface({input:stream,crlfDelay:Infinity})){
  const r=JSON.parse(line),p=r.payload;
  if(r.type==='session_meta')sessionId=p.id;
  if(r.type==='event_msg'){
   if(p.type==='task_started'){
    if(p.turn_id===latestStartedTurn||p.turn_id===latestCompletedTurn)afterCompletionEvents++;
    else{latestStartedTurn=p.turn_id;afterCompletionEvents=0;completedItems=[];}
   }
   if(p.type==='task_complete'){
    // Completion cannot erase activity observed after the pinned completion.
    if(p.turn_id!==latestStartedTurn||p.turn_id===latestCompletedTurn)afterCompletionEvents++;
    latestCompletedTurn=p.turn_id;
   }
   else if(latestCompletedTurn===latestStartedTurn&&p.type==='user_message')afterCompletionEvents++;
   if(p.type==='item_completed'&&p.turn_id===latestStartedTurn){
    const i=p.item,record={id:i.id,type:i.type,threadId:p.thread_id,turnId:p.turn_id};
    if(i.type==='CommandExecution'){
     requireThat(Array.isArray(i.command)&&i.command.every(x=>typeof x==='string'),'ARCHIVED_COMMAND_FORMAT_REJECTED');
     record.commandVectorDigest=digest(i.command);record.cwd=fileURLToPath(i.cwd);
    }
    if(i.type==='McpToolCall'){record.argumentsDigest=digest(i.arguments);record.server=i.server;record.tool=i.tool;}
    completedItems.push(record);
    if(latestCompletedTurn===latestStartedTurn)afterCompletionEvents++;
   }
  }
  if(r.type==='response_item'){
   if(['function_call','custom_tool_call'].includes(p.type))pendingActions.add(p.call_id);
   if(['function_call_output','custom_tool_call_output'].includes(p.type))pendingActions.delete(p.call_id);
   if(latestCompletedTurn===latestStartedTurn)afterCompletionEvents++;
  }
 }
 return {sessionId,latestStartedTurn,latestCompletedTurn,afterCompletionEvents,pendingActions:pendingActions.size,sha256:hash.digest('hex'),completedItems};
}
async function sources(root,input){
 const hq=await import(pathToFileURL(path.join(root,'scripts/bot-lifecycle.mjs')));
 const adapter=await import(pathToFileURL(path.join(root,'scripts/control-handoff.mjs')));
 const inside=relative=>{requireThat(typeof relative==='string'&&!path.isAbsolute(relative),'RELATIVE_EVIDENCE_REQUIRED');const f=fs.realpathSync(path.resolve(root,relative)),rel=path.relative(fs.realpathSync(root),f);requireThat(rel&&!rel.startsWith('..')&&!path.isAbsolute(rel),'EVIDENCE_OUTSIDE_ROOT');return f;};
 const pre=read(inside(input.preNativePath)),post=read(inside(input.postNativePath)),anchor=read(inside(input.anchorPath)),archive=read(inside(input.archiveNativePath));
 const historyAnchor=input.historyAnchorPath?read(inside(input.historyAnchorPath)):undefined;
 const self=read(inside(input.selfNativePath));adapter.assertNativeActor(input.actor,process.env.CODEX_THREAD_ID,self);
 const {current}=await hq.database(root,{action:'read',project_id:input.projectId,role_id:'control'});
 const proof=hq.archiveProof(archive,current.handoff.old_thread_id,current.owner_thread_id);
 const db=new DatabaseSync(path.join(homedir(),'.codex/state_5.sqlite'),{readOnly:true});let durable;
 try{durable=db.prepare('SELECT id,archived,archived_at,updated_at,rollout_path FROM threads WHERE id=?').get(current.handoff.old_thread_id);}finally{db.close();}
 const history=await inspectHistory(durable.rollout_path);
 return {hq,adapter,current,pre,post,anchor,archive,durable,history,historyAnchor,proof};
}
export async function prepareRecovery(root,input){
 const x=await sources(root,input),{current,hq,adapter}=x;
 requireThat(input.actor.role_id==='control'&&input.actor.thread_id===current.owner_thread_id&&input.actor.generation===current.generation&&input.actor.host_id===current.owner_host_id,'CURRENT_CONTROL_ACTOR_REQUIRED');
 requireThat(current.policy_digest===hq.digest(hq.policy(root,'control'))&&current.rules_digest===hq.snapshotRulesDigest(hq.sharedSnapshot(root,'control')),'ENROLLED_CONTROL_CONTRACT_CHANGED');
 const proof=validateProof(x),evidence={preNativeDigest:digest(x.pre),postNativeDigest:digest(x.post),historyAnchorDigest:x.historyAnchor?digest(x.historyAnchor):null,durable:x.durable,history:x.history,archiveProof:x.proof};
 requireThat(/^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(input.evidencePath)&&!input.evidencePath.split('/').includes('..'),'RELATIVE_EVIDENCE_REQUIRED');
 proof.evidence_digest=digest(evidence);
 hq.saveExclusive(path.resolve(root,input.evidencePath),evidence);
 const old=adapter.boundaryFromEvidence(x.post,{source:'codex.read_thread',threadId:current.handoff.old_thread_id,latestTurnId:x.post.pages[0].turns[0].id,nativeDigest:digest(x.post),pendingInputs:0,inflightActions:0,checkedSourceRefs:[input.evidencePath]},current.handoff.old_thread_id,current.handoff.old_host_id,input.postNativePath);
 const request={action:'reconcile-archived-boundary',request_id:input.requestId,project_id:input.projectId,role_id:'control',expected_revision:current.revision,expected_generation:current.generation,owner_thread_id:current.owner_thread_id,owner_host_id:current.owner_host_id,handoff_id:current.handoff.handoff_id,actor:input.actor,native:{old,new:null},body:{manifest_digest:current.manifest_digest,anchor_request_id:x.anchor.request.request_id,pre_native_digest:digest(x.pre),archive_operation_key:current.steps.old_archived.operation_key,archive_receipt:{tool:'set_thread_archived',target_thread_id:current.handoff.old_thread_id,target_host_id:current.handoff.old_host_id,result_digest:digest(x.archive.result??x.archive),readback_digest:digest(evidence),evidence_ref:input.evidencePath,success:true,readback_verified:true,old_pending_count:0},history_proof:proof}};
 return {schemaVersion:1,kind:'control-archive-reconciliation',request,requestDigest:digest(request),sourceInput:input,historyAnchorDigest:evidence.historyAnchorDigest,proofDigest:digest(evidence),backend:read(path.join(root,'state/bot-lifecycle-backend.local.json'))};
}
export async function executeRecovery(root,prepared,freshSelfPath){
 requireThat(prepared.kind==='control-archive-reconciliation'&&prepared.requestDigest===digest(prepared.request),'SAVED_REQUEST_DIGEST_MISMATCH');
 requireThat(digest(prepared.request.actor)===digest(prepared.sourceInput.actor)&&prepared.request.owner_thread_id===process.env.CODEX_THREAD_ID&&prepared.request.actor.thread_id===process.env.CODEX_THREAD_ID&&prepared.request.expected_generation===prepared.request.actor.generation,'ACTUAL_CURRENT_OWNER_REQUIRED');
 const input={...prepared.sourceInput,selfNativePath:freshSelfPath},x=await sources(root,input);
 requireThat((x.historyAnchor?digest(x.historyAnchor):null)===(prepared.historyAnchorDigest??null),'HISTORY_ANCHOR_CHANGED');
 const same={...x.current,phase:'MIGRATING',archive_reconciliation:undefined,handoff_boundary:x.pre.pages[0]&&{latest_turn_id:x.pre.pages[0].turns[0].id,thread_updated_at:x.pre.pages[0].thread.updatedAt},steps:{...x.current.steps,old_archived:{...x.current.steps.old_archived,status:'RESERVED'}}};
 // Revalidate actual evidence even on exact replay; this local projection is
 // validation-only, never serialized or supplied as authoritative DB state.
 const proof=validateProof({...x,current:same});
 requireThat(proof.rollout_sha256===prepared.request.body.history_proof.rollout_sha256&&digest(x.post)===prepared.request.native.old.readback_digest,'RECOVERY_EVIDENCE_CHANGED');
 const {evidence_digest,...savedProof}=prepared.request.body.history_proof;
 requireThat(digest(proof)===digest(savedProof)&&prepared.request.body.pre_native_digest===digest(x.pre)&&prepared.request.body.anchor_request_id===x.anchor.request.request_id&&prepared.request.body.archive_receipt.result_digest===digest(x.archive.result??x.archive),'RECOVERY_PROOF_BINDING_CHANGED');
 requireThat(digest(prepared.backend)===digest(read(path.join(root,'state/bot-lifecycle-backend.local.json'))),'BACKEND_CHANGED');
 const {environment}=await loadLatticeRuntimeConfiguration(),backend=prepared.backend;
 const output=execFileSync(backend.binary,['bot-lifecycle-reconcile-archive','--postgres-host',backend.host,'--postgres-port',String(backend.port),'--postgres-run-id',backend.runId],{input:JSON.stringify(prepared.request),env:closedChildEnvironment(environment),encoding:'utf8',windowsHide:true,timeout:30000,maxBuffer:1048576});
 return JSON.parse(output);
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url)){
 const [command,root,inputPath,outputPath,selfPath]=process.argv.slice(2),hq=await import(pathToFileURL(path.join(root,'scripts/bot-lifecycle.mjs')));
 const value=command==='prepare'?await prepareRecovery(root,read(inputPath)):command==='execute'?await executeRecovery(root,read(inputPath),selfPath):null;
 requireThat(value,'UNKNOWN_COMMAND');hq.saveExclusive(outputPath,value);
 console.log(JSON.stringify({status:value.status??'PREPARED',requestDigest:value.requestDigest,phase:value.current?.phase,revision:value.current?.revision,outputPath}));
}
