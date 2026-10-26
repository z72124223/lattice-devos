import { createServer } from 'node:http';
import { readFile, mkdir, open, rename, unlink } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';

const ROOT = fileURLToPath(new URL('./', import.meta.url));
const HOST = '127.0.0.1';
const MAX_TEXT = 200;
const MAX_BODY = 8192;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

class UserError extends Error {
  constructor(status, message) { super(message); this.status = status; }
}

const escape = value => String(value).replace(/[&<>"']/g, character => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
})[character]);

function validText(text) {
  return typeof text === 'string' &&
    text.replace(/[\s\u200b-\u200d\u2060\ufeff]/gu, '').length > 0 &&
    [...text.trim()].length <= MAX_TEXT;
}

async function loadStore(dataFile) {
  let todos = [];
  try {
    todos = JSON.parse(await readFile(dataFile, 'utf8'));
    const ids = new Set();
    if (!Array.isArray(todos) || !todos.every(todo => {
      if (!todo || typeof todo.id !== 'string' || !UUID.test(todo.id) || ids.has(todo.id) ||
        !validText(todo.text) || typeof todo.completed !== 'boolean' ||
        typeof todo.createdAt !== 'string' || !Number.isFinite(Date.parse(todo.createdAt)) ||
        typeof todo.updatedAt !== 'string' || !Number.isFinite(Date.parse(todo.updatedAt))) return false;
      ids.add(todo.id);
      return true;
    })) throw new Error('invalid data');
  } catch (error) {
    if (error.code !== 'ENOENT') throw new Error('資料檔無法讀取或格式損壞，已保留原檔；請檢查資料檔或還原備份。');
  }
  await mkdir(dirname(dataFile), { recursive: true });
  let pending = Promise.resolve();

  return {
    async list() { await pending; return todos; },
    change(transform) {
      const operation = pending.then(async () => {
        const next = transform(todos);
        const temporary = `${dataFile}.${randomUUID()}.tmp`;
        let file;
        try {
          file = await open(temporary, 'wx', 0o600);
          await file.writeFile(JSON.stringify(next, null, 2) + '\n', 'utf8');
          await file.sync();
          await file.close();
          file = undefined;
          await rename(temporary, dataFile);
          // Only publish the new state after the complete file is saved.
          todos = next;
        } catch {
          throw new UserError(500, '待辦保存失敗，這次變更尚未套用。請檢查資料夾權限或可用空間後再試。');
        } finally {
          if (file) await file.close().catch(() => {});
          await unlink(temporary).catch(() => {});
        }
      });
      // A rejected mutation must not poison later requests or reads.
      pending = operation.catch(() => {});
      return operation;
    },
  };
}

async function readForm(request) {
  if (request.headers['content-type']?.split(';')[0].trim().toLowerCase() !== 'application/x-www-form-urlencoded') {
    throw new UserError(415, '請使用網頁表單送出待辦。');
  }
  let length = 0;
  let tooLarge = false;
  const chunks = [];
  for await (const chunk of request) {
    length += chunk.length;
    if (length > MAX_BODY) tooLarge = true;
    if (!tooLarge) chunks.push(chunk);
  }
  if (tooLarge) throw new UserError(413, '送出的資料太大，待辦最多 200 字。');
  return new URLSearchParams(Buffer.concat(chunks).toString('utf8'));
}

function field(form, name) {
  if (form.getAll(name).length !== 1 || [...form.keys()].some(key => key !== name)) {
    throw new UserError(400, '待辦表單欄位不正確，請重新操作。');
  }
  return form.get(name);
}

function page(todos, error = '', draft = '') {
  const done = todos.filter(todo => todo.completed).length;
  const remaining = todos.length - done;
  const date = new Intl.DateTimeFormat('zh-TW', {
    timeZone: 'Asia/Taipei', month: 'long', day: 'numeric', weekday: 'long',
  }).format(new Date());
  const item = todo => `<li class="todo${todo.completed ? ' is-done' : ''}">
    <span class="state-icon" aria-hidden="true">${todo.completed ? '✓' : ''}</span>
    <div class="todo-copy"><span class="todo-text">${escape(todo.text)}</span>
      <span class="state-label">${todo.completed ? '已完成' : '待完成'}</span></div>
    <form method="post" action="/todos/${todo.id}/state">
      <input type="hidden" name="completed" value="${!todo.completed}">
      <button class="state-button" type="submit">${todo.completed ? '重新開啟' : '標記完成'}</button>
    </form></li>`;
  return `<!doctype html>
<html lang="zh-Hant">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>一件件｜今天的待辦</title><link rel="stylesheet" href="/style.css"></head>
<body><div class="shell">
  <header class="topbar"><a class="brand" href="/"><span class="brand-icon" aria-hidden="true">✓</span> 一件件</a>
    <span class="local-badge"><span aria-hidden="true">●</span> 本機保存</span></header>
  <main id="main">
    <section class="intro" aria-labelledby="heading"><p class="date">${escape(date)}</p>
      <h1 id="heading">把今天，<br><span>一件件完成。</span></h1>
      <p class="subtitle">記下想做的事，照自己的步調前進。</p></section>
    <div class="stats" aria-label="待辦進度">
      <div><span class="stat-number">${remaining}</span><span>待完成 ${remaining}</span></div>
      <div><span class="stat-number">${done}</span><span>已完成 ${done}</span></div>
      <p>${todos.length ? (remaining ? '每完成一件，就向前一點。' : '都完成了，給自己一點休息。') : '從一件小事開始。'}</p>
    </div>
    <section class="board" aria-labelledby="list-heading">
      <div class="board-heading"><h2 id="list-heading">今天的待辦</h2><span>共 ${todos.length} 件</span></div>
      ${error ? `<div class="error" role="alert">${escape(error)}</div>` : ''}
      <form class="add-form" method="post" action="/todos">
        <label for="todo-text">新增待辦</label>
        <div class="input-row"><input id="todo-text" name="text" type="text" required maxlength="200"
          placeholder="接下來，想完成什麼？" value="${escape(draft)}" aria-describedby="input-hint" autocomplete="off">
          <button class="add-button" type="submit"><span aria-hidden="true">＋</span> 新增待辦</button></div>
        <p class="hint" id="input-hint">最多 200 字，不能只有空白。按 Enter 即可新增。</p>
      </form>
      ${todos.length ? `<ul class="todo-list" aria-label="待辦清單">${[
        ...todos.filter(todo => !todo.completed), ...todos.filter(todo => todo.completed),
      ].map(item).join('')}</ul>` : `<div class="empty"><span class="empty-icon" aria-hidden="true">✓</span>
        <h3>目前沒有待辦</h3><p>把第一件想做的事，寫在上方吧。</p></div>`}
    </section>
    <p class="save-note">變更成功即自動保存；重新整理或重啟後，待辦都還在。</p>
  </main><footer><span>一步一步，也是在前進。</span><a href="/">重新整理清單 ↻</a></footer>
</div></body></html>`;
}

function send(response, status, content, type = 'text/html; charset=utf-8', extra = {}) {
  response.writeHead(status, {
    'Content-Type': type, 'Cache-Control': 'no-store',
    'Content-Security-Policy': "default-src 'none'; style-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
    // no-referrer makes native (non-CORS) form POSTs send Origin: null.
    // Keep same-origin form provenance without sending referrers to other sites.
    'X-Content-Type-Options': 'nosniff', 'Referrer-Policy': 'same-origin', ...extra,
  });
  response.end(content);
}

export async function startServer({ port = 3000, dataFile = resolve(ROOT, 'data/todos.json') } = {}) {
  if (!Number.isInteger(port) || port < 0 || port > 65535) throw new Error('PORT 必須是 0 到 65535 的整數。');
  const store = await loadStore(resolve(dataFile));
  const css = await readFile(new URL('./public/style.css', import.meta.url), 'utf8');
  const server = createServer(async (request, response) => {
    let draft = '';
    try {
      const authority = `${HOST}:${server.address().port}`;
      if (request.headers.host !== authority) throw new UserError(403, '請使用啟動訊息中的 127.0.0.1 網址。');
      const origin = request.headers.origin;
      const fetchSite = request.headers['sec-fetch-site'];
      // Never trust null or a missing Origin for a write. Metadata may only
      // tighten the exact-origin check; clients without it still need Origin.
      if ((origin !== undefined && origin !== `http://${authority}`) ||
        (request.method === 'POST' && (origin === undefined ||
          (fetchSite !== undefined && fetchSite !== 'same-origin')))) {
        throw new UserError(403, '不接受其他網站送出的操作，請回到本機待辦頁面。');
      }
      const url = new URL(request.url, `http://${authority}`);
      const route = url.pathname;
      const stateMatch = route.match(/^\/todos\/([^/]+)\/state$/);
      if (route !== '/' && route !== '/style.css' && route !== '/todos' && !stateMatch) {
        throw new UserError(404, '找不到這個頁面。');
      }
      const expectedMethod = route === '/' || route === '/style.css' ? 'GET' : 'POST';
      if (request.method !== expectedMethod) {
        send(response, 405, page(await store.list(), '此操作方式不支援，請使用頁面按鈕。'), undefined, { Allow: expectedMethod });
        return;
      }
      if (route === '/') return send(response, 200, page(await store.list()));
      if (route === '/style.css') return send(response, 200, css, 'text/css; charset=utf-8');
      const form = await readForm(request);
      if (route === '/todos') {
        draft = field(form, 'text').trim();
        if (!validText(draft)) throw new UserError(400, '請輸入待辦內容，不能只有空白，最多 200 字。');
        await store.change(todos => {
          const now = new Date().toISOString();
          return [...todos, { id: randomUUID(), text: draft, completed: false, createdAt: now, updatedAt: now }];
        });
      } else {
        const value = field(form, 'completed');
        if (value !== 'true' && value !== 'false') throw new UserError(400, '待辦狀態不正確，請重新操作。');
        await store.change(todos => {
          if (!todos.some(todo => todo.id === stateMatch[1])) throw new UserError(404, '找不到這筆待辦，請重新整理清單。');
          return todos.map(todo => todo.id === stateMatch[1]
            ? { ...todo, completed: value === 'true', updatedAt: new Date().toISOString() } : todo);
        });
      }
      // Post/Redirect/Get prevents browser refresh from repeating a successful write.
      send(response, 303, '', 'text/plain; charset=utf-8', { Location: '/' });
    } catch (error) {
      if (!response.destroyed && !response.headersSent) {
        send(response, error instanceof UserError ? error.status : 500,
          page(await store.list(), error instanceof UserError ? error.message : '暫時無法處理，請稍後再試。', draft));
      }
    }
  });
  server.requestTimeout = 15000;
  server.headersTimeout = 10000;
  server.setTimeout(15000, socket => socket.destroy());
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, HOST, () => { server.removeListener('error', reject); resolve(); });
  });
  return server;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const portValue = process.env.PORT ?? '3000';
    if (!/^\d+$/.test(portValue)) throw new Error('PORT 必須是 0 到 65535 的整數。');
    const server = await startServer({
      port: Number(portValue),
      dataFile: process.env.TODO_DATA_FILE || resolve(ROOT, 'data/todos.json'),
    });
    process.stdout.write(`待辦清單已啟動：http://${HOST}:${server.address().port}\n按 Ctrl+C 停止；資料會保留。\n`);
    for (const signal of ['SIGINT', 'SIGTERM']) {
      process.once(signal, () => server.close(() => { process.exitCode = 0; }));
    }
  } catch (error) {
    process.stderr.write(`無法啟動：${error.code === 'EADDRINUSE' ? '連接埠已被使用，請更換 PORT 或停止原有程序。' : error.message}\n`);
    process.exitCode = 1;
  }
}
