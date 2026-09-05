// Both views consume the same read-only snapshot. Example data never enters the store.
const palette = {
  complete: { color: '#7ed46b', label: '已完成' },
  active: { color: '#39d5e7', label: '進行中' },
  pending: { color: '#aebdca', label: '待開始' },
  approval: { color: '#ffc43d', label: '等你決定' },
  blocked: { color: '#ffc43d', label: '遇到阻礙' },
  failed: { color: '#ff8391', label: '執行失敗' },
  review: { color: '#aebdca', label: '等待驗收' },
  archived: { color: '#aebdca', label: '已封存' },
  unknown: { color: '#aebdca', label: '狀態待確認' },
};

export function workAppearance(work) {
  if (work.status === 'failed') return palette.failed;
  if (work.status === 'waiting_approval') return palette.approval;
  if (work.blocker?.status === 'blocked') return palette.blocked;
  if (work.status === 'verified') return palette.complete;
  if (['running', 'starting'].includes(work.status)) return palette.active;
  if (work.status === 'draft') return palette.pending;
  if (work.status === 'codex_done') return palette.review;
  if (work.status === 'archived') return palette.archived;
  return palette.unknown;
}

// Layer real dependencies, rather than recycling six fixed positions as work grows.
export function layoutGraph(nodes) {
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const ranks = new Map(), visiting = new Set();
  function rank(id) {
    if (ranks.has(id)) return ranks.get(id);
    if (visiting.has(id)) throw new Error('工作依賴形成循環，請確認工作關係。');
    visiting.add(id);
    const dependencies = (byId.get(id).depends_on || []).filter((value) => byId.has(value));
    const result = dependencies.length ? Math.max(...dependencies.map(rank)) + 1 : 0;
    visiting.delete(id); ranks.set(id, result); return result;
  }
  nodes.forEach((node) => rank(node.id));
  const connected = nodes.some((node) => (node.depends_on || []).some((id) => byId.has(id)));
  const columns = new Map();
  nodes.forEach((node, index) => {
    const column = connected ? ranks.get(node.id) : index % 3;
    if (!columns.has(column)) columns.set(column, []);
    columns.get(column).push(node);
  });
  const rows = Math.max(1, ...[...columns.values()].map((column) => column.length));
  const positions = new Map();
  for (const [column, items] of columns) {
    items.forEach((node, row) => positions.set(node.id, {
      x: 28 + column * 254, y: 26 + row * 116 + (rows - items.length) * 58,
      width: 204, height: 78,
    }));
  }
  return { positions, width: Math.max(760, columns.size * 254 + 24), height: Math.max(540, rows * 116 + 28) };
}

// The first two levels spread horizontally. Deeper work forms connected branches.
export function layoutTree(tree, collapsed = new Set()) {
  const byId = new Map(tree.nodes.map((node) => [node.id, node]));
  const placed = new Set(), positions = new Map(), edges = [];
  const roots = tree.roots.filter((id) => byId.has(id));
  const singleRoot = roots.length === 1 ? byId.get(roots[0]) : null;
  const rootId = singleRoot?.id ?? '__project__';
  const firstLevel = singleRoot ? (singleRoot.children || []) : roots;
  const columns = collapsed.has(rootId) ? [] : firstLevel.filter((id) => byId.has(id));
  const branchWidth = (id, ancestors = new Set(), depth = 1) => {
    if (ancestors.has(id)) throw new Error('工作階層形成循環，請確認工作關係。');
    const children = collapsed.has(id) ? [] : (byId.get(id)?.children || []).filter((child) => byId.has(child));
    return Math.max(depth === 1 ? 204 : 178, ...children.map((child) => 52 + branchWidth(child, new Set([...ancestors, id]), depth + 1)));
  };
  const laneCount = Math.min(3, columns.length);
  const widths = Array.from({ length: laneCount }, (_, lane) => Math.max(244,
    ...columns.filter((_, index) => index % laneCount === lane).map((id) => branchWidth(id) + 20)));
  const totalWidth = widths.reduce((sum, value) => sum + value, 0);
  const width = Math.max(760, totalWidth + 20);
  positions.set(rootId, { x: (width - 340) / 2, y: 12, width: 340, height: 86, kind: 'root' });
  placed.add(rootId);
  let maxY = 350, maxX = width;
  const placeBranch = (id, x, y, depth, ancestors) => {
    if (ancestors.has(id)) throw new Error('工作階層形成循環，請確認工作關係。');
    const work = byId.get(id);
    if (!work || placed.has(id)) return y;
    placed.add(id);
    const box = { x, y, width: depth === 1 ? 204 : 178, height: 70, kind: depth === 1 ? 'category' : 'work' };
    positions.set(id, box); maxX = Math.max(maxX, x + box.width + 20);
    let cursor = y + 90;
    if (!collapsed.has(id)) {
      for (const childId of work.children || []) {
        if (!byId.has(childId)) continue;
        const before = positions.has(childId);
        cursor = placeBranch(childId, x + 52, cursor, depth + 1, new Set([...ancestors, id]));
        if (!before && positions.has(childId)) edges.push({ from: id, to: childId, kind: 'branch' });
      }
    }
    maxY = Math.max(maxY, cursor); return cursor;
  };
  const cursors = widths.map(() => 174);
  columns.forEach((id, index) => {
    const lane = index % laneCount;
    const x = (width - totalWidth) / 2 + 8 + widths.slice(0, lane).reduce((sum, value) => sum + value, 0);
    cursors[lane] = placeBranch(id, x, cursors[lane], 1, new Set([rootId])) + 20;
    edges.push({ from: rootId, to: id, kind: columns.length > 3 ? 'root-branch' : 'trunk' });
  });
  return { positions, edges, rootId, width: maxX, height: Math.max(540, maxY) };
}

const exampleItems = [
  ['goal', '陪玩網站上線', 'running', null],
  ['website', '網站功能', 'running', 'goal'],
  ['payments', '收款準備', 'waiting_approval', 'goal'],
  ['delivery', '交付驗收', 'draft', 'goal'],
  ['requirements', '需求確認', 'verified', null],
  ['login', '會員登入', 'verified', 'website'],
  ['list', '陪玩列表', 'running', 'website'],
  ['booking', '預約功能', 'draft', 'website'],
  ['payment', '付款設定', 'waiting_approval', 'payments'],
  ['acceptance', '整體驗收', 'draft', 'delivery'],
  ['launch', '正式上線', 'draft', 'delivery'],
];
const exampleDependencies = {
  login: ['requirements'], list: ['requirements'], payment: ['requirements'],
  booking: ['login', 'list'], acceptance: ['booking', 'payment'], launch: ['acceptance'],
};
export const exampleSnapshot = (() => {
  const nodes = exampleItems.map(([id, title, status, parent_id]) => ({
    id, title, status, parent_id, children: exampleItems.filter((item) => item[3] === id).map((item) => item[0]),
    depends_on: exampleDependencies[id] || [],
    reverse_dependents: Object.keys(exampleDependencies).filter((key) => exampleDependencies[key].includes(id)),
    blocker: { status: 'clear', reasons: [] },
  }));
  return {
    graph: { nodes: nodes.filter((node) => !['goal', 'website', 'payments', 'delivery'].includes(node.id)) },
    tree: { nodes: nodes.filter((node) => node.id !== 'requirements'), roots: ['goal'] },
  };
})();
const examplePositions = new Map([
  ['requirements', { x: 32, y: 0, width: 182, height: 68 }],
  ['login', { x: 345, y: 12, width: 184, height: 68 }],
  ['list', { x: 245, y: 125, width: 184, height: 68 }],
  ['payment', { x: 184, y: 278, width: 184, height: 70 }],
  ['booking', { x: 532, y: 132, width: 194, height: 68 }],
  ['acceptance', { x: 532, y: 277, width: 194, height: 70 }],
  ['launch', { x: 532, y: 422, width: 194, height: 70 }],
]);

const svgNS = 'http://www.w3.org/2000/svg';
function svg(tag, attributes = {}) {
  const node = document.createElementNS(svgNS, tag);
  for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
  return node;
}
function html(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
const icons = {
  goal: '<circle cx="24" cy="24" r="19"/><circle cx="24" cy="24" r="11"/><path d="m24 24 20-20m-10 1h9v9"/>',
  screen: '<rect x="5" y="7" width="38" height="28" rx="2"/><path d="M24 35v7M13 43h22"/>',
  wallet: '<rect x="5" y="8" width="36" height="32" rx="4"/><path d="M6 17h32m-2 7h8v11H31V24z"/>',
  shield: '<path d="m24 4 17 7v13c0 10-17 20-17 20S7 34 7 24V11z"/><path d="m15 23 6 6 12-14"/>',
  bulb: '<path d="M16 33c0-6-7-8-7-16a15 15 0 0 1 30 0c0 8-7 10-7 16zM17 39h14m-12 5h10"/>',
  alert: '<circle cx="24" cy="24" r="20"/><path d="M24 12v15m0 7v1"/>',
};
function icon(name) {
  const node = svg('svg', { viewBox: '0 0 48 48', fill: 'none', stroke: 'currentColor', 'stroke-width': 3, 'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'aria-hidden': 'true' });
  node.innerHTML = icons[name] || icons.screen; // Only our static icon paths, never work data.
  return node;
}

export function createWorkView({ onSelect, onOpen, onNavigate }) {
  const host = document.querySelector('#work-views');
  const graphLayer = document.querySelector('#graph-edge-layer');
  const graphList = document.querySelector('#graph-list');
  const treeList = document.querySelector('#tree-list');
  const sourceButton = document.querySelector('#work-source');
  const graphNote = document.querySelector('#graph-note');
  const treeNote = document.querySelector('#tree-current');
  let snapshot = null, projectName = '', message = '正在讀取工作…';
  let example = new URLSearchParams(location.search).get('workExample') === '1';
  let selected = null, collapsed = new Set(), lastKey = null;
  const actualWork = (id) => snapshot?.graph.nodes.find((work) => work.id === id);

  function nodeButton(work, box, { isExample, structural = false, iconName, onActivate } = {}) {
    const foreign = svg('foreignObject', { x: box.x, y: box.y, width: box.width, height: box.height });
    const appearance = workAppearance(work);
    const button = html('button', `work-chip${structural ? ' structural' : ''}${box.kind === 'root' ? ' root-chip' : ''}`);
    button.type = 'button'; button.dataset.nodeId = work.id;
    button.dataset.status = work.status; button.style.setProperty('--node-color', structural ? '#45d5e5' : appearance.color);
    button.setAttribute('aria-label', `${work.title}，${appearance.label}`);
    button.title = `${work.title} · ${appearance.label}`;
    button.classList.toggle('is-selected', selected === work.id);
    if (structural) button.append(icon(iconName || 'screen'));
    else button.append(html('span', 'work-dot'));
    const title = html('span', 'work-chip-title', work.title); button.append(title);
    button.addEventListener('click', () => {
      selected = work.id;
      for (const item of host.querySelectorAll('[data-node-id]')) item.classList.toggle('is-selected', item.dataset.nodeId === selected);
      if (onActivate) onActivate();
      else if (isExample) showExampleDetail(work);
      else { onSelect?.(actualWork(work.id) || work); onOpen?.(work.id); }
    });
    foreign.append(button); return foreign;
  }
  function showExampleDetail(work) {
    const dialog = document.querySelector('#work-example-detail');
    dialog.querySelector('h2').textContent = work.title;
    dialog.querySelector('p').textContent = `${workAppearance(work).label}。這是外觀示例，不是你的實際工作，也不會修改工作紀錄。`;
    dialog.showModal();
  }
  function drawGraph(graph, isExample) {
    const layout = isExample ? { positions: examplePositions, width: 760, height: 540 } : layoutGraph(graph.nodes);
    const { positions, width, height } = layout;
    for (const layer of [graphLayer, graphList]) layer.setAttribute('viewBox', `0 0 ${width} ${height}`);
    graphLayer.parentElement.style.setProperty('--diagram-ratio', `${width} / ${height}`);
    graphLayer.parentElement.style.setProperty('--diagram-width', `${Math.max(520, width * .72)}px`);
    const definitions = svg('defs'); graphLayer.replaceChildren(definitions); graphList.replaceChildren();
    for (const [index, work] of graph.nodes.entries()) {
      const end = positions.get(work.id);
      for (const [dependencyIndex, id] of (work.depends_on || []).entries()) {
        const start = positions.get(id); if (!start) continue;
        const dependency = graph.nodes.find((item) => item.id === id), color = workAppearance(dependency).color;
        const markerId = `work-arrow-${index}-${dependencyIndex}`;
        const marker = svg('marker', { id: markerId, viewBox: '0 0 10 10', refX: 9, refY: 5, markerWidth: 5, markerHeight: 5, orient: 'auto' });
        marker.append(svg('path', { d: 'M 0 0 L 10 5 L 0 10 z', fill: color })); definitions.append(marker);
        let path;
        if (Math.abs(start.x - end.x) < 10) {
          const x = start.x + start.width / 2;
          path = `M ${x} ${start.y + start.height} V ${end.y - 6}`;
        } else if (end.y > start.y + start.height && (dependency.reverse_dependents?.length > 1 || end.x < start.x + start.width)) {
          const x = start.x + start.width / 2, y = start.y + start.height, endY = end.y + end.height / 2;
          path = `M ${x} ${y} V ${endY - 58} Q ${x} ${endY} ${x + 64} ${endY} H ${end.x - 6}`;
        } else if (end.y > start.y + start.height && end.x - start.x - start.width < 70) {
          const x = start.x + start.width, y = start.y + start.height / 2, endX = end.x + end.width / 2;
          path = `M ${x} ${y} H ${endX - 54} Q ${endX} ${y} ${endX} ${y + 54} V ${end.y - 6}`;
        } else {
          const x = start.x + start.width, y = start.y + start.height / 2, endX = end.x - 6, endY = end.y + end.height / 2;
          const bend = Math.max(42, (endX - x) * .55);
          path = `M ${x} ${y} C ${x + bend} ${y} ${endX} ${y} ${endX} ${endY}`;
        }
        graphLayer.append(svg('path', { class: 'work-dependency', d: path, stroke: color, 'marker-end': `url(#${markerId})`, 'data-from': id, 'data-to': work.id }));
      }
      graphList.append(nodeButton(work, end, { isExample }));
    }
    graphNote.replaceChildren();
    const waiting = graph.nodes.find((work) => work.status === 'waiting_approval');
    const blocker = graph.nodes.find((work) => work.blocker?.status === 'blocked' || work.status === 'failed');
    const focus = waiting || blocker;
    if (focus) {
      const affected = graph.nodes.filter((work) => (work.depends_on || []).includes(focus.id));
      graphNote.classList.add('attention'); graphNote.append(icon('alert'));
      graphNote.append(html('span', '', `${focus.title}${waiting ? '等你決定' : '遇到阻礙'}${affected.length ? `，會影響${affected.map((work) => work.title).join('、')}` : ''}`));
    } else {
      graphNote.classList.remove('attention'); graphNote.append(icon('bulb'));
      graphNote.append(html('span', '', graphLayer.querySelector('.work-dependency') ? '沿著箭頭，就能看懂工作先後與影響' : '尚未記錄依賴關係；點選工作可查看詳情'));
    }
  }
  function drawTree(tree, isExample) {
    const layout = layoutTree(tree, collapsed);
    const { positions, edges, width, height, rootId } = layout;
    const canvas = svg('svg', { viewBox: `0 0 ${width} ${height}`, class: 'work-tree-svg', 'aria-label': '目標與子工作階層' });
    treeList.style.setProperty('--diagram-width', `${Math.max(520, width * .72)}px`);
    const byId = new Map(tree.nodes.map((work) => [work.id, work]));
    for (const edge of edges) {
      const start = positions.get(edge.from), end = positions.get(edge.to);
      let path;
      if (edge.kind === 'trunk') {
        const x = start.x + start.width / 2, y = start.y + start.height, endX = end.x + end.width / 2, middle = y + 38;
        path = `M ${x} ${y} V ${middle} H ${endX} V ${end.y}`;
      } else if (edge.kind === 'root-branch') {
        const x = start.x + start.width / 2, y = start.y + start.height, trunk = end.x - 12, endY = end.y + end.height / 2;
        path = `M ${x} ${y} V ${y + 38} H ${trunk} V ${endY} H ${end.x}`;
      } else {
        const x = start.x + 22, y = start.y + start.height, endY = end.y + end.height / 2;
        path = `M ${x} ${y} V ${endY - 14} Q ${x} ${endY} ${x + 14} ${endY} H ${end.x}`;
      }
      canvas.append(svg('path', { d: path, class: 'work-tree-branch', 'data-from': edge.from, 'data-to': edge.to }));
    }
    for (const [id, box] of positions) {
      const work = byId.get(id) || { id, title: projectName || '目前專案', status: 'unknown', children: tree.roots };
      const children = work.children || [], hasChildren = children.length > 0;
      const structural = hasChildren || box.kind === 'root';
      const iconName = box.kind === 'root' ? 'goal' : /付款|收款|金流/.test(work.title) ? 'wallet' : /驗收|交付/.test(work.title) ? 'shield' : 'screen';
      const group = nodeButton(work, box, { isExample, structural, iconName,
        onActivate: id === '__project__' ? () => { collapsed.has(id) ? collapsed.delete(id) : collapsed.add(id); drawTree(tree, isExample); } : undefined,
      });
      canvas.append(group);
      if (hasChildren) {
        const control = svg('foreignObject', { x: box.x + box.width - 25, y: box.y + box.height - 24, width: 24, height: 24 });
        const button = html('button', 'work-collapse', collapsed.has(id) ? '+' : '−');
        button.type = 'button'; button.title = `${collapsed.has(id) ? '展開' : '收合'}${work.title}`;
        button.setAttribute('aria-label', button.title); button.setAttribute('aria-expanded', String(!collapsed.has(id)));
        button.dataset.branchId = id;
        button.addEventListener('click', () => {
          collapsed.has(id) ? collapsed.delete(id) : collapsed.add(id); drawTree(tree, isExample);
          treeList.querySelector(`[data-branch-id="${CSS.escape(id)}"]`)?.focus({ preventScroll: true });
        });
        control.append(button); canvas.append(control);
      }
    }
    treeList.replaceChildren(canvas);
    treeNote.replaceChildren(icon('bulb'), html('span', '', '展開大目標，就能看到每一層工作'));
  }
  function render() {
    host.dataset.source = example ? 'example' : 'live';
    sourceButton.textContent = example ? '示意資料 · 返回我的工作' : '我的工作 · 查看設計示例';
    sourceButton.setAttribute('aria-pressed', String(example));
    document.querySelector('#work-source-label').textContent = example ? '概念預覽・示意資料' : projectName || '我的工作';
    const data = example ? exampleSnapshot : snapshot;
    const empty = !data?.graph.nodes.length;
    document.querySelector('#work-empty').hidden = !empty;
    document.querySelector('#work-empty').textContent = message;
    document.querySelector('#work-diagrams').hidden = empty;
    if (!empty) { drawGraph(data.graph, example); drawTree(data.tree, example); }
  }
  sourceButton.addEventListener('click', () => {
    example = !example; selected = null; collapsed.clear();
    const url = new URL(location.href);
    if (example) url.searchParams.set('workExample', '1'); else url.searchParams.delete('workExample');
    history.replaceState(null, '', url); render();
  });
  document.querySelector('#work-back').addEventListener('click', () => onNavigate('conversation'));
  document.querySelector('#work-decisions').addEventListener('click', () => onNavigate('decisions'));
  return {
    update(data, context = {}) {
      snapshot = data; projectName = context.project_name || ''; message = context.status_text || '這個專案還沒有工作。';
      const key = `${context.project_id}:${data?.revision}:${data?.digest}:${message}`;
      if (key !== lastKey) { lastKey = key; render(); }
    },
    show() { host.hidden = false; render(); },
    hide() { host.hidden = true; },
  };
}
