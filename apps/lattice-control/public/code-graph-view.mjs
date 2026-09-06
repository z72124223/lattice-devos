import { codeIndex, codeNeighbors, codeBranches, searchCode, relationLabels } from './code-graph-model.mjs';
const ns = 'http://www.w3.org/2000/svg';
const html = (tag, text, className) => { const el = document.createElement(tag); if (text != null) el.textContent = text;
  if (className) el.className = className; return el; };
const svg = (tag, attrs = {}) => { const el = document.createElementNS(ns, tag);
  for (const [key, value] of Object.entries(attrs)) el.setAttribute(key, value); return el; };
const confidence = { EXTRACTED:'分析找到', INFERRED:'推測關聯', AMBIGUOUS:'尚待確認' };

export function createCodeGraphView({ api, onNavigate, onProjectChange }) {
  const host = document.querySelector('#code-views');
  const projectSelect = host.querySelector('#code-project');
  const search = host.querySelector('#code-search');
  const results = host.querySelector('#code-results');
  const notice = host.querySelector('#code-notice');
  const detail = host.querySelector('#code-detail');
  const canvas = host.querySelector('#code-canvas');
  const refreshButton = host.querySelector('#code-analyze');
  const checkoutSelect = host.querySelector('#code-checkout');
  let checkout = null;
  let projectId = null, loadedProject = null, requestId = 0, timer = null, packet = null, index = null;
  let root = null, selected = null, expanded = new Set(), direction = 'incoming', relation = 'all', zoom = 1;
  const placements = new Map();
  function status(message) { notice.textContent = message; }
  function resetGraph() {
    packet = null; index = null; root = null; selected = null; expanded.clear(); placements.clear();
    canvas.replaceChildren(); results.replaceChildren(); detail.replaceChildren();
    host.querySelector('#code-summary').textContent = '';
    host.querySelector('#code-canvas-note').textContent = '分析資料尚未載入。';
    host.querySelector('#code-source-path').textContent = '';
    checkoutSelect.replaceChildren();
  }
  async function load({ force = false } = {}) {
    if (!projectId || (!force && loadedProject === projectId)) return;
    const requestedProject = projectId, request = ++requestId;
    loadedProject = projectId;
    status('正在讀取這個專案的程式圖譜…');
    try {
      const value = await api('/api/code-graph?projectId=' + encodeURIComponent(projectId)
        + (checkout ? '&checkout=' + encodeURIComponent(checkout) : ''));
      if (request !== requestId || requestedProject !== projectId) return;
      if (value.project_id !== projectId) throw new Error('圖譜不屬於所選專案，已停止顯示。');
      const changed = value.graph?.graph_digest !== packet?.graph?.graph_digest;
      packet = value;
      if (value.checkouts) {
        checkout = value.checkout;
        checkoutSelect.replaceChildren(...value.checkouts.map(c => {
          const option = html('option', `${c.label} · ${c.commit.slice(0, 8)}`); option.value = c.id; return option;
        })); checkoutSelect.value = checkout;
        host.querySelector('#code-source-path').textContent = `${value.source_label} · ${value.source_root}`;
      }
      const messages = [];
      if (value.status === 'analyzing') messages.push('正在分析程式關係，你可以先查看工作樹。');
      else if (value.status === 'failed') messages.push('這次分析未完成，請保留目前資料並檢查原因：' + value.error);
      else if (!value.graph) messages.push(value.status === 'not_analyzed'
        ? '這個專案尚未建立程式圖譜。按「分析目前版本」開始。' : value.message);
      if (value.graph) {
        messages.push(value.stale ? '有較新的程式變動；目前顯示先前分析，請勿當成最新結果。' : '顯示這個專案已保存版本的程式關係。');
        if (value.working_changes) messages.push('尚未保存的修改未納入這張圖。');
        for (const warning of value.graph.coverage_warnings || []) messages.push(warning.code === 'SQL_PARSER_UNAVAILABLE'
          ? `${warning.files} 個 SQL 檔案尚未涵蓋：目前分析器缺少 SQL 解析功能。`
          : `${warning.files} 個檔案沒有產生可查詢的節點。`);
      }
      status(messages.join(' ')); notice.dataset.state = value.stale || value.status === 'failed' || value.graph?.coverage_warnings?.length ? 'warning' : value.status;
      refreshButton.disabled = value.status === 'analyzing' || value.status === 'unavailable';
      refreshButton.textContent = value.status === 'analyzing' ? '分析中…' : value.graph ? '重新分析目前版本' : '分析目前版本';
      if (value.graph) {
        const graph = value.graph;
        host.querySelector('#code-summary').textContent = `Graphify · ${graph.nodes.length} 個節點 · ${graph.edges.length} 條關係 · 版本 ${graph.commit.slice(0, 8)} · ${new Date(graph.generated_at).toLocaleString('zh-TW')}`;
        if (changed || !index) {
          index = codeIndex(graph); root = null; placements.clear(); expanded.clear();
          renderSearch(); const first = searchCode(index, search.value)[0]; if (first) chooseRoot(first.id);
        }
      }
      clearTimeout(timer);
      if (value.status === 'analyzing') timer = setTimeout(() => void load({force:true}), 2500);
    } catch (error) {
      if (request !== requestId || requestedProject !== projectId) return;
      loadedProject = null; status('無法讀取程式圖譜：' + error.message); refreshButton.disabled = false;
    }
  }
  function renderSearch() {
    const matches = index ? searchCode(index, search.value) : [];
    results.replaceChildren(...matches.map(node => {
      const button = html('button', null, 'code-search-result'); button.type = 'button';
      button.append(html('strong', node.label), html('small', `${node.file} ${node.location}`));
      button.title = `${node.label}\n${node.file} ${node.location}`;
      button.addEventListener('click', () => chooseRoot(node.id)); return button;
    }));
    host.querySelector('#code-search-count').textContent = matches.length
      ? `顯示 ${matches.length} 筆，點選一項開始追查` : index ? '沒有符合的程式節點' : '分析完成後可搜尋';
  }
  function chooseRoot(id) {
    root = id; selected = id; expanded = new Set([id]); placements.clear(); zoom = 1;
    draw(); showDetail(id);
    host.querySelector('.code-scroll').scrollTo(0, 0);
  }
  function showDetail(id) {
    const node = index?.nodes.get(id); if (!node) return;
    selected = id;
    const incoming = index.incoming.get(id), outgoing = index.outgoing.get(id);
    detail.replaceChildren(html('h3', node.label), html('p', `${node.file} ${node.location}`, 'code-location'),
      html('p', `${incoming.length} 條傳入關係 · ${outgoing.length} 條傳出關係`),
      html('p', '實線：分析找到的關係；虛線：推測或待確認。圖譜不代表相關功能已通過測試。', 'code-help'));
    const heading = html('h4', '相連的程式與關係'); detail.append(heading);
    const rows = [...incoming, ...outgoing];
    for (const edge of rows.slice(0, 40)) {
      const other = index.nodes.get(edge.source === id ? edge.target : edge.source);
      const button = html('button', null, 'code-relation-row'); button.type = 'button';
      button.append(html('span', `${edge.source === id ? '→' : '←'} ${relationLabels[edge.relation] || edge.relation} · ${other.label}`),
        html('small', `${confidence[edge.confidence]} · ${edge.file} ${edge.location}`));
      button.addEventListener('click', () => chooseRoot(other.id)); detail.append(button);
    }
    if (rows.length > 40) detail.append(html('p', `還有 ${rows.length - 40} 條關係，可用方向與關係篩選縮小範圍。`));
  }
  function draw() {
    if (!index || !root) return;
    const scroller = host.querySelector('.code-scroll');
    const anchor = canvas.querySelector(`[data-code-node="${CSS.escape(root)}"]`);
    const anchorX = anchor?.getBoundingClientRect().left;
    const branch = codeBranches(index, root, expanded, direction, relation);
    // Keep existing cards anchored while one branch opens or closes.
    for (const node of branch.nodes) if (!placements.has(node.id)) {
      const depth = branch.depths.get(node.id), column = direction === 'incoming' ? -depth : depth;
      const row = [...placements.values()].filter(box => box.column === column).length;
      placements.set(node.id, { column, row });
    }
    const boxes = branch.nodes.map(node => placements.get(node.id));
    const minimum = Math.min(0, ...[...placements.values()].map(p => p.column));
    const maximum = Math.max(0, ...[...placements.values()].map(p => p.column));
    const width = Math.max(880, (maximum - minimum + 1) * 285 + 45);
    const height = Math.max(440, (Math.max(0, ...boxes.map(p => p.row)) + 1) * 104 + 70);
    canvas.setAttribute('viewBox', `0 0 ${width} ${height}`);
    canvas.style.width = `${width * zoom}px`; canvas.style.height = `${height * zoom}px`;
    const at = id => { const p = placements.get(id); return { x:30 + (p.column - minimum) * 285, y:40 + p.row * 104 }; };
    const defs = svg('defs'), marker = svg('marker', { id:'code-arrow', viewBox:'0 0 10 10', refX:9, refY:5,
      markerWidth:6, markerHeight:6, orient:'auto' });
    marker.append(svg('path', {d:'M 0 0 L 10 5 L 0 10 z', fill:'#52d8e8'})); defs.append(marker); canvas.replaceChildren(defs);
    for (const edge of branch.edges) {
      const start = at(edge.source), end = at(edge.target);
      const forward = start.x < end.x, x1 = start.x + (forward ? 235 : 0), x2 = end.x + (forward ? 0 : 235);
      const y1 = start.y + 35, y2 = end.y + 35;
      const same = start.x === end.x;
      const d = same ? `M ${start.x + 235} ${y1} C ${start.x + 270} ${y1}, ${start.x + 270} ${y2}, ${end.x + 235} ${y2}`
        : `M ${x1} ${y1} C ${(x1+x2)/2} ${y1}, ${(x1+x2)/2} ${y2}, ${x2} ${y2}`;
      const line = svg('path', {d, class:'code-edge', 'marker-end':'url(#code-arrow)',
        'data-confidence':edge.confidence, 'data-source':edge.source, 'data-target':edge.target});
      line.append(svg('title')); line.firstChild.textContent = `${relationLabels[edge.relation] || edge.relation} · ${confidence[edge.confidence]}`; canvas.append(line);
    }
    for (const node of branch.nodes) {
      const p = at(node.id), neighbors = codeNeighbors(index, node.id, direction, relation);
      const foreign = svg('foreignObject', {x:p.x, y:p.y, width:235, height:76});
      const button = html('button', null, `code-node${node.id === root ? ' code-root' : ''}`); button.type = 'button';
      button.dataset.codeNode = node.id; button.title = `${node.label}\n${node.file} ${node.location}`;
      button.setAttribute('aria-label', `${expanded.has(node.id) ? '收合' : '展開'}程式節點 ${node.label}`);
      if (neighbors.length) button.setAttribute('aria-expanded', String(expanded.has(node.id)));
      button.append(html('strong', node.label), html('small', node.file.split('/').at(-1)),
        html('small', neighbors.length ? `${expanded.has(node.id) ? '▾ 收合' : '▸ 展開'} ${neighbors.length} 條關係` : '沒有此方向的關係'));
      button.addEventListener('click', () => {
        if (expanded.has(node.id)) expanded.delete(node.id); else expanded.add(node.id);
        draw(); showDetail(node.id);
        canvas.querySelector(`[data-code-node="${CSS.escape(node.id)}"]`)?.focus({preventScroll:true});
      }); foreign.append(button); canvas.append(foreign);
    }
    host.querySelector('#code-canvas-note').textContent = `箭頭由使用者指向被使用的程式。顯示 ${branch.nodes.length} 個節點、${branch.edges.length} 條關係。`
      + (branch.truncated ? ' 此分支超過 80 個節點，請搜尋更具體的程式或縮小關係範圍。' : ' 點節點展開，再點一次收合。');
    if (anchorX != null) {
      const nextAnchor = canvas.querySelector(`[data-code-node="${CSS.escape(root)}"]`);
      if (nextAnchor) scroller.scrollLeft += nextAnchor.getBoundingClientRect().left - anchorX;
    }
  }
  projectSelect.addEventListener('change', () => onProjectChange(projectSelect.value));
  checkoutSelect.addEventListener('change', () => {
    checkout = checkoutSelect.value; loadedProject = null; requestId++; clearTimeout(timer); resetGraph(); void load();
  });
  search.addEventListener('input', renderSearch);
  search.addEventListener('keydown', event => { if (event.key === 'Enter' && index) {
    const first = searchCode(index, search.value)[0]; if (first) chooseRoot(first.id);
  }});
  host.querySelector('#code-direction').addEventListener('change', event => {
    direction = event.target.value; if (root) chooseRoot(root);
  });
  host.querySelector('#code-relation').addEventListener('change', event => {
    relation = event.target.value; if (root) chooseRoot(root);
  });
  refreshButton.addEventListener('click', async () => {
    if (!projectId) return;
    const requestedProject = projectId;
    refreshButton.disabled = true; status('正在啟動分析…');
    try { await api('/api/code-graph/analyze', {method:'POST', body:JSON.stringify({projectId,checkout})});
      if (requestedProject === projectId) await load({force:true});
    } catch (error) { if (requestedProject === projectId) { status('分析未啟動：' + error.message); refreshButton.disabled = false; } }
  });
  host.querySelector('#code-reload').addEventListener('click', () => void load({force:true}));
  host.querySelector('#code-collapse').addEventListener('click', () => { expanded.clear(); draw(); });
  host.querySelector('#code-expand').addEventListener('click', () => { if (!index || !root) return;
    for (const node of codeBranches(index, root, expanded, direction, relation).nodes) expanded.add(node.id); draw(); });
  for (const [id, factor] of [['code-zoom-in', 1.2],['code-zoom-out', 1/1.2]])
    host.querySelector('#'+id).addEventListener('click', () => { zoom = Math.max(.5, Math.min(1.8, zoom * factor)); draw(); });
  for (const button of host.querySelectorAll('[data-code-nav]')) button.addEventListener('click', () => onNavigate(button.dataset.codeNav));
  return {
    setProjects(projects, selectedId) {
      const key = JSON.stringify(projects.map(p => [p.id,p.name]));
      if (projectSelect.dataset.catalog !== key) {
        projectSelect.dataset.catalog = key;
        projectSelect.replaceChildren(...projects.map(p => { const option = html('option',p.name); option.value=p.id; return option; }));
      }
      if (projectId !== selectedId) {
        projectId = selectedId; checkout = null; loadedProject = null; requestId++; clearTimeout(timer); resetGraph(); search.value = '';
        status(projectId ? '正在讀取這個專案的程式圖譜…' : '請先選擇專案。');
        if (!host.hidden) void load();
      }
      projectSelect.value = projectId || '';
    },
    show() { host.hidden = false; void load({force:loadedProject === projectId && Boolean(packet)}); },
    hide() { host.hidden = true; },
  };
}
