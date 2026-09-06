import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { CodeGraphStore, validateCodeGraph } from '../src/code-graph.mjs';
import { codeIndex, codeBranches, searchCode } from '../src/code-graph-model.mjs';

const node = (id, file = 'src/main.js') => ({id,label:id,file,location:'L1',kind:'function'});
const edge = (source,target,relation='calls') => ({source,target,relation,confidence:'EXTRACTED',file:'src/main.js',location:'L1'});
function graph(commit = 'a'.repeat(40)) { return {schema_version:'lattice.control.code-graph.v1', source:'GRAPHIFY', authority:'DERIVED',
  commit, graph_digest:'b'.repeat(64), nodes:['entry','checkout','refund','price','other'].map(id => node(id)),
  edges:[edge('entry','checkout'),edge('checkout','price'),edge('refund','price'),edge('other','refund','contains')]}; }

test('reverse traversal finds callers and their callers without including callees as callers', () => {
  const index=codeIndex(graph());
  const direct=codeBranches(index,'price',new Set(['price']),'incoming','calls');
  assert.deepEqual(direct.nodes.map(n=>n.id),['price','checkout','refund']);
  const expanded=codeBranches(index,'price',new Set(['price','checkout','refund']),'incoming','calls');
  assert.deepEqual(expanded.nodes.map(n=>n.id),['price','checkout','refund','entry']);
  assert.deepEqual(codeBranches(index,'price',new Set(['price']),'outgoing').nodes.map(n=>n.id),['price']);
  assert.deepEqual(codeBranches(index,'checkout',new Set(['checkout']),'outgoing').nodes.map(n=>n.id),['checkout','price']);
});
test('code branches handle cycles, shared callers, folding and bounded expansion', () => {
  const data=graph();data.edges.push(edge('price','entry'),edge('entry','refund'));
  const index=codeIndex(data), all=new Set(data.nodes.map(n=>n.id));
  const result=codeBranches(index,'price',all,'incoming');
  assert.equal(result.nodes.length,5); assert.equal(new Set(result.nodes.map(n=>n.id)).size,5);
  assert.equal(codeBranches(index,'price',new Set()).nodes.length,1);
  const bounded=codeBranches(index,'price',all,'incoming','all',3);
  assert.equal(bounded.nodes.length,3);assert.equal(bounded.truncated,true);
  assert.ok(bounded.edges.every(e=>bounded.nodes.some(n=>n.id===e.source)&&bounded.nodes.some(n=>n.id===e.target)));
});
test('search matches paths and identifiers, never treats task statuses as code', () => {
  const index=codeIndex(graph()); assert.equal(searchCode(index,'PRICE')[0].id,'price');
  assert.equal(searchCode(index,'src/main.js').length,5);assert.equal(searchCode(index,'進行中').length,0);
});
test('graph cache rejects foreign versions, dangling edges and escaped source locations', () => {
  assert.equal(validateCodeGraph(graph(),'a'.repeat(40)).nodes.length,5);
  assert.throws(()=>validateCodeGraph(graph(),'c'.repeat(40)));
  for (const invalid of [ {...graph(),source:'TASK_LEDGER'}, {...graph(),nodes:[node('price','../private.txt')]},
    {...graph(),edges:[edge('missing','price')]}, {...graph(),nodes:[node('price'),node('price')]}])
    assert.throws(()=>validateCodeGraph(invalid,'a'.repeat(40)));
});

test('real repository binding, cache replay, stale changes, project isolation and failed refresh', async t => {
  const dir=await mkdtemp(path.join(tmpdir(),'lattice-code-graph-'));
  t.after(()=>rm(dir,{recursive:true,force:true}));
  const repo=path.join(dir,'repo');await mkdir(repo);
  const git=process.platform==='win32'?execFileSync('where.exe',['git.exe'],{encoding:'utf8'}).trim().split(/\r?\n/)[0]
    :execFileSync('which',['git'],{encoding:'utf8'}).trim();
  const run=(...args)=>execFileSync(git,args,{cwd:repo,windowsHide:true,encoding:'utf8'}).trim();
  run('init','-q');run('config','user.name','Graph test');run('config','user.email','graph@example.invalid');
  await writeFile(path.join(repo,'source.js'),'export const one = 1;\n');run('add','source.js');run('commit','-qm','fixture');
  const commit=run('rev-parse','HEAD');
  const project={id:'project-one',name:'Project one',root_path:repo};
  const options={cacheDirectory:path.join(dir,'cache'),executable:process.execPath,
    configurationLoader:async()=>({environment:{LATTICE_DELIVERY_GIT_EXE:git,LATTICE_GRAPHIFY_RUNTIME_ROOT:dir,LATTICE_GRAPHIFY_WSL_EXE:git}})};
  const store=new CodeGraphStore(options);
  assert.equal((await store.read(project)).status,'not_analyzed');
  const inspection=await store.inspect(project);
  store.run=async(exe,args,opts)=>exe===process.execPath?{stdout:JSON.stringify(graph(commit))}
    :{stdout:execFileSync(exe,args,{...opts,encoding:'utf8'})};
  await store.start(project);await store.active;
  const first=await store.read(project);assert.equal(first.status,'ready');assert.equal(first.stale,false);
  assert.equal(first.graph.project_id,project.id);
  const replay=new CodeGraphStore(options);
  assert.equal((await replay.read(project)).graph.commit,commit);
  assert.equal((await replay.read({...project,id:'project-two'})).graph,null,'another project cannot reuse the cache');
  assert.equal((await replay.read(project,{checkout:'../foreign'})).status,'unavailable');
  const sliced=await store.read(project,{node:'price',direction:'incoming',depth:2});
  assert.ok(sliced.graph.nodes.some(n=>n.id==='entry'));assert.equal(sliced.query.direction,'incoming');
  await writeFile(path.join(repo,'source.js'),'export const one = 2;\n');
  assert.equal((await replay.read(project)).stale,true);
  store.run=async(exe,args,opts)=>{if(exe===process.execPath)throw Object.assign(new Error('failed'),{stderr:'GRAPHIFY_TIMEOUT'});
    return {stdout:execFileSync(exe,args,{...opts,encoding:'utf8'})};};
  await store.start(project,inspection.checkout);await store.active;
  const failed=await store.read(project);assert.equal(failed.status,'failed');assert.equal(failed.error,'GRAPHIFY_TIMEOUT');
  assert.equal(failed.graph.commit,commit,'failed refresh preserves earlier derived evidence');
});


test('HTTP code graph uses the selected project and rejects source paths and foreign origins', async t => {
  const {EventEmitter}=await import('node:events');
  const {createLatticeServer}=await import('../src/server.mjs');
  const codex=new EventEmitter();codex.close=async()=>{};
  const app=createLatticeServer({databasePath:':memory:',codex});
  const project=app.store.createProject({name:'Graph project',rootPath:process.cwd()});
  const calls=[];
  app.service.codeGraphStore={read:async(p,q)=>{calls.push(['read',p.id,q]);return {project_id:p.id,status:'not_analyzed'};},
    start:async(p,checkout)=>{calls.push(['start',p.id,checkout]);return {project_id:p.id,status:'analyzing'};},close:async()=>{}};
  await new Promise(resolve=>app.server.listen(0,'127.0.0.1',resolve));
  t.after(()=>new Promise(resolve=>app.server.close(resolve)));
  const origin='http://127.0.0.1:'+app.server.address().port;
  const view=await fetch(origin+'/api/code-graph?projectId='+project.id+'&q=price&direction=incoming&depth=2');
  assert.equal(view.status,200);assert.equal((await view.json()).project_id,project.id);
  assert.equal(calls[0][2].direction,'incoming');assert.equal(calls.length,1);
  assert.equal((await fetch(origin+'/api/code-graph?projectId=missing')).status,404);
  const rejected=await fetch(origin+'/api/code-graph/analyze',{method:'POST',headers:{'content-type':'application/json'},
    body:JSON.stringify({projectId:project.id,rootPath:'C:/foreign'})});assert.equal(rejected.status,400);
  const foreign=await fetch(origin+'/api/code-graph/analyze',{method:'POST',headers:{'content-type':'application/json',origin:'https://foreign.invalid'},
    body:JSON.stringify({projectId:project.id})});assert.equal(foreign.status,403);assert.equal(calls.length,1);
  const started=await fetch(origin+'/api/code-graph/analyze',{method:'POST',headers:{'content-type':'application/json'},
    body:JSON.stringify({projectId:project.id})});assert.equal(started.status,202);assert.equal(calls[1][0],'start');
  for(const asset of ['/code-graph-view.mjs','/code-graph-model.mjs','/code-graph.css'])assert.equal((await fetch(origin+asset)).status,410);
});
