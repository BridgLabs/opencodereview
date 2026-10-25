import { describe, expect, it } from "vitest";
import type { IndexJob } from "../../services/setupApi";
import type { RepoIndexState } from "../../types";
import { indexGroups, indexingWorkspaceReducer, indexRevision, selectRepository } from "./indexing";

const job = (id: number, status: RepoIndexState) => ({ repo: { id }, status: { status } }) as IndexJob;

describe("indexing workspace", () => {
  it("groups every backend state exactly once", () => {
    const states = indexGroups.flatMap((group) => group.states);
    expect(new Set(states).size).toBe(8);
    expect([...states].sort()).toEqual(
      ["ready", "queued", "indexing", "stale", "failed", "cancelled", "not_indexed", "empty"].sort(),
    );
  });
  it("prefers ready repos initially but preserves selection during polling", () => {
    const jobs = [job(1, "failed"), job(2, "ready")];
    expect(selectRepository(jobs, null)).toBe(2);
    expect(selectRepository(jobs, 1)).toBe(1);
    expect(selectRepository([job(1, "indexing"), job(2, "ready")], 1)).toBe(1);
    expect(selectRepository(jobs, 99)).toBe(2);
    expect(selectRepository([], 1)).toBeNull();
  });
  it("restores the repo list when inspection closes and keeps overlays exclusive", () => {
    const initial = { listOpen: true, restoreList: true, nodeId: null };
    const inspecting = indexingWorkspaceReducer(initial, { type: "inspect", nodeId: "a" });
    expect(inspecting).toEqual({ listOpen: false, restoreList: true, nodeId: "a" });
    expect(indexingWorkspaceReducer(inspecting, { type: "close" })).toEqual(initial);
    expect(indexingWorkspaceReducer(inspecting, { type: "toggle-list" })).toEqual(initial);
    expect(indexingWorkspaceReducer(inspecting, { type: "reset", wide: false })).toEqual({
      listOpen: false,
      restoreList: false,
      nodeId: null,
    });
  });
  it("keeps inspection open on resize but does not restore the list on mobile", () => {
    const next = indexingWorkspaceReducer(
      { listOpen: false, restoreList: true, nodeId: "a" },
      { type: "resize", wide: false },
    );
    expect(next.nodeId).toBe("a");
    expect(indexingWorkspaceReducer(next, { type: "close" }).listOpen).toBe(false);
  });
  it("uses completion time, not just SHA, to identify a new snapshot", () => {
    expect(indexRevision({ indexedAt: 1, baseSha: "same" })).not.toBe(indexRevision({ indexedAt: 2, baseSha: "same" }));
    expect(indexRevision({ indexedAt: null, baseSha: "same" })).toBeNull();
  });
});
