// Windows-only lifecycle for one explicitly launched, shared Control process.
// No scheduler, restart loop, process-tree kill, or platform-policy fallback.
import { createHash, createHmac, randomBytes, timingSafeEqual } from 'node:crypto';
import { execFile } from 'node:child_process';
import { lstat, open, readFile, rename, unlink } from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { startDefaultServer } from './server.mjs';

const exec = promisify(execFile);
const schema = 'lattice.shared-control.v1';
const fail = code => { throw new Error(code); };
const fields = ['pid', 'createdAt', 'executable', 'ownerSid'];
const sameIdentity = (a, b) => fields.every(key => a?.[key] === b?.[key]);
const absolute = value => {
  if (typeof value !== 'string' || !path.isAbsolute(value) || /[\x00-\x1f]/u.test(value)) fail('SHARED_CONTROL_PATH_INVALID');
  return path.resolve(value);
};

// Inspect only. Never change ACLs, owner, Job flags or execution policy.
export async function windowsIdentity(pid) {
  if (process.platform !== 'win32' || !Number.isSafeInteger(pid) || pid < 1) fail('SHARED_CONTROL_WINDOWS_REQUIRED');
  const script = `$ErrorActionPreference='Stop'
$p=Get-CimInstance Win32_Process -Filter ('ProcessId='+$env:LATTICE_INSPECT_PID)
if(-not $p){throw 'SHARED_CONTROL_PROCESS_ABSENT'}
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$owner=(Invoke-CimMethod -InputObject $p -MethodName GetOwnerSid)
if($owner.ReturnValue -ne 0 -or $owner.Sid -ne $sid){throw 'SHARED_CONTROL_OWNER_MISMATCH'}
[pscustomobject]@{pid=[int]$p.ProcessId;createdAt=$p.CreationDate.ToUniversalTime().ToString('o');executable=$p.ExecutablePath;ownerSid=$owner.Sid}|ConvertTo-Json -Compress`;
  const shell = path.join(process.env.SystemRoot, 'System32/WindowsPowerShell/v1.0/powershell.exe');
  try {
    const { stdout } = await exec(shell, ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', script], {
      env: { ...process.env, LATTICE_INSPECT_PID: String(pid) },
      windowsHide: true, timeout: 15000, maxBuffer: 65536,
    });
    return JSON.parse(stdout);
  } catch { fail('SHARED_CONTROL_IDENTITY_OR_ACCESS_REJECTED'); }
}

const binding = (file, record) => createHash('sha256').update(JSON.stringify({ file: file.toLowerCase(),
  schema: record.schema, identity: record.identity, channelPort: record.channelPort, databasePath: record.databasePath })).digest('hex');
const mac = (token, value) => createHmac('sha256', Buffer.from(token, 'hex')).update(JSON.stringify(value)).digest('hex');
const authentic = (signature, expected) => typeof signature === 'string' && /^[a-f0-9]{64}$/u.test(signature)
  && timingSafeEqual(Buffer.from(signature, 'hex'), Buffer.from(expected, 'hex'));
const requestBody = frame => ({ schema: frame.schema, operation: frame.operation, identity: frame.identity, nonce: frame.nonce });
const persisted = record => { const { token, ...value } = record; return value; };
const terminalBody = record => ({ status: record.status, identity: record.identity, stoppedAt: record.stoppedAt });

// DPAPI protects the random service key across Windows accounts. It is not a
// boundary against malicious programs running as the same signed-in account.
export function windowsProtect(mode, value, entropy) {
  if (process.platform !== 'win32' || !['protect', 'unprotect'].includes(mode)) fail('SHARED_CONTROL_WINDOWS_REQUIRED');
  const script = `$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Security
$payload=[Console]::In.ReadToEnd()|ConvertFrom-Json
$entropy=[Text.Encoding]::UTF8.GetBytes($payload.entropy)
if($payload.mode -eq 'protect'){
 $value=[Text.Encoding]::UTF8.GetBytes($payload.value)
 [Convert]::ToBase64String([Security.Cryptography.ProtectedData]::Protect($value,$entropy,[Security.Cryptography.DataProtectionScope]::CurrentUser))
}else{
 $value=[Convert]::FromBase64String($payload.value)
 [Text.Encoding]::UTF8.GetString([Security.Cryptography.ProtectedData]::Unprotect($value,$entropy,[Security.Cryptography.DataProtectionScope]::CurrentUser))
}`;
  return new Promise((resolve, reject) => {
    const child = execFile(path.join(process.env.SystemRoot, 'System32/WindowsPowerShell/v1.0/powershell.exe'),
      ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', script],
      { windowsHide: true, timeout: 15000, maxBuffer: 16384 },
      (error, stdout) => error ? reject(new Error('SHARED_CONTROL_KEY_PROTECTION_FAILED')) : resolve(stdout.trim()));
    child.stdin.on('error', () => {});
    child.stdin.end(JSON.stringify({ mode, value, entropy }));
  });
}

async function safeStatePath(file) {
  file = absolute(file);
  for (let parent = path.dirname(file); ; parent = path.dirname(parent)) {
    const stat = await lstat(parent);
    if (!stat.isDirectory() || stat.isSymbolicLink()) fail('SHARED_CONTROL_PATH_ALIAS');
    if (path.dirname(parent) === parent) break;
  }
  try {
    const stat = await lstat(file);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1 || stat.size > 16384) fail('SHARED_CONTROL_STATE_INVALID');
  } catch (error) { if (error.code !== 'ENOENT') throw error; }
  return file;
}

async function publish(file, value, first = false) {
  const temporary = first ? file : `${file}.next`;
  const handle = await open(temporary, 'wx', 0o600);
  try { await handle.writeFile(JSON.stringify(value)); await handle.sync(); } finally { await handle.close(); }
  if (!first) {
    for (let attempt = 0; ; attempt++) {
      try { await rename(temporary, file); break; }
      catch (error) {
        // A simultaneous Windows reader can briefly hold a sharing lock.
        // Keep replacement atomic; never unlink the destination or change ACLs.
        if (process.platform !== 'win32' || !['EPERM','EBUSY'].includes(error.code) || attempt >= 9) throw error;
        await new Promise(resolve => setTimeout(resolve, 50));
      }
    }
  }
}

export function validRequest(frame, record) {
  if (!frame || Object.keys(frame).sort().join(',') !== 'identity,nonce,operation,schema,signature'
    || frame.schema !== schema || !['status', 'stop'].includes(frame.operation)
    || !sameIdentity(frame.identity, record.identity) || typeof frame.nonce !== 'string'
    || !/^[a-f0-9]{64}$/u.test(frame.nonce)) return false;
  return authentic(frame.signature, mac(record.token, requestBody(frame)));
}

export async function runSharedControl(statePath, { start = startDefaultServer, inspect = windowsIdentity, protect = windowsProtect } = {}) {
  const file = await safeStatePath(statePath);
  const identity = await inspect(process.pid, path.dirname(file));
  const databasePath = absolute(process.env.LATTICE_CONTROL_DATABASE_PATH);
  const lock = await open(`${file}.lock`, 'wx', 0o600);
  let application, channel, stopping = false;
  const record = { schema, identity, status: 'STARTING', token: randomBytes(32).toString('hex'),
    databasePath, startupMode: 'explicit-only' };
  let published = false;
  try {
    // New state paths are explicit. Existing receipts are never overwritten by a
    // duplicate launch, even when the recorded process is no longer available.
    await publish(file, persisted(record), true); published = true;
    application = await start({ autoRestore: false });
    record.port = application.server.address().port;
    channel = net.createServer(socket => {
      socket.setTimeout(2000, () => socket.destroy());
      const challenge = randomBytes(32).toString('hex');
      socket.write(JSON.stringify({ challenge }) + '\n');
      let input = '', handled = false;
      socket.on('error', () => {});
      socket.on('data', chunk => {
        if (handled) return;
        input += chunk.toString('utf8');
        if (Buffer.byteLength(input) > 2048) { handled = true; socket.destroy(); return; }
        if (!input.includes('\n')) return;
        handled = true;
        let frame;
        try { frame = JSON.parse(input.trim()); } catch { socket.end('{"status":"REJECTED"}\n'); return; }
        if (frame?.nonce !== challenge || !validRequest(frame, record)) { socket.end('{"status":"REJECTED"}\n'); return; }
        const reply = result => socket.end(JSON.stringify({ ...result, signature: mac(record.token, { nonce: frame.nonce, result }) }) + '\n');
        if (frame.operation === 'status') {
          reply({ status: record.status, identity, port: record.port, startupMode: record.startupMode }); return;
        }
        if (stopping) { reply({ status: 'STOPPING', identity }); return; }
        stopping = true; record.status = 'STOPPING';
        socket.setTimeout(15000, () => socket.destroy());
        void application.shutdownOwned({ timeoutMs: 10000 }).then(async () => {
          record.status = 'STOPPED'; record.stoppedAt = new Date().toISOString();
          record.terminalSignature = mac(record.token, terminalBody(record));
          await publish(file, persisted(record));
          reply({ status: 'STOPPED', identity });
          channel.close();
        }).catch(async () => {
          record.status = 'SHUTDOWN_INCOMPLETE';
          await publish(file, persisted(record)).catch(() => {});
          reply({ status: 'SHUTDOWN_INCOMPLETE', identity });
          stopping = false;
          // Keep the control channel and process available for inspection. Never kill.
          // The application may retain a failed shutdown promise. Another request
          // reports that outcome; this wrapper does not promise recovery or restart.
        });
      });
    });
    channel.maxConnections = 16;
    await new Promise((resolve, reject) => { channel.once('error', reject); channel.listen(0, '127.0.0.1', resolve); });
    record.channelPort = channel.address().port;
    record.protectedToken = await protect('protect', record.token, binding(file, record));
    record.status = 'RUNNING'; await publish(file, persisted(record));
    await new Promise(resolve => channel.once('close', resolve));
  } catch (error) {
    if (channel?.listening) channel.close();
    if (application) await application.shutdownOwned({ timeoutMs: 10000 }).catch(() => {});
    if (published) { record.status = 'START_FAILED'; await publish(file, persisted(record)).catch(() => {}); }
    throw error;
  } finally { await lock.close(); await unlink(`${file}.lock`); }
}

export async function sharedControlRequest(statePath, operation, { inspect = windowsIdentity, protect = windowsProtect } = {}) {
  if (!['status', 'stop'].includes(operation)) fail('SHARED_CONTROL_OPERATION_INVALID');
  const file = await safeStatePath(statePath), record = JSON.parse(await readFile(file, 'utf8'));
  if (record.schema !== schema || typeof record.protectedToken !== 'string' || record.protectedToken.length > 4096
    || !Number.isInteger(record.channelPort) || record.channelPort < 1024 || record.channelPort > 65535) fail('SHARED_CONTROL_STATE_INVALID');
  if (record.status === 'STOPPED') {
    const stoppedKey = await protect('unprotect', record.protectedToken, binding(file, record));
    if (!/^[a-f0-9]{64}$/u.test(stoppedKey) || !authentic(record.terminalSignature, mac(stoppedKey, terminalBody(record)))) fail('SHARED_CONTROL_STATE_INVALID');
    // Signed receipt of graceful shutdown; it does not replace OS exit readback.
    return { status: 'STOPPED', identity: record.identity };
  }
  const observed = await inspect(record.identity?.pid, path.dirname(file));
  if (!sameIdentity(observed, record.identity)) fail('SHARED_CONTROL_IDENTITY_CHANGED');
  const token = await protect('unprotect', record.protectedToken, binding(file, { ...record, identity: observed }));
  if (!/^[a-f0-9]{64}$/u.test(token)) fail('SHARED_CONTROL_KEY_PROTECTION_FAILED');
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: '127.0.0.1', port: record.channelPort });
    let output = '', body;
    socket.setTimeout(15000, () => socket.destroy(new Error('SHARED_CONTROL_RESPONSE_TIMEOUT')));
    socket.once('error', () => reject(new Error('SHARED_CONTROL_CHANNEL_UNAVAILABLE')));
    socket.on('data', chunk => {
      output += chunk;
      if (output.length > 4096) { socket.destroy(new Error('SHARED_CONTROL_RESPONSE_TOO_LARGE')); return; }
      if (!body && output.includes('\n')) {
        try {
          const split = output.indexOf('\n'), greeting = JSON.parse(output.slice(0, split));
          if (Object.keys(greeting).join(',') !== 'challenge' || !/^[a-f0-9]{64}$/u.test(greeting.challenge)) fail('SHARED_CONTROL_RESPONSE_REJECTED');
          body = { schema, operation, identity: observed, nonce: greeting.challenge };
          output = output.slice(split + 1);
          socket.write(JSON.stringify({ ...body, signature: mac(token, body) }) + '\n');
        } catch { socket.destroy(new Error('SHARED_CONTROL_RESPONSE_REJECTED')); }
      }
    });
    socket.once('end', () => {
      try {
        const { signature, ...result } = JSON.parse(output);
        if (!body || !authentic(signature, mac(token, { nonce: body.nonce, result }))) fail('SHARED_CONTROL_RESPONSE_REJECTED');
        if (!sameIdentity(result.identity, observed) || !['RUNNING','STOPPING','STOPPED','SHUTDOWN_INCOMPLETE'].includes(result.status)) fail('SHARED_CONTROL_RESPONSE_REJECTED');
        resolve(result);
      } catch { reject(new Error('SHARED_CONTROL_RESPONSE_REJECTED')); }
    });
    socket.once('close', () => reject(new Error('SHARED_CONTROL_CHANNEL_CLOSED')));
  });
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [operation, flag, state, ...extra] = process.argv.slice(2);
  if (!['host', 'status', 'stop'].includes(operation) || flag !== '--state' || !state || extra.length) fail('SHARED_CONTROL_ARGUMENTS_INVALID');
  if (operation === 'host') await runSharedControl(state);
  else process.stdout.write(JSON.stringify(await sharedControlRequest(state, operation)) + '\n');
}
