// Work hierarchy and progress only. Code relationships use a separate view and data source.
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
  if (work.completion_verified === true) return palette.complete;
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

export const workFilters = [
  ['all', '全部'], ['complete', '已完成'], ['active', '進行中'], ['pending', '待開始'],
  ['review', '等待驗收'], ['approval', '等你決定'], ['blocked', '遇到阻礙'],
  ['failed', '執行失敗'], ['archived', '已封存'], ['unknown', '狀態待確認'],
];

export function workCategory(work) {
  // Completion remains discoverable after archival; archival alone is not completion.
  if (work.completion_verified === true) return 'complete';
  return Object.keys(palette).find((key) => palette[key] === workAppearance(work)) || 'unknown';
}

export function workScope(data, focusId = null, filter = 'all') {
  const all = new Map(data.tree.nodes.map((work) => [work.id, work]));
  const treeById = new Map(data.tree.nodes.map((work) => [work.id, work]));
  const lineage = [], visited = new Set();
  let cursor = focusId && treeById.get(focusId);
  while (cursor) {
    if (visited.has(cursor.id)) throw new Error('工作階層形成循環，請確認工作關係。');
    visited.add(cursor.id); lineage.unshift(cursor); cursor = treeById.get(cursor.parent_id);
  }
  const scope = new Set();
  function descend(id) {
    if (scope.has(id) || !all.has(id)) return;
    scope.add(id); (treeById.get(id)?.children || []).forEach(descend);
  }
  if (lineage.length) descend(focusId); else all.forEach((_, id) => scope.add(id));
  const counts = Object.fromEntries(workFilters.map(([key]) => [key, 0]));
  const matches = new Set();
  for (const id of scope) {
    const category = workCategory(all.get(id)); counts.all++; counts[category]++;
    if (filter === 'all' || category === filter) matches.add(id);
  }
  // Keep the paths to matching work, so a filter cannot invent a new hierarchy.
  const retained = new Set(matches);
  for (const id of matches) {
    let parent = treeById.get(id)?.parent_id;
    const seen = new Set();
    while (parent && scope.has(parent) && !seen.has(parent)) {
      seen.add(parent); retained.add(parent); parent = treeById.get(parent)?.parent_id;
    }
  }
  const treeNodes = data.tree.nodes.filter((work) => retained.has(work.id)).map((work) => ({ ...work,
    children: (work.children || []).filter((id) => retained.has(id)),
  }));
  const graphNodes = data.graph.nodes.filter((work) => retained.has(work.id));
  return { counts, matches, lineage,
    children: (lineage.length ? treeById.get(focusId).children || [] : data.tree.roots).map((id) => treeById.get(id)).filter(Boolean),
    graph: { ...data.graph, nodes: graphNodes },
    tree: { ...data.tree, nodes: treeNodes, roots: treeNodes.filter((work) => !retained.has(work.parent_id)).map((work) => work.id) },
  };
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

// Each node owns its open/closed state. Shared downstream work remains visible
// while another open branch still reaches it.
export function graphBranches(graph, choices = new Map(), revealAll = false) {
  const children = new Map(graph.nodes.map((work) => [work.id, []]));
  const roots = graph.nodes.filter((work) => !(work.depends_on || []).some((id) => children.has(id)));
  const rootIds = new Set(roots.map((work) => work.id));
  for (const work of graph.nodes) for (const id of work.depends_on || []) children.get(id)?.push(work.id);
  const expanded = new Map(graph.nodes.map((work) => [work.id, choices.get(work.id) ?? (revealAll || rootIds.has(work.id))]));
  const visible = new Set();
  function visit(id) {
    if (visible.has(id)) return;
    visible.add(id); if (expanded.get(id)) children.get(id).forEach(visit);
  }
  roots.forEach((work) => visit(work.id));
  return { children, expanded, visible };
}

export function treeBranches(tree, choices = new Map(), revealAll = false) {
  const rootId = tree.roots.length === 1 ? tree.roots[0] : '__project__';
  return new Set([...tree.nodes.map((work) => work.id), '__project__']
    .filter((id) => !(choices.get(id) ?? (revealAll || id === rootId))));
}

// The first two levels spread horizontally. Deeper work forms connected branches.
export function layoutTree(tree, collapsed = new Set()) {
  const byId = new Map(tree.nodes.map((node) => [node.id, node]));
  const placed = new Set(), positions = new Map(), edges = [];
  const roots = tree.roots.filter((id) => byId.has(id));
  const singleRoot = roots.length === 1 ? byId.get(roots[0]) : null;
  const rootId = singleRoot?.id ?? '__project__';
  const firstLevel = singleRoot ? (singleRoot.children || []) : roots;
  const columns = firstLevel.filter((id) => byId.has(id));
  const branchWidth = (id, ancestors = new Set(), depth = 1) => {
    if (ancestors.has(id)) throw new Error('工作階層形成循環，請確認工作關係。');
    // Reserve the branch width while folded, keeping parent cards in place.
    const children = (byId.get(id)?.children || []).filter((child) => byId.has(child));
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
  if (!collapsed.has(rootId)) columns.forEach((id, index) => {
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

export function createWorkView({ onSelect, onOpen, onNavigate, onProjectChange }) {
  const host = document.querySelector('#work-views');
  const treeList = document.querySelector('#tree-list');
  const sourceButton = document.querySelector('#work-source');
  const treeNote = document.querySelector('#tree-current');
  let snapshot = null, projectName = '', message = '正在讀取工作…';
  let example = new URLSearchParams(location.search).get('workExample') === '1';
  let selected = null, lastKey = null, projectId = null;
  let filter = 'all', currentScope = null;
  const treeChoices = new Map(), graphChoices = new Map();
  const projectSelect = document.querySelector('#work-project');
  const filters = document.querySelector('#work-filters');
  const detailButton = document.querySelector('#work-selected-detail');
  const selectionLabel = document.querySelector('#work-selection');
  const actualWork = (id) => snapshot?.graph.nodes.find((work) => work.id === id);

  function nodeButton(work, box, { isExample, structural = false, iconName, onActivate, branchCount = 0, expanded = false } = {}) {
    const foreign = svg('foreignObject', { x: box.x, y: box.y, width: box.width, height: box.height });
    const appearance = workAppearance(work);
    const button = html('button', `work-chip${structural ? ' structural' : ''}${box.kind === 'root' ? ' root-chip' : ''}`);
    button.type = 'button'; button.dataset.nodeId = work.id;
    button.dataset.status = work.status; button.style.setProperty('--node-color', structural ? '#45d5e5' : appearance.color);
    button.setAttribute('aria-label', `${work.title}，${appearance.label}`);
    button.title = `${work.title} · ${appearance.label}`;
    if (branchCount) {
      button.classList.add('has-branches'); button.setAttribute('aria-expanded', String(expanded));
      button.setAttribute('aria-label', `${expanded ? '收合' : '展開'}${work.title}的分支`);
      button.title += ` · 點一下${expanded ? '收合' : '展開'} ${branchCount} 個分支`;
    }
    button.classList.toggle('is-selected', selected === work.id);
    if (structural) button.append(icon(iconName || 'screen'));
    else button.append(html('span', 'work-dot'));
    const title = html('span', 'work-chip-title', work.title);
    const copy = html('span', 'work-chip-copy'); copy.append(title);
    if (branchCount) copy.append(html('small', 'work-branch-hint', `${expanded ? '▾ 收合' : '▸ 展開'} ${branchCount} 個分支`));
    else if (!isExample && work.id !== '__project__') copy.append(html('small', 'work-chip-status',
      work.completion_verified && work.status === 'archived' ? '已完成 · 已封存' : appearance.label));
    button.append(copy);
    button.classList.toggle('context-only', filter !== 'all' && !currentScope?.matches.has(work.id));
    button.addEventListener('click', () => {
      selected = work.id;
      for (const item of host.querySelectorAll('[data-node-id]')) item.classList.toggle('is-selected', item.dataset.nodeId === selected);
      if (!isExample && work.id !== '__project__') onSelect?.(actualWork(work.id) || work);
      updateSelection(work, branchCount);
      if (onActivate) onActivate();
    });
    foreign.append(button); return foreign;
  }
  function updateSelection(work, branchCount = 0) {
    detailButton.disabled = !work || work.id === '__project__';
    selectionLabel.textContent = work
      ? `已選：${work.title}${branchCount ? '' : filter === 'all' ? ' · 沒有下層分支' : ' · 沒有符合篩選的下層分支'}`
      : '直接點節點，展開或收合它下面的分支';
    selectionLabel.title = selectionLabel.textContent;
  }
  function showExampleDetail(work) {
    const dialog = document.querySelector('#work-example-detail');
    dialog.querySelector('h2').textContent = work.title;
    dialog.querySelector('p').textContent = `${workAppearance(work).label}。這是外觀示例，不是你的實際工作，也不會修改工作紀錄。`;
    dialog.showModal();
  }
  function drawTree(tree, isExample) {
    const collapsed = treeBranches(tree, treeChoices, filter !== 'all');
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
        branchCount: children.length, expanded: !collapsed.has(id),
        onActivate: hasChildren ? () => {
          treeChoices.set(id, collapsed.has(id)); drawTree(tree, isExample);
          treeList.querySelector(`[data-node-id="${CSS.escape(id)}"]`)?.focus({ preventScroll: true });
        } : undefined,
      });
      canvas.append(group);
    }
    treeList.replaceChildren(canvas);
    const hasHierarchy = tree.nodes.some((work) => work.parent_id);
    treeNote.replaceChildren(icon('bulb'), html('span', '', hasHierarchy
      ? '直接點節點展開分支，再點一次收合；父目標會留在原處'
      : '尚未記錄上下層關係；目前工作並列在專案下'));
  }
  function renderNavigation(data) {
    currentScope = data ? workScope(data, null, filter) : null;
    const counts = currentScope?.counts;
    for (const button of filters.querySelectorAll('button')) {
      const [key, label] = workFilters.find(([key]) => key === button.dataset.filter);
      button.textContent = `${label} ${counts ? counts[key] : '—'}`;
      button.setAttribute('aria-pressed', String(key === filter));
      button.hidden = !['all', 'complete', 'active', 'pending', 'review', 'approval'].includes(key) && !counts?.[key] && filter !== key;
      button.disabled = !data;
    }
    const selectedWork = [...(data?.tree.nodes || []), ...(data?.graph.nodes || [])].find((work) => work.id === selected);
    updateSelection(selectedWork, (selectedWork?.children?.length || selectedWork?.reverse_dependents?.length || 0));
    const notice = document.querySelector('#work-data-note');
    notice.textContent = example ? '示意資料：用來示範外觀與操作，不代表實際進度。'
      : '這裡顯示已登記的工作紀錄。尚未接上的 Codex 對話與驗收結果，不會自動變成圖中的進度。';
    if (data && data.graph.nodes.length && data.graph.nodes.every((work) => work.status === 'draft'))
      notice.textContent += ` 目前 ${data.graph.nodes.length} 項都記錄為待開始。`;
  }
  function render() {
    host.dataset.source = example ? 'example' : 'live';
    sourceButton.textContent = example ? '示意資料 · 返回我的工作' : '我的工作 · 查看設計示例';
    sourceButton.setAttribute('aria-pressed', String(example));
    document.querySelector('#work-source-label').textContent = example ? '概念預覽・示意資料' : projectName || '我的工作';
    const data = example ? exampleSnapshot : snapshot;
    renderNavigation(data);
    const empty = !currentScope?.matches.size;
    document.querySelector('#work-empty').hidden = !empty;
    document.querySelector('#work-empty').textContent = data
      ? !currentScope.counts.all ? '這個專案還沒有已登記的工作。'
        : `這個專案沒有「${workFilters.find(([key]) => key === filter)[1]}」的工作紀錄。可切回「全部」或查看其他專案。`
      : message;
    document.querySelector('#work-diagrams').hidden = empty;
    if (!empty) drawTree(currentScope.tree, example);
  }
  for (const [key] of workFilters) {
    const button = html('button', 'work-filter'); button.type = 'button'; button.dataset.filter = key;
    button.style.setProperty('--filter-color', palette[key]?.color || '#e3eaf2');
    button.addEventListener('click', () => { filter = key; treeChoices.clear(); graphChoices.clear(); render(); }); filters.append(button);
  }
  detailButton.addEventListener('click', () => {
    const data = example ? exampleSnapshot : snapshot;
    const work = [...(data?.tree.nodes || []), ...(data?.graph.nodes || [])].find((work) => work.id === selected);
    if (!work) return;
    if (example) showExampleDetail(work); else onOpen?.(work.id);
  });
  for (const [id, expand] of [['work-expand-all', true], ['work-collapse-all', false]]) {
    document.querySelector(`#${id}`).addEventListener('click', () => {
      const data = example ? exampleSnapshot : snapshot;
      treeChoices.clear(); graphChoices.clear();
      if (data) {
        const treeRoot = data.tree.roots.length === 1 ? data.tree.roots[0] : '__project__';
        data.tree.nodes.forEach((work) => treeChoices.set(work.id, expand || work.id === treeRoot));
        treeChoices.set('__project__', true);
        const roots = new Set(data.graph.nodes.filter((work) => !(work.depends_on || []).some((id) => data.graph.nodes.some((node) => node.id === id))).map((work) => work.id));
        data.graph.nodes.forEach((work) => graphChoices.set(work.id, expand || roots.has(work.id)));
      }
      render();
    });
  }
  projectSelect.addEventListener('change', () => {
    example = false; selected = null; filter = 'all'; treeChoices.clear(); graphChoices.clear();
    const url = new URL(location.href); url.searchParams.delete('workExample'); url.searchParams.set('project', projectSelect.value);
    history.replaceState(null, '', url); onProjectChange?.(projectSelect.value);
  });
  sourceButton.addEventListener('click', () => {
    example = !example; selected = null; filter = 'all'; treeChoices.clear(); graphChoices.clear();
    const url = new URL(location.href);
    if (example) url.searchParams.set('workExample', '1'); else url.searchParams.delete('workExample');
    history.replaceState(null, '', url); render();
  });
  document.querySelector('#work-back').addEventListener('click', () => onNavigate('conversation'));
  document.querySelector('#work-decisions').addEventListener('click', () => onNavigate('decisions'));
  return {
    selectLiveProject() { example = false; selected = null; filter = 'all'; treeChoices.clear(); graphChoices.clear(); },
    update(data, context = {}) {
      if (!example && projectId !== context.project_id) { selected = null; treeChoices.clear(); graphChoices.clear(); filter = 'all'; }
      projectId = context.project_id;
      snapshot = data; projectName = context.project_name || ''; message = context.status_text || '這個專案還沒有工作。';
      const key = `${context.project_id}:${data?.revision}:${data?.digest}:${message}`;
      if (key !== lastKey) { lastKey = key; render(); }
    },
    setProjects(projects, selectedId) {
      const value = JSON.stringify(projects.map(({ id, name }) => [id, name]));
      if (projectSelect.dataset.catalog !== value) {
        projectSelect.dataset.catalog = value;
        const prompt = html('option', '', '選擇專案'); prompt.value = ''; prompt.disabled = true;
        projectSelect.replaceChildren(prompt, ...projects.map((project) => {
          const option = html('option', '', project.name); option.value = project.id; return option;
        }));
      }
      projectSelect.value = selectedId || '';
    },
    show() { host.hidden = false; render(); },
    hide() { host.hidden = true; },
  };
}
