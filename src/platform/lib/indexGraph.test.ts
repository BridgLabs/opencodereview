import { describe, expect, it } from "vitest";
import type { IndexGraph } from "../../types";
import {
  graphSlice,
  isTestFile,
  layoutGraph,
  normalizeGraph,
  resizeGraphViewport,
  searchNodes,
  withoutTests,
} from "./indexGraph";

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
  it("drops a node that would start another cluster with no room to join it", () => {
    const graph = {
      nodes: [node("a"), node("hub"), node("b"), node("c")],
      edges: [edge("hub", "a"), edge("b", "c")],
    };
    const slice = graphSlice(graph, { nodeLimit: 3 });
    expect(slice.nodes.map((item) => item.id)).toEqual(["a", "hub"]);
    expect(slice.edges).toEqual([edge("hub", "a")]);
  });
  it("shows the best connected nodes first when a graph is cut down", () => {
    const hub = {
      nodes: [node("a"), node("b"), node("c"), node("hub")],
      edges: [edge("hub", "a"), edge("hub", "b"), edge("hub", "c"), edge("a", "b")],
    };
    expect(graphSlice(hub, { nodeLimit: 2 }).nodes.map((item) => item.id)).toEqual(["hub", "a"]);
  });
  it("recognizes test files across languages", () => {
    for (const path of [
      "spec/a_spec.rb",
      "test/models/user_test.rb",
      "src/a.test.ts",
      "pkg/a_test.go",
      "tests/test_a.py",
      "x/__tests__/a.js",
    ]) {
      expect(isTestFile(path), path).toBe(true);
    }
    for (const path of ["lib/inspector.rb", "src/latest.ts", "app/models/user.rb", "lib/spectrum/a.rb"]) {
      expect(isTestFile(path), path).toBe(false);
    }
  });
  it("drops test files and the edges that touch them", () => {
    const graph = {
      nodes: [node("lib", "lib/a.rb"), node("other", "lib/b.rb"), node("spec", "spec/a_spec.rb")],
      edges: [edge("lib", "other"), edge("spec", "lib")],
    };
    expect(withoutTests(graph)).toEqual({
      nodes: [node("lib", "lib/a.rb"), node("other", "lib/b.rb")],
      edges: [edge("lib", "other")],
    });
  });
  it("never leaves a visible node without a visible edge when it cuts a large graph", () => {
    const nodes = Array.from({ length: 60 }, (_, i) => node(`n${String(i).padStart(2, "0")}`));
    const edges = nodes
      .slice(1)
      .map((item, i) => edge(i % 7 === 0 ? "n00" : `n${String((i % 20) + 1).padStart(2, "0")}`, item.id));
    const slice = graphSlice({ nodes, edges }, { nodeLimit: 25, edgeLimit: 40 });
    expect(slice.nodes).toHaveLength(25);
    const touched = new Set(slice.edges.flatMap((item) => [item.source, item.target]));
    expect(slice.nodes.every((item) => touched.has(item.id))).toBe(true);
  });
});
