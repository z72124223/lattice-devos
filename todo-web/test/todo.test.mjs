import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm, rmdir } from 'node:fs/promises';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { once } from 'node:events';

const root = fileURLToPath(new URL('../', import.meta.url));
const scratch = join(root, '.test-tmp');

async function fixture(t) {
  await mkdir(scratch, { recursive: true });
  const directory = await mkdtemp(join(scratch, 'todo-'));
  t.after(async () => {
    assert.ok(resolve(directory).startsWith(resolve(scratch) + sep));
    await rm(directory, { recursive: true, force: true });
  });
  return { directory, dataFile: join(directory, 'todos.json') };
}

async function app(t, dataFile) {
  const { startServer } = await import('../server.mjs');
  const server = await startServer({ port: 0, dataFile });
  t.after(() => new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve())));
  const base = `http://127.0.0.1:${server.address().port}`;
  return { server, base, get: () => fetch(base) };
}

function post(base, route, fields, headers = {}) {
  return fetch(base + route, {
    method: 'POST', redirect: 'manual',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded', Origin: base, ...headers },
    body: new URLSearchParams(fields),
  });
}

const saved = async dataFile => JSON.parse(await readFile(dataFile, 'utf8'));

test('繁中網頁、表單與 CSS 可存取，HTTP 只綁定 127.0.0.1', async t => {
  const { dataFile } = await fixture(t);
  const { server, base, get } = await app(t, dataFile);
  assert.equal(server.address().address, '127.0.0.1');
  const response = await get();
  assert.equal(response.status, 200);
  assert.match(response.headers.get('content-type'), /text\/html.*utf-8/i);
  assert.equal(response.headers.get('cache-control'), 'no-store');
  const html = await response.text();
  assert.match(html, /lang="zh-Hant"/);
  assert.match(html, /<form[^>]+action="\/todos"/);
  assert.match(html, /新增待辦/);
  assert.match(html, /目前沒有待辦/);
  assert.equal((await fetch(base + '/style.css')).status, 200);
  assert.equal((await fetch(base + '/data/todos.json')).status, 404);
  assert.equal((await fetch(base + '/server.mjs')).status, 404);
});

test('新增、完成、重開及重新整理保留同一筆資料；文字安全呈現', async t => {
  const { dataFile } = await fixture(t);
  const { base, get } = await app(t, dataFile);
  const title = '買牛奶 <script>alert(1)</script> & 麵包';
  let response = await post(base, '/todos', { text: `  ${title}  ` });
  assert.equal(response.status, 303);
  assert.equal(response.headers.get('location'), '/');
  let records = await saved(dataFile);
  assert.equal(records.length, 1);
  assert.equal(records[0].text, title);
  assert.equal(records[0].completed, false);
  const id = records[0].id;
  let html = await (await get()).text();
  assert.match(html, /&lt;script&gt;alert\(1\)&lt;\/script&gt; &amp;/);
  assert.ok(!html.includes('<script>alert(1)</script>'));
  assert.ok(html.includes(`action="/todos/${id}/state"`));
  assert.match(html, /標記完成/);
  response = await post(base, `/todos/${id}/state`, { completed: 'true' });
  assert.equal(response.status, 303);
  records = await saved(dataFile);
  assert.equal(records[0].completed, true);
  assert.equal(records[0].id, id);
  html = await (await get()).text();
  assert.match(html, /重新開啟/);
  assert.match(html, /已完成 1/);
  response = await post(base, `/todos/${id}/state`, { completed: 'false' });
  assert.equal(response.status, 303);
  for (let i = 0; i < 2; i++) {
    html = await (await get()).text();
    assert.match(html, /標記完成/);
    assert.match(html, /待完成 1/);
  }
  records = await saved(dataFile);
  assert.equal(records.length, 1);
  assert.equal(records[0].completed, false);
});

test('空字串、各類空白、缺欄位、重複欄位和過長內容皆拒絕，不更動資料', async t => {
  const { dataFile } = await fixture(t);
  const { base } = await app(t, dataFile);
  assert.equal((await post(base, '/todos', { text: '保留此項' })).status, 303);
  const before = await readFile(dataFile, 'utf8');
  for (const text of ['', ' \t\n', '\u3000\u00a0', '\u200b\u200d', '字'.repeat(201)]) {
    const response = await post(base, '/todos', { text });
    assert.equal(response.status, 400, JSON.stringify(text));
    assert.match(await response.text(), /待辦|字/);
  }
  assert.equal((await post(base, '/todos', {})).status, 400);
  assert.equal((await post(base, '/todos', [['text', 'A'], ['text', 'B']])).status, 400);
  assert.equal(await readFile(dataFile, 'utf8'), before);
});

test('無效 ID、非法狀態、未知路徑及 HTTP 方法不會改動資料', async t => {
  const { dataFile } = await fixture(t);
  const { base } = await app(t, dataFile);
  await post(base, '/todos', { text: '不能被錯誤修改' });
  const before = await readFile(dataFile, 'utf8');
  const [{ id }] = JSON.parse(before);
  for (const completed of ['yes', '', '1', 'null']) {
    assert.equal((await post(base, `/todos/${id}/state`, { completed })).status, 400);
  }
  assert.equal((await post(base, '/todos/missing/state', { completed: 'true' })).status, 404);
  assert.equal((await post(base, '/unknown', {})).status, 404);
  assert.equal((await fetch(base + `/todos/${id}/state`)).status, 405);
  assert.equal((await fetch(base, { method: 'DELETE' })).status, 405);
  assert.equal(await readFile(dataFile, 'utf8'), before);
});

test('拒絕不支援格式、過大表單及跨來源寫入，伺服器可繼續操作', async t => {
  const { dataFile } = await fixture(t);
  const { base } = await app(t, dataFile);
  assert.equal((await fetch(base + '/todos', {
    method: 'POST', headers: { 'Content-Type': 'application/json', Origin: base }, body: '{bad json',
  })).status, 415);
  assert.equal((await post(base, '/todos', { text: 'x'.repeat(10000) })).status, 413);
  assert.equal((await post(base, '/todos', { text: '外部' }, { Origin: 'https://example.invalid' })).status, 403);
  assert.equal((await post(base, '/todos', { text: '正常' })).status, 303);
  assert.deepEqual((await saved(dataFile)).map(todo => todo.text), ['正常']);
});

test('並行新增無遺失，重複完成操作具冪等性', async t => {
  const { dataFile } = await fixture(t);
  const { base } = await app(t, dataFile);
  const results = await Promise.all(Array.from({ length: 12 }, (_, i) => post(base, '/todos', { text: `工作 ${i}` })));
  assert.ok(results.every(response => response.status === 303));
  const records = await saved(dataFile);
  assert.equal(records.length, 12);
  assert.equal(new Set(records.map(todo => todo.id)).size, 12);
  assert.equal(new Set(records.map(todo => todo.text)).size, 12);
  const route = `/todos/${records[0].id}/state`;
  assert.equal((await post(base, route, { completed: 'true' })).status, 303);
  assert.equal((await post(base, route, { completed: 'true' })).status, 303);
  assert.equal((await saved(dataFile))[0].completed, true);
});

test('資料損壞時拒絕啟動，原始檔保留', async t => {
  const { directory, dataFile } = await fixture(t);
  const { startServer } = await import('../server.mjs');
  for (const content of ['{invalid', '{}', '[{"id":"bad","text":"","completed":false}]']) {
    await writeFile(dataFile, content);
    await assert.rejects(startServer({ port: 0, dataFile }), /資料/);
    assert.equal(await readFile(dataFile, 'utf8'), content);
  }
  assert.equal(dirname(dataFile), directory);
});

test('保存失敗不回成功、不發布未落盤資料，恢復後仍可新增', async t => {
  const { dataFile } = await fixture(t);
  const { base, get } = await app(t, dataFile);
  // 在測試專用目錄以同名資料夾阻擋原子更名，跨 Windows/Linux 皆可重現。
  await mkdir(dataFile);
  const response = await post(base, '/todos', { text: '不應出現' });
  assert.equal(response.status, 500);
  assert.match(await response.text(), /保存/);
  assert.ok(!(await (await get()).text()).includes('不應出現'));
  await rmdir(dataFile);
  assert.equal((await post(base, '/todos', { text: '恢復成功' })).status, 303);
  assert.deepEqual((await saved(dataFile)).map(todo => todo.text), ['恢復成功']);
});

async function childServer(t, dataFile) {
  const child = spawn(process.execPath, [join(root, 'server.mjs')], {
    cwd: root, env: { ...process.env, PORT: '0', TODO_DATA_FILE: dataFile },
    windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  });
  const exited = once(child, 'exit');
  t.after(async () => { if (child.exitCode === null && child.signalCode === null) child.kill(); await exited; });
  let output = '';
  let errorOutput = '';
  child.stderr.setEncoding('utf8').on('data', chunk => { errorOutput += chunk; });
  const base = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('啟動逾時: ' + errorOutput)), 10000);
    child.once('error', error => { clearTimeout(timer); reject(error); });
    child.once('exit', code => { clearTimeout(timer); reject(new Error(`提早結束 ${code}: ${errorOutput}`)); });
    child.stdout.setEncoding('utf8').on('data', chunk => {
      output += chunk;
      const match = output.match(/http:\/\/127\.0\.0\.1:\d+/);
      if (match) { clearTimeout(timer); resolve(match[0]); }
    });
  });
  return { base, stop: async () => { child.kill(); await exited; } };
}

test('真實 Node CLI 程序終止再啟動後保留文字與完成狀態', async t => {
  const { dataFile } = await fixture(t);
  const first = await childServer(t, dataFile);
  assert.equal((await post(first.base, '/todos', { text: '重啟後繼續' })).status, 303);
  const [{ id }] = await saved(dataFile);
  assert.equal((await post(first.base, `/todos/${id}/state`, { completed: 'true' })).status, 303);
  await first.stop();
  const second = await childServer(t, dataFile);
  const html = await (await fetch(second.base)).text();
  assert.match(html, /重啟後繼續/);
  assert.match(html, /重新開啟/);
  assert.equal((await saved(dataFile))[0].completed, true);
  assert.equal((await post(second.base, `/todos/${id}/state`, { completed: 'false' })).status, 303);
  assert.equal((await saved(dataFile))[0].completed, false);
});
