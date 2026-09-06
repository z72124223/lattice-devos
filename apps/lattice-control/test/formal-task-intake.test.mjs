import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { FormalTaskService } from '../src/formal-task-service.mjs';
import { projectFormalWork } from '../src/formal-work-store.mjs';
const projectId='project-a', parent='a'.repeat(64), child='b'.repeat(64);
function fixture(t){
  const tasks=new Map([[parent,{id:parent,project_id:projectId,metadata:null}]]);let submitted=0,started=0;
  const store={
    async detail(project,id){const task=tasks.get(id);if(!task||task.project_id!==project)throw Object.assign(new Error('wrong project'),{code:'CONTROL_WORK_PROJECT_MISMATCH'});return structuredClone(task);},
    async submit({objective}){submitted++;if(!tasks.has(child))tasks.set(child,{id:child,project_id:projectId,objective,metadata:null});return {task_ref:child};},
    async update(command){assert.equal(command.action,'METADATA');const task=tasks.get(command.task_ref);task.metadata={...command,revision:1};return {record:structuredClone(task.metadata)};},
    invalidate(){},async close(){}
  };
  const codex=new EventEmitter();codex.startThread=async()=>{started++;throw Error('must not dispatch on save');};codex.close=async()=>{};
  const service=new FormalTaskService({store,codex});t.after(()=>service.close());
  return {service,tasks,counts:()=>({submitted,started})};
}
test('save a child with its parent and criteria, replay it, and never start Codex implicitly',async t=>{
  const {service,counts}=fixture(t);const request={projectId,objective:'驗證工作狀態',parentTaskRef:parent,successCriteria:'實測並保留結果',clientRequestId:'child-intake-one'};
  const a=await service.create(request),b=await service.create(request);
  assert.equal(a.id,b.id);assert.equal(a.metadata.parent_ref,parent);assert.equal(a.metadata.success_criteria,request.successCriteria);assert.equal(counts().started,0);
  await assert.rejects(service.create({...request,parentTaskRef:null}),{code:'CONTROL_WORK_INTAKE_CHANGED'});
  await assert.rejects(service.create({...request,successCriteria:'降低驗收要求'}),{code:'CONTROL_WORK_INTAKE_CHANGED'});
});
test('reject an invalid or foreign parent before creating an orphan task',async t=>{
  const {service,tasks,counts}=fixture(t);tasks.set(parent,{id:parent,project_id:'other-project',metadata:null});
  await assert.rejects(service.create({projectId,objective:'驗證',parentTaskRef:parent,clientRequestId:'foreign-parent'}),{code:'CONTROL_WORK_PROJECT_MISMATCH'});
  assert.equal(counts().submitted,0);
  assert.throws(()=>service.create({projectId,objective:'驗證',parentTaskRef:'invalid',clientRequestId:'bad-parent'}),TypeError);
});
test('the same durable child remains under its parent through execution and verified completion',()=>{
  const page={source:{authority:'POSTGRESQL_TASK_LEDGER'},project:{id:projectId},revision:'one',tasks:[parent,child].map(task_ref=>({task_ref,objective:'工作',ledger:{project_id:projectId,status:'SUBMITTED',result_digest:null}})),product:{metadata:[{task_ref:child,parent_ref:parent,dependency_refs:[]}],claims:[],observations:[],decisions:[]}};
  const project=()=>projectFormalWork([page]).snapshot;
  assert.equal(project().tree.nodes.find(n=>n.id===child).status,'draft');
  page.product.claims.push({task_ref:child,claim_id:'claim',phase:'EXECUTION',turn_status:'TURN_BOUND',turn_id:'real-turn'});
  const running=project();assert.equal(running.tree.nodes.find(n=>n.id===child).status,'running');assert.equal(running.tree.nodes.find(n=>n.id===child).parent_id,parent);
  page.product.claims[0].turn_status='TURN_COMPLETED';assert.equal(project().tree.nodes.find(n=>n.id===child).status,'codex_done');
  page.tasks[1].ledger.status='COMPLETED';page.tasks[1].ledger.result_digest='c'.repeat(64);
  const complete=project();assert.equal(complete.tree.nodes.find(n=>n.id===child).status,'verified');assert.equal(complete.tree.nodes.find(n=>n.id===parent).status,'draft');assert.deepEqual(complete.tree.nodes.find(n=>n.id===parent).children,[child]);
});
