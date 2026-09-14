import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {digest,validateProof,inspectHistory,commandVector} from './bot-lifecycle-archive-reconcile.mjs';
const now=Date.now(),stamp=Math.floor(now/1000)-60,turn='completed-checkpoint',old='old-owner';
function fixture(){
 const pre={source:'codex.read_thread',observedAt:new Date(now-60000).toISOString(),pages:[{thread:{id:old,hostId:'local',updatedAt:stamp,status:{type:'idle'}},turns:[{id:turn,status:'completed',items:[{type:'agentMessage',id:'final',text:'retained complete result',phase:'final_answer'}]}]}]};
 const post=structuredClone(pre);post.observedAt=new Date(now).toISOString();post.pages[0].thread.updatedAt=stamp-100;post.pages[0].thread.status.type='notLoaded';
 const current={contract_version:2,phase:'MIGRATING',handoff:{old_thread_id:old,old_host_id:'local',handoff_id:'same-handoff'},handoff_boundary:{latest_turn_id:turn,thread_updated_at:stamp},manifest_digest:'frozen-manifest',steps:{old_archived:{status:'RESERVED'}}};
 const request={action:'commit',handoff_id:'same-handoff',body:{manifest_digest:'frozen-manifest'},native:{old:{readback_digest:digest(pre)}}};
 return {current,pre,post,anchor:{request,requestDigest:digest(request)},archive:{content:[{type:'text',text:JSON.stringify({threadId:old,archived:true})}],isError:false},durable:{id:old,archived:1,updated_at:stamp-100,archived_at:stamp+10},history:{sessionId:old,latestStartedTurn:turn,latestCompletedTurn:turn,afterCompletionEvents:0,pendingActions:0,sha256:'a'.repeat(64)}};
}
test('metadata regression with full anchored turn and archive evidence is eligible',()=>{
 const x=fixture(),before=structuredClone(x),p=validateProof(x,now);assert.equal(p.pre_turn_digest,p.post_turn_digest);assert.equal(p.durable_updated_at,stamp-100);assert.deepEqual(x,before);
});
const invalid=[
 ['new completed turn',x=>x.post.pages[0].turns[0].id='new-turn'],
 ['active turn',x=>x.post.pages[0].thread.status.type='active'],
 ['changed completed content',x=>x.post.pages[0].turns[0].items[0].text='changed'],
 ['missing full turn evidence',x=>delete x.post.pages[0].turns[0].items],
 ['changed pre source',x=>x.pre.pages[0].turns[0].items[0].text='changed'],
 ['unanchored manifest',x=>x.current.manifest_digest='changed'],
 ['wrong archive target',x=>x.archive.content[0].text=JSON.stringify({threadId:'another',archived:true})],
 ['failed archive',x=>x.archive.isError=true],
 ['restored thread',x=>x.durable.archived=0],
 ['unmatched durable timestamp',x=>x.durable.updated_at--],
 ['old archive proof',x=>x.durable.archived_at=stamp-1],
 ['new user input after completion',x=>x.history.afterCompletionEvents=1],
 ['new started turn',x=>x.history.latestStartedTurn='new-turn'],
 ['pending native action',x=>x.history.pendingActions=1],
 ['stale observation',x=>x.post.observedAt=new Date(now-301000).toISOString()],
 ['forward timestamp drift',x=>x.post.pages[0].thread.updatedAt=stamp+1],
 ['already reconciled',x=>x.current.archive_reconciliation={handoff_id:'same-handoff'}],
];
for(const [name,change] of invalid)test('reject '+name,()=>{const x=fixture();change(x);assert.throws(()=>validateProof(x,now));});

for(const field of ['command','arguments'])test('reject changed '+field+' in an otherwise identical anchored turn',()=>{
 const x=fixture(),item={type:'commandExecution',id:'call',command:'original',arguments:'original'};
 x.pre.pages[0].turns[0].items.push(item);x.post.pages[0].turns[0].items.push(structuredClone(item));
 x.anchor.request.native.old.readback_digest=digest(x.pre);x.anchor.requestDigest=digest(x.anchor.request);
 validateProof(x,now);x.post.pages[0].turns[0].items[1][field]='changed';
 assert.throws(()=>validateProof(x,now),/COMPLETED_TURN_CONTENT_CHANGED/);
});
test('archived history detects post-completion input and unfinished calls',async()=>{
 const dir=fs.mkdtempSync(path.join(os.tmpdir(),'lattice-archive-history-')),file=path.join(dir,'fixture.jsonl');
 const lines=[{type:'session_meta',payload:{id:old}},{type:'event_msg',payload:{type:'task_started',turn_id:turn}},{type:'event_msg',payload:{type:'task_complete',turn_id:turn}}];
 try{
  fs.writeFileSync(file,lines.map(x=>JSON.stringify(x)).join('\n')+'\n');const good=await inspectHistory(file);assert.equal(good.afterCompletionEvents,0);assert.equal(good.latestCompletedTurn,turn);
  fs.appendFileSync(file,JSON.stringify({type:'response_item',payload:{type:'message',role:'user',content:[]}})+'\n');assert.equal((await inspectHistory(file)).afterCompletionEvents,1);
  fs.appendFileSync(file,JSON.stringify({type:'response_item',payload:{type:'custom_tool_call',call_id:'unfinished'}})+'\n');assert.equal((await inspectHistory(file)).pendingActions,1);
 }finally{fs.unlinkSync(file);fs.rmdirSync(dir);}
});

for(const activity of ['response_item','user_message'])test('duplicate completion cannot conceal '+activity,async()=>{
 const dir=fs.mkdtempSync(path.join(os.tmpdir(),'lattice-archive-history-')),file=path.join(dir,'fixture.jsonl');
 const complete={type:'event_msg',payload:{type:'task_complete',turn_id:turn}};
 const lines=[{type:'session_meta',payload:{id:old}},{type:'event_msg',payload:{type:'task_started',turn_id:turn}},complete,
  activity==='response_item'?{type:'response_item',payload:{type:'message',role:'user',content:[]}}:{type:'event_msg',payload:{type:'user_message',message:'new input'}},complete];
 try{fs.writeFileSync(file,lines.map(x=>JSON.stringify(x)).join('\n')+'\n');const x=fixture();x.history=await inspectHistory(file);assert.ok(x.history.afterCompletionEvents>0);assert.throws(()=>validateProof(x,now),/ARCHIVED_HISTORY_CHANGED/);}
 finally{fs.unlinkSync(file);fs.rmdirSync(dir);}
});

test('earlier completed turn followed by a different valid turn remains eligible',async()=>{
 const dir=fs.mkdtempSync(path.join(os.tmpdir(),'lattice-archive-history-')),file=path.join(dir,'fixture.jsonl');
 const lines=[{type:'session_meta',payload:{id:old}},...['earlier',turn].flatMap(id=>[{type:'event_msg',payload:{type:'task_started',turn_id:id}},{type:'response_item',payload:{type:'message',role:'user',content:[]}},{type:'event_msg',payload:{type:'task_complete',turn_id:id}}])];
 try{fs.writeFileSync(file,lines.map(x=>JSON.stringify(x)).join('\n')+'\n');const x=fixture();x.history=await inspectHistory(file);assert.equal(x.history.afterCompletionEvents,0);validateProof(x,now);}
 finally{fs.unlinkSync(file);fs.rmdirSync(dir);}
});

function enrichedFixture(){
 const x=fixture(),items=[{id:'command',type:'commandExecution',cwd:'retained-directory'},{id:'tool',type:'mcpToolCall',server:'codex',tool:'read_thread'}];
 x.pre.pages[0].turns[0].items.push(...items);x.post.pages[0].turns[0].items.push(...structuredClone(items));
 x.post.pages[0].turns[0].items[1].command='node "a b"';x.post.pages[0].turns[0].items[2].arguments={threadId:old};
 x.anchor.request.native.old.readback_digest=digest(x.pre);x.anchor.requestDigest=digest(x.anchor.request);
 const original=digest(x.pre.pages[0].turns[0]);x.historyAnchor={status:'READ_ONLY_ELIGIBLE',databaseMutation:false,proof:{pre_turn_digest:original,post_turn_digest:original,latest_turn_id:turn,rollout_sha256:x.history.sha256}};
 x.history.completedItems=[{id:'command',type:'CommandExecution',threadId:old,turnId:turn,cwd:'retained-directory',commandVectorDigest:digest(['node','a b'])},{id:'tool',type:'McpToolCall',threadId:old,turnId:turn,server:'codex',tool:'read_thread',argumentsDigest:digest({threadId:old})}];return x;
}
test('only archive-proven missing fields enrich the anchored turn without source mutation',()=>{
 const x=enrichedFixture(),before=structuredClone(x),proof=validateProof(x,now);
 assert.equal(proof.pre_turn_digest,digest(x.post.pages[0].turns[0]));assert.equal(proof.pre_turn_digest,proof.post_turn_digest);assert.deepEqual(x,before);
});
for(const [name,change] of [
 ['missing prior history anchor',x=>delete x.historyAnchor],
 ['changed archived rollout',x=>x.history.sha256='b'.repeat(64)],
 ['unbound prior turn digest',x=>x.historyAnchor.proof.pre_turn_digest='b'.repeat(64)],
 ['changed newly disclosed command',x=>x.post.pages[0].turns[0].items[1].command='node changed'],
 ['changed newly disclosed arguments',x=>x.post.pages[0].turns[0].items[2].arguments.threadId='another'],
 ['missing archived item',x=>x.history.completedItems.shift()],
 ['ambiguous duplicate item',x=>x.history.completedItems.push(x.history.completedItems[0])],
 ['different source turn',x=>x.history.completedItems[0].turnId='another'],
 ['different source thread',x=>x.history.completedItems[0].threadId='another'],
 ['different command directory',x=>x.history.completedItems[0].cwd='another'],
 ['different tool target',x=>x.history.completedItems[1].tool='another'],
 ['unrecognized new field',x=>x.post.pages[0].turns[0].items[1].newField=true],
 ['changed retained field',x=>x.post.pages[0].turns[0].items[0].text='changed'],
 ['deleted retained field',x=>delete x.post.pages[0].turns[0].items[0].text],
 ['reordered items',x=>x.post.pages[0].turns[0].items.reverse()],
])test('reject enrichment with '+name,()=>{const x=enrichedFixture();change(x);assert.throws(()=>validateProof(x,now));});
test('command display decoding preserves argument boundaries and quoting without evaluation',()=>{
 assert.deepEqual(commandVector(`node 'a b' "c d" plain`),['node','a b','c d','plain']);
 assert.deepEqual(commandVector(`node 'a'"'"'b'`),['node',"a'b"]);
 assert.deepEqual(commandVector(String.raw`"C:\\tools\\node.exe" "a\"b" '$literal' "back\\slash"`),['C:\\tools\\node.exe','a"b','$literal','back\\slash']);
 assert.deepEqual(commandVector(`node '' 'line\nline'`),['node','','line\nline']);
 assert.throws(()=>commandVector(`node 'unclosed`));assert.throws(()=>commandVector('node \\'));
});
