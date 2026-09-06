// Code relationships only. This model accepts no work-item status or hierarchy.
export const relationLabels = { calls:'呼叫', imports_from:'引用', contains:'包含', references:'參照',
  inherits:'繼承', implements:'實作', depends_on:'依賴', exports:'匯出', overrides:'覆寫',
  imports:'引用', indirect_call:'間接呼叫', method:'方法', extends:'擴充', re_exports:'再次匯出',
  uses:'使用', defines:'定義', includes:'引入', instantiates:'建立實例', binds_method:'綁定方法',
  bound_to:'綁定到', crate_depends_on:'套件依賴', listened_by:'被監聽', references_constant:'參照常數',
  uses_component:'使用元件', uses_static_prop:'使用靜態屬性', cites:'引用出處', rationale_for:'相關依據' };
export function codeIndex(graph) {
  const nodes = new Map(graph.nodes.map(node => [node.id, node]));
  const incoming = new Map(), outgoing = new Map();
  for (const id of nodes.keys()) { incoming.set(id, []); outgoing.set(id, []); }
  for (const edge of graph.edges) {
    if (!nodes.has(edge.source) || !nodes.has(edge.target)) continue;
    incoming.get(edge.target).push(edge); outgoing.get(edge.source).push(edge);
  }
  return { nodes, incoming, outgoing };
}
export function codeNeighbors(index, id, direction = 'incoming', relation = 'all') {
  const edges = direction === 'incoming' ? index.incoming.get(id) || []
    : direction === 'outgoing' ? index.outgoing.get(id) || []
      : [...index.incoming.get(id) || [], ...index.outgoing.get(id) || []];
  return edges.filter(edge => relation === 'all' || edge.relation === relation)
    .map(edge => ({ id: edge.source === id ? edge.target : edge.source, edge }));
}
export function codeBranches(index, root, expanded, direction = 'incoming', relation = 'all', limit = 80) {
  const visible = new Map(), edges = new Set(); let truncated = false;
  if (!index.nodes.has(root)) return { nodes: [], edges: [], truncated, depths: visible };
  const queue = [root]; visible.set(root, 0);
  for (let i = 0; i < queue.length; i++) {
    const id = queue[i]; if (!expanded.has(id)) continue;
    for (const neighbor of codeNeighbors(index, id, direction, relation)) {
      if (!visible.has(neighbor.id)) {
        if (visible.size >= limit) { truncated = true; continue; }
        visible.set(neighbor.id, visible.get(id) + 1); queue.push(neighbor.id);
      }
      edges.add(neighbor.edge);
    }
  }
  return { nodes: [...visible.keys()].map(id => index.nodes.get(id)), edges: [...edges], truncated, depths: visible };
}
export function searchCode(index, query = '', limit = 30) {
  const q = query.trim().toLocaleLowerCase();
  return [...index.nodes.values()].filter(n => !q || `${n.label} ${n.file}`.toLocaleLowerCase().includes(q))
    .sort((a, b) => {
      if (q) {
        const exact = Number(b.label.toLocaleLowerCase() === q) - Number(a.label.toLocaleLowerCase() === q);
        if (exact) return exact;
      }
      return (index.incoming.get(b.id)?.length || 0) - (index.incoming.get(a.id)?.length || 0)
        || a.file.localeCompare(b.file) || a.label.localeCompare(b.label);
    }).slice(0, limit);
}
