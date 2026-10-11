import assert from 'node:assert/strict';
import { createHmac } from 'node:crypto';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';
import test from 'node:test';
import { validRequest, runSharedControl, sharedControlRequest, windowsProtect, publishSharedControlState } from '../src/shared-control.mjs';

const identity = { pid: process.pid, createdAt: 'fixture-created-at', executable: process.execPath, ownerSid: 'fixture-owner' };
const inspect = async () => identity;
// Unit fixture only. Real CurrentUser encryption is exercised separately below.
const protect = async (mode, value, entropy) => {
  if (mode === 'protect') return JSON.stringify({ value, entropy });
  const decoded = JSON.parse(value);
  assert.equal(decoded.entropy, entropy, 'protected metadata changed');
  return decoded.value;
};

test('authenticated shutdown binds operation, nonce, owner and exact process instance', () => {
  const record = { identity, token: 'a'.repeat(64) };
  const body = { schema: 'lattice.shared-control.v1', operation: 'stop', identity, nonce: 'b'.repeat(64) };
  const frame = { ...body, signature: createHmac('sha256', Buffer.from(record.token, 'hex')).update(JSON.stringify(body)).digest('hex') };
  assert.equal(validRequest(frame, record), true);
  for (const field of ['pid','createdAt','executable','ownerSid']) {
    assert.equal(validRequest({ ...frame, identity: { ...identity, [field]: 'changed' } }, record), false);
  }
  for (const changed of [{ signature: '0'.repeat(64) }, { operation: 'status' }, { operation: 'kill' }, { nonce: 'c'.repeat(64) }, { extra: true }]) {
    assert.equal(validRequest({ ...frame, ...changed }, record), false);
  }
});

async function fixture(t, shutdown, writeState = publishSharedControlState) {
  const parent = path.resolve('.lattice/shared-control-tests'); await mkdir(parent, { recursive: true });
  const root = await mkdtemp(path.join(parent, 'case-'));
  const file = path.join(root, 'control.json');
  const previous = process.env.LATTICE_CONTROL_DATABASE_PATH;
  process.env.LATTICE_CONTROL_DATABASE_PATH = path.join(root, 'control.sqlite');
  const host = runSharedControl(file, { inspect, protect, writeState, start: async options => {
    assert.equal(options.autoRestore, false);
    return { server: { address: () => ({ port: 23456 }) }, shutdownOwned: shutdown };
  } });
  let startupError;
  host.catch(error => { startupError = error; });
  t.after(async () => {
    await sharedControlRequest(file, 'stop', { inspect, protect }).catch(() => {});
    await host;
    if (previous === undefined) delete process.env.LATTICE_CONTROL_DATABASE_PATH; else process.env.LATTICE_CONTROL_DATABASE_PATH = previous;
    await rm(root, { recursive: true, force: true });
  });
  let record;
  for (let tries = 0; tries < 100; tries++) {
    if (startupError) throw startupError;
    try { record = JSON.parse(await readFile(file, 'utf8')); if (record.status === 'RUNNING') break; } catch { /* starting */ }
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  assert.equal(record?.status, 'RUNNING');
  return { file, record, host };
}

test('client disconnect does not stop Control; metadata mismatch and duplicate launch fail closed', async t => {
  let stopCalls = 0;
  const { file, record, host } = await fixture(t, async () => { stopCalls++; });
  const disconnected = net.createConnection({ host: '127.0.0.1', port: record.channelPort });
  await new Promise(resolve => disconnected.once('connect', resolve)); disconnected.destroy();
  assert.equal((await sharedControlRequest(file, 'status', { inspect, protect })).status, 'RUNNING');
  assert.equal(stopCalls, 0);
  const rawRequest = build => new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: '127.0.0.1', port: record.channelPort });
    let data = '', sent = false;
    socket.on('error', () => resolve({ status: 'CLOSED' }));
    socket.on('data', chunk => {
      data += chunk;
      if (!sent && data.includes('\n')) {
        const end = data.indexOf('\n'), greeting = JSON.parse(data.slice(0, end));
        data = data.slice(end + 1); sent = true;
        socket.write(JSON.stringify(build(greeting.challenge)) + '\n');
      }
    });
    socket.on('end', () => { try { resolve(data ? JSON.parse(data) : { status: 'CLOSED' }); } catch(error) { reject(error); } });
  });
  let captured;
  await rawRequest(nonce => {
    const body = { schema: record.schema, operation: 'status', identity, nonce };
    const key = JSON.parse(record.protectedToken).value;
    captured = { ...body, signature: createHmac('sha256', Buffer.from(key, 'hex')).update(JSON.stringify(body)).digest('hex') };
    return captured;
  });
  assert.equal((await rawRequest(nonce => { assert.notEqual(nonce, captured.nonce); return captured; })).status, 'REJECTED');
  for (const malformed of [null, [], 'x', 1]) assert.equal((await rawRequest(() => malformed)).status, 'REJECTED');
  assert.equal((await rawRequest(() => 'x'.repeat(3000))).status, 'CLOSED');
  assert.equal((await sharedControlRequest(file, 'status', { inspect, protect })).status, 'RUNNING');
  await assert.rejects(sharedControlRequest(file, 'stop', { protect, inspect: async () => ({ ...identity, createdAt: 'reused-pid' }) }), /IDENTITY_CHANGED/);
  await writeFile(file, JSON.stringify({ ...record, channelPort: record.channelPort === 65535 ? 65534 : record.channelPort + 1 }));
  await assert.rejects(sharedControlRequest(file, 'stop', { inspect, protect }), /protected metadata changed/);
  await writeFile(file, JSON.stringify(record));
  assert.equal(stopCalls, 0);
  await assert.rejects(runSharedControl(file, { inspect, protect, start: () => { throw Error('must not start twice'); } }), { code: 'EEXIST' });
  assert.equal((await sharedControlRequest(file, 'stop', { inspect, protect })).status, 'STOPPED');
  await host; assert.equal(stopCalls, 1);
  assert.equal((await sharedControlRequest(file, 'status', { inspect, protect })).status, 'STOPPED');
  assert.equal(JSON.parse(await readFile(file, 'utf8')).token, undefined);
  const terminal = JSON.parse(await readFile(file, 'utf8'));
  await writeFile(file, JSON.stringify({ ...terminal, stoppedAt: 'forged' }));
  await assert.rejects(sharedControlRequest(file, 'status', { inspect, protect }), /STATE_INVALID/);
  await writeFile(file, JSON.stringify(terminal));
});

test('shutdown responses reflect the application outcome, including a failed attempt', async t => {
  let calls = 0;
  const { file, host } = await fixture(t, async () => { if (++calls === 1) throw Error('fixture shutdown failure'); });
  assert.equal((await sharedControlRequest(file, 'stop', { inspect, protect })).status, 'SHUTDOWN_INCOMPLETE');
  assert.equal((await sharedControlRequest(file, 'status', { inspect, protect })).status, 'SHUTDOWN_INCOMPLETE');
  assert.equal((await sharedControlRequest(file, 'stop', { inspect, protect })).status, 'STOPPED');
  await host; assert.equal(calls, 2);
});

test('successful shutdown with failed terminal receipt exits honestly without shutting down twice', async t => {
  let calls = 0;
  const { file, host } = await fixture(t, async () => { calls++; }, async (file, value, first) => {
    if (value.status === 'STOPPED') throw new Error('fixture receipt write failure');
    return publishSharedControlState(file, value, first);
  });
  assert.equal((await sharedControlRequest(file, 'stop', { inspect, protect })).status, 'STOPPED_RECEIPT_UNWRITTEN');
  await host;
  assert.equal(calls, 1);
  assert.equal(JSON.parse(await readFile(file, 'utf8')).status, 'RUNNING');
});

test('real Windows DPAPI roundtrip rejects changed binding', { skip: process.platform !== 'win32' }, async () => {
  const key = 'e'.repeat(64);
  const encrypted = await windowsProtect('protect', key, 'fixture-process-binding');
  assert.equal(encrypted.includes(key), false);
  assert.equal(await windowsProtect('unprotect', encrypted, 'fixture-process-binding'), key);
  await assert.rejects(windowsProtect('unprotect', encrypted, 'changed-process-binding'), /KEY_PROTECTION_FAILED/);
});

test('failed atomic replacement preserves the prior receipt and does not block a later write', async t => {
  const parent = path.resolve('.lattice/shared-control-tests'); await mkdir(parent, { recursive: true });
  const root = await mkdtemp(path.join(parent, 'case-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const file = path.join(root, 'control.json');
  await publishSharedControlState(file, { status: 'RUNNING' }, true);
  let attempts = 0;
  await assert.rejects(publishSharedControlState(file, { status: 'STOPPED' }, false, async () => {
    attempts++; throw Object.assign(new Error('fixture sharing lock'), { code: 'EPERM' });
  }), /fixture sharing lock/);
  assert.equal(attempts, process.platform === 'win32' ? 10 : 1);
  assert.equal(JSON.parse(await readFile(file, 'utf8')).status, 'RUNNING');
  assert.equal((await readdir(root)).filter(name => name.includes('.next.')).length, 1);
  await publishSharedControlState(file, { status: 'STOPPED' });
  assert.equal(JSON.parse(await readFile(file, 'utf8')).status, 'STOPPED');
});
