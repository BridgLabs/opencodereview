import { describe, expect, it } from "vitest";
import type { IndexGraph } from "../../types";
import { graphSlice, layoutGraph, normalizeGraph, resizeGraphViewport, searchNodes } from "./indexGraph";

const node = (id: string, path = id) => ({ id, name: id, path, kind: "lib" });
const edge = (source: string, target: string, dev = false) => ({ source, target, dev });
const graph: IndexGraph = {
  nodes: [node("a"), node("b"), node("c"), node("isolated")],
  edges: [edge("a", "b"), edge("b", "c"), edge("c", "a")],
};

describe("index graph", () => {
  it("preserves graph-space center and zoom when the canvas shrinks", () => {
    const previous = { width: 1600, height: 800 };
    const next = { width: 390, height: 384 };
    const viewport = { x: 550, y: 300, zoom: 0.75 };
    const resized = resizeGraphViewport(viewport, previous, next);
    expect(resized.zoom).toBe(viewport.zoom);
    expect((next.width / 2 - resized.x) / resized.zoom).toBe((previous.width / 2 - viewport.x) / viewport.zoom);
    expect((next.height / 2 - resized.y) / resized.zoom).toBe((previous.height / 2 - viewport.y) / viewport.zoom);
    expect(resizeGraphViewport(resized, next, previous)).toEqual(viewport);
  });
  it("normalizes duplicates and invalid endpoints without losing direction or dev dependencies", () => {
    expect(
      normalizeGraph({
        nodes: [node("a"), node("a"), node("b")],
        edges: [edge("a", "b"), edge("a", "b"), edge("a", "b", true), edge("b", "a"), edge("a", "missing")],
      }),
    ).toEqual({ nodes: [node("a"), node("b")], edges: [edge("a", "b"), edge("a", "b", true), edge("b", "a")] });
  });
  it("lays out cycles and isolated nodes deterministically without overlapping", () => {
    const first = layoutGraph(graph);
    expect(layoutGraph({ nodes: [...graph.nodes].reverse(), edges: [...graph.edges].reverse() })).toEqual(first);
    expect(new Set(first.map((item) => `${item.position.x}:${item.position.y}`)).size).toBe(4);
    expect(first.every((item) => Number.isFinite(item.position.x) && Number.isFinite(item.position.y))).toBe(true);
  });
  it("places dependencies after their source", () => {
    const positions = layoutGraph({
      nodes: [node("a"), node("b"), node("c")],
      edges: [edge("a", "b"), edge("b", "c")],
    });
    expect(positions[0].position.x).toBeLessThan(positions[1].position.x);
    expect(positions[1].position.x).toBeLessThan(positions[2].position.x);
  });
  it("keeps isolated nodes accessible, including an entirely disconnected graph", () => {
    expect(graphSlice(graph, {}).hiddenIsolated).toBe(1);
    expect(graphSlice(graph, { includeIsolated: true }).nodes).toHaveLength(4);
    expect(graphSlice({ ...graph, edges: [] }, {}).nodes).toHaveLength(4);
    expect(graphSlice(graph, { focusId: "isolated" }).nodes.map((item) => item.id)).toEqual(["isolated"]);
  });
  it("bounds rendering and preserves a focused node outside the default slice", () => {
    const large = {
      nodes: Array.from({ length: 180 }, (_, i) => node(String(i))),
      edges: Array.from({ length: 179 }, (_, i) => edge("179", String(i))),
    };
    const slice = graphSlice(large, { focusId: "179", nodeLimit: 5, edgeLimit: 2 });
    expect(slice.nodes).toHaveLength(5);
    expect(slice.nodes.some((item) => item.id === "179")).toBe(true);
    expect(slice.edges).toHaveLength(2);
    expect(slice.omittedNodes).toBe(175);
    expect(slice.omittedEdges).toBe(177);
    expect(
      slice.edges.every(
        (item) => slice.nodes.some((n) => n.id === item.source) && slice.nodes.some((n) => n.id === item.target),
      ),
    ).toBe(true);
  });
  it("filters symbols by file but includes real cross-file neighbors when focused", () => {
    const symbols = { nodes: [node("one", "src/a.ts"), node("two", "src/b.ts")], edges: [edge("one", "two")] };
    expect(graphSlice(symbols, { filePath: "src/a.ts" }).nodes.map((item) => item.id)).toEqual(["one"]);
    expect(graphSlice(symbols, { filePath: "src/a.ts", focusId: "one" }).nodes).toHaveLength(2);
    expect(searchNodes(symbols.nodes, "SRC/B")).toEqual([node("two", "src/b.ts")]);
  });
  it("handles empty data and missing focus IDs", () => {
    expect(graphSlice({ nodes: [], edges: [] }, {}).nodes).toEqual([]);
    expect(graphSlice(graph, { focusId: "deleted" }).nodes).toEqual(graphSlice(graph, {}).nodes);
  });
});
