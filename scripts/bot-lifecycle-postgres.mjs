// Windows local component activation. No scheduler, service installation or deletion.
import {spawnSync} from 'node:child_process';
import {mkdirSync,existsSync,readFileSync,writeFileSync,appendFileSync,unlinkSync} from 'node:fs';
import {join,resolve} from 'node:path';
import {homedir} from 'node:os';
import {createServer} from 'node:net';
import {randomUUID} from 'node:crypto';
import {loadLatticeRuntimeConfiguration,closedChildEnvironment} from '../apps/lattice-control/src/lattice-runtime-health.mjs';

export const lifecycleRoot=join(homedir(),'AppData','Local','LATTICE','bot-lifecycle-postgres','v1');
export const lifecycleIdentityPath=join(lifecycleRoot,'identity.json');
const pgBin='C:/Program Files/PostgreSQL/17/bin';
const data=join(lifecycleRoot,'data'),log=join(lifecycleRoot,'postgres.log');
const action=process.argv[2];
if(process.argv.length!==3 || !['init','start','status','stop'].includes(action))throw Error('usage: node scripts/bot-lifecycle-postgres.mjs init|start|status|stop');
const {environment}=await loadLatticeRuntimeConfiguration();
const env={...closedChildEnvironment(environment),PGPASSWORD:environment.LATTICE_TASK019_PASSWORD};
function command(bin,args,input){return spawnSync(bin,args,{input,env,encoding:'utf8',windowsHide:true,timeout:30000,...(bin.endsWith('pg_ctl.exe')?{stdio:'ignore'}:{})});}
function pg(name,args,input){const r=command(join(pgBin,`${name}.exe`),args,input);if(r.status!==0)throw Error(`BOT_LIFECYCLE_${name.toUpperCase()}_FAILED_${r.status}`);return (r.stdout??'').trim();}
function psql(marker,sql){return pg('psql',['-X','-h','127.0.0.1','-p',String(marker.port),'-U','runtime_bootstrap','-d','postgres','-A','-t','-v','ON_ERROR_STOP=1'],sql);}
function writeIdentity(marker){writeFileSync(lifecycleIdentityPath,JSON.stringify(marker,null,2)+'\n');}
function loadIdentity(){
 const m=JSON.parse(readFileSync(lifecycleIdentityPath,'utf8'));
 if(m.schemaVersion!==1||m.host!=='127.0.0.1'||m.dataDirectory!==data||m.pgBin!==pgBin||!Number.isInteger(m.port)||m.port<1024||m.port>65535||!/^[a-f0-9]{32}$/.test(m.runId))throw Error('BOT_LIFECYCLE_CLUSTER_IDENTITY_REJECTED');
 return m;
}
function running(){return command(join(pgBin,'pg_ctl.exe'),['-D',data,'status']).status===0;}
function start(){if(!running())pg('pg_ctl',['-D',data,'-l',log,'-w','-t','20','start']);}
function verify(marker){
 const observed=JSON.parse(psql(marker,"SELECT json_build_object('systemIdentifier',system_identifier::text,'dataDirectory',current_setting('data_directory'),'port',current_setting('port')::int,'listen',current_setting('listen_addresses')) FROM pg_control_system();"));
 if(resolve(observed.dataDirectory)!==resolve(data)||observed.port!==marker.port||observed.listen!=='127.0.0.1'||observed.systemIdentifier!==marker.systemIdentifier)throw Error('BOT_LIFECYCLE_CLUSTER_IDENTITY_REJECTED');
 return observed;
}
if(action==='init'){
 if(existsSync(lifecycleRoot))throw Error('BOT_LIFECYCLE_CLUSTER_EXISTS_USE_START_OR_REVIEW_PARTIAL');
 const port=await new Promise((accept,reject)=>{const s=createServer();s.on('error',reject);s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>accept(p));});});
 mkdirSync(lifecycleRoot,{recursive:true});
 // Restrict the task-owned folder before briefly writing initdb's password input.
 const identity=command('whoami.exe',[]).stdout.trim();
 if(!identity)throw Error('BOT_LIFECYCLE_LOCAL_IDENTITY_MISSING');
 const acl=command('icacls.exe',[lifecycleRoot,'/inheritance:r','/grant:r',`${identity}:(OI)(CI)F`,'SYSTEM:(OI)(CI)F']);
 if(acl.status!==0)throw Error('BOT_LIFECYCLE_PRIVATE_DIRECTORY_REJECTED');
 const passwordFile=join(lifecycleRoot,`initdb-password-${randomUUID()}.tmp`);
 try{writeFileSync(passwordFile,environment.LATTICE_TASK019_PASSWORD+'\n',{flag:'wx'});pg('initdb',['-D',data,'--username=runtime_bootstrap','--auth=scram-sha-256','--encoding=UTF8','--locale=C',`--pwfile=${passwordFile}`]);}
 finally{if(existsSync(passwordFile))unlinkSync(passwordFile);}
 appendFileSync(join(data,'postgresql.conf'),`\nlisten_addresses = '127.0.0.1'\nport = ${port}\n`);
 const marker={schemaVersion:1,host:'127.0.0.1',port,runId:randomUUID().replaceAll('-',''),dataDirectory:data,pgBin,systemIdentifier:null,initialized:false};
 writeIdentity(marker);start();
 marker.systemIdentifier=psql(marker,'SELECT system_identifier::text FROM pg_control_system();');
 const quoted="'"+environment.LATTICE_TASK019_PASSWORD.replaceAll("'","''")+"'";
 psql(marker,`BEGIN; CREATE ROLE lattice_migrator NOLOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; CREATE ROLE lattice_runtime NOLOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; CREATE ROLE lattice_migrator_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD ${quoted}; CREATE ROLE lattice_runtime_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD ${quoted}; GRANT lattice_migrator TO lattice_migrator_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; GRANT lattice_runtime TO lattice_runtime_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; REVOKE ALL ON DATABASE postgres FROM PUBLIC; REVOKE ALL ON DATABASE template1 FROM PUBLIC; REVOKE ALL ON DATABASE template0 FROM PUBLIC; COMMIT;`);
 marker.initialized=true;writeIdentity(marker);verify(marker);console.log(JSON.stringify({status:'INITIALIZED',...marker}));
}else{
 const marker=loadIdentity();if(!marker.initialized)throw Error('BOT_LIFECYCLE_PARTIAL_CLUSTER_REVIEW_REQUIRED');
 if(action==='start')start();
 if(action==='stop'){
   if(running()){verify(marker);pg('pg_ctl',['-D',data,'-m','fast','-w','-t','20','stop']);}
   if(running())throw Error('BOT_LIFECYCLE_STOP_NOT_CONFIRMED');
   console.log(JSON.stringify({status:'STOPPED',...marker}));
 }else{
   if(!running())throw Error('BOT_LIFECYCLE_CLUSTER_STOPPED');
   verify(marker);console.log(JSON.stringify({status:'RUNNING_IDENTITY_VERIFIED',...marker}));
 }
}
