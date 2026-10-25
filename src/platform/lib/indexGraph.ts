import type { IndexGraph, IndexGraphEdge, IndexGraphNode } from "../../types";

export type GraphMode = "packages" | "files" | "symbols";

export function resizeGraphViewport(
  viewport: { x: number; y: number; zoom: number },
  previous: { width: number; height: number },
  next: { width: number; height: number },
) {
  return {
    ...viewport,
    x: viewport.x + (next.width - previous.width) / 2,
    y: viewport.y + (next.height - previous.height) / 2,
  };
}

const graphEdgeId = (edge: IndexGraphEdge) => JSON.stringify([edge.source, edge.target, edge.dev]);
const compareNodes = (a: IndexGraphNode, b: IndexGraphNode) => a.path.localeCompare(b.path) || a.id.localeCompare(b.id);

export function normalizeGraph(graph: IndexGraph): IndexGraph {
  const nodes = [...new Map(graph.nodes.map((node) => [node.id, node])).values()].sort(compareNodes);
  const ids = new Set(nodes.map((node) => node.id));
  const edges = [
    ...new Map(
      graph.edges
        .filter((edge) => ids.has(edge.source) && ids.has(edge.target))
        .map((edge) => [graphEdgeId(edge), edge]),
    ).values(),
  ].sort((a, b) => graphEdgeId(a).localeCompare(graphEdgeId(b)));
  return { nodes, edges };
}

export function searchNodes(nodes: IndexGraphNode[], query: string) {
  const text = query.trim().toLowerCase();
  return nodes.filter((node) => `${node.name} ${node.path}`.toLowerCase().includes(text));
}

export function graphSlice(
  graph: IndexGraph,
  {
    focusId,
    filePath,
    includeIsolated = false,
    nodeLimit = 150,
    edgeLimit = 300,
  }: {
    focusId?: string | null;
    filePath?: string | null;
    includeIsolated?: boolean;
    nodeLimit?: number;
    edgeLimit?: number;
  },
) {
  const focus = graph.nodes.some((node) => node.id === focusId) ? focusId : null;
  let nodes = graph.nodes;
  let edges = graph.edges;
  if (focus) {
    edges = edges.filter((edge) => edge.source === focus || edge.target === focus);
    const neighbors = new Set([focus, ...edges.flatMap((edge) => [edge.source, edge.target])]);
    nodes = nodes.filter((node) => neighbors.has(node.id));
  } else if (filePath) {
    nodes = nodes.filter((node) => node.path === filePath);
    const ids = new Set(nodes.map((node) => node.id));
    edges = edges.filter((edge) => ids.has(edge.source) && ids.has(edge.target));
  }
  const connected = new Set(edges.flatMap((edge) => [edge.source, edge.target]));
  const isolated = nodes.filter((node) => !connected.has(node.id));
  const hiddenIsolated = !focus && !includeIsolated && edges.length > 0 ? isolated.length : 0;
  if (hiddenIsolated) nodes = nodes.filter((node) => connected.has(node.id));
  const sorted = [...nodes].sort((a, b) => Number(b.id === focus) - Number(a.id === focus) || compareNodes(a, b));
  const visibleNodes = sorted.slice(0, Math.max(1, nodeLimit));
  const ids = new Set(visibleNodes.map((node) => node.id));
  const visibleEdges = edges.filter((edge) => ids.has(edge.source) && ids.has(edge.target)).slice(0, edgeLimit);
  return {
    nodes: visibleNodes,
    edges: visibleEdges,
    hiddenIsolated,
    isolatedCount: isolated.length,
    omittedNodes: nodes.length - visibleNodes.length,
    omittedEdges: edges.length - visibleEdges.length,
  };
}

export function layoutGraph(graph: IndexGraph) {
  const nodes = [...graph.nodes].sort(compareNodes);
  const adjacency = new Map(nodes.map((node) => [node.id, [] as string[]]));
  for (const edge of graph.edges) if (adjacency.has(edge.target)) adjacency.get(edge.source)?.push(edge.target);
  for (const neighbors of adjacency.values()) neighbors.sort();
  const indices = new Map<string, number>();
  const low = new Map<string, number>();
  const stack: string[] = [];
  const stacked = new Set<string>();
  const components: string[][] = [];
  const visit = (id: string) => {
    const index = indices.size;
    let lowest = index;
    indices.set(id, index);
    stack.push(id);
    stacked.add(id);
    for (const next of adjacency.get(id) ?? []) {
      if (!indices.has(next)) {
        visit(next);
        lowest = Math.min(lowest, low.get(next) ?? lowest);
      } else if (stacked.has(next)) lowest = Math.min(lowest, indices.get(next) ?? lowest);
    }
    low.set(id, lowest);
    if (lowest !== index) return;
    const component: string[] = [];
    let next: string | undefined;
    do {
      next = stack.pop();
      if (next === undefined) break;
      stacked.delete(next);
      component.push(next);
    } while (next !== id);
    components.push(component.sort());
  };
  for (const node of nodes) if (!indices.has(node.id)) visit(node.id);
  const componentOf = new Map(components.flatMap((component, index) => component.map((id) => [id, index] as const)));
  const outgoing = components.map(() => new Set<number>());
  const incoming = components.map(() => 0);
  for (const edge of graph.edges) {
    const from = componentOf.get(edge.source),
      to = componentOf.get(edge.target);
    if (from === undefined || to === undefined || from === to || outgoing[from].has(to)) continue;
    outgoing[from].add(to);
    incoming[to]++;
  }
  const queue = incoming.flatMap((count, index) => (count === 0 ? [index] : []));
  const levels = components.map(() => 0);
  for (let i = 0; i < queue.length; i++) {
    const from = queue[i];
    for (const to of outgoing[from]) {
      levels[to] = Math.max(levels[to], levels[from] + 1);
      if (--incoming[to] === 0) queue.push(to);
    }
  }
  const neighbors = new Map(nodes.map((node) => [node.id, new Set<string>()]));
  for (const edge of graph.edges) {
    neighbors.get(edge.source)?.add(edge.target);
    neighbors.get(edge.target)?.add(edge.source);
  }
  const positions = new Map<string, { x: number; y: number }>();
  let offsetX = 0,
    offsetY = 0,
    shelfHeight = 0;
  for (const node of nodes) {
    if (positions.has(node.id)) continue;
    const connected = new Set([node.id]);
    for (const id of connected) for (const next of neighbors.get(id) ?? []) connected.add(next);
    const rows = new Map<number, number>();
    const group = nodes
      .filter((item) => connected.has(item.id))
      .map((item) => {
        const level = levels[componentOf.get(item.id) ?? 0];
        const row = rows.get(level) ?? 0;
        rows.set(level, row + 1);
        return { id: item.id, x: level * 320, y: row * 120 };
      });
    const width = Math.max(...group.map((item) => item.x)) + 240;
    const height = Math.max(...group.map((item) => item.y)) + 84;
    if (offsetX > 0 && offsetX + width > 1200) {
      offsetX = 0;
      offsetY += shelfHeight + 80;
      shelfHeight = 0;
    }
    for (const item of group) positions.set(item.id, { x: offsetX + item.x, y: offsetY + item.y });
    offsetX += width + 80;
    shelfHeight = Math.max(shelfHeight, height);
  }
  return nodes.map((node) => ({ ...node, position: positions.get(node.id) ?? { x: 0, y: 0 } }));
}
