import { act, type ComponentProps, useState } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FindingAuditView, FindingCandidate } from "../../types";
import { FindingSelection } from "./FindingSelection";

const candidate = (
  id: string,
  aiDecision: FindingCandidate["aiDecision"],
  blockedReason: string | null = null,
): FindingCandidate => ({
  id,
  aiDecision,
  blockedReason,
  selected: aiDecision === "kept",
  defaultSelected: aiDecision === "kept",
  origin: "chunk",
  chunkId: 1,
  aiReason: `${id} evidence`,
  body: `<sub>🤖 Automated review by OpenCodeReview</sub>\n\n[Finding] ${id} title\n\n${id} explanation`,
  originalBody: null,
  comment: { path: "src/example.ts", line: 1 },
});
const view: FindingAuditView = {
  runId: 10,
  prId: 1,
  revision: 2,
  requiresConfirmation: true,
  selectionSaved: false,
  editable: true,
  audit: {
    headSha: "abc",
    complete: true,
    originalBody: "Original summary",
    candidates: [
      candidate("kept", "kept"),
      candidate("discarded", "discarded"),
      candidate("blocked", "kept", "Invalid anchor"),
      candidate("unknown", "unmapped"),
    ],
  },
};
const props: ComponentProps<typeof FindingSelection> = {
  view,
  selectedIds: ["kept"],
  loading: false,
  busy: false,
  dirty: false,
  conflict: false,
  error: null,
  onToggle: () => undefined,
  onSave: () => undefined,
  onReset: () => undefined,
};
const render = (overrides: Partial<typeof props> = {}) =>
  renderToStaticMarkup(
    <FindingSelection {...props} {...overrides}>
      <p>Legacy final output</p>
    </FindingSelection>,
  );
const tab = (container: HTMLElement, label: string) =>
  [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find((button) =>
    button.textContent?.startsWith(label),
  )!;
const click = async (element: HTMLElement) => act(async () => element.click());

afterEach(() => vi.unstubAllGlobals());

async function interact(overrides: Partial<typeof props>, test: (container: HTMLElement) => Promise<void>) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  function Harness() {
    const [selectedIds, setSelectedIds] = useState(overrides.selectedIds ?? props.selectedIds);
    return (
      <FindingSelection
        {...props}
        {...overrides}
        selectedIds={selectedIds}
        onToggle={(id) => {
          overrides.onToggle?.(id);
          setSelectedIds((current) =>
            current.includes(id) ? current.filter((value) => value !== id) : [...current, id],
          );
        }}
      >
        <p>Legacy final output</p>
      </FindingSelection>
    );
  }
  try {
    await act(async () => root.render(<Harness />));
    await test(container);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
}

describe("tabbed finding selection", () => {
  it("defaults to eligible findings and renders only the selected panel", () => {
    const html = render();
    expect(html).toContain('role="tablist"');
    expect(html).toContain('data-finding-group="eligible"');
    expect(html).toContain("Eligible findings");
    expect(html).toContain("Discarded by AI");
    expect(html).toContain("Blocked by validation");
    expect(html).toContain("Original AI summary");
    expect(html).toContain("kept explanation");
    expect(html).toContain("<sub>🤖 Automated review by OpenCodeReview</sub>");
    expect(html).not.toContain("discarded explanation");
    expect(html).not.toContain("Original summary");
    expect(html).not.toContain("Legacy final output");
    expect(html).not.toContain("<select");
  });

  it("navigates with arrows, preserves choices across tabs and keeps blocked findings disabled", async () => {
    const onToggle = vi.fn();
    const onSave = vi.fn();
    const onReset = vi.fn();
    await interact({ onToggle, onSave, onReset }, async (container) => {
      const previous = container.querySelector<HTMLButtonElement>('[aria-label="Previous finding group"]')!;
      const next = container.querySelector<HTMLButtonElement>('[aria-label="Next finding group"]')!;
      expect(previous.disabled).toBe(true);
      await click(next);
      expect(tab(container, "Discarded by AI").getAttribute("aria-selected")).toBe("true");
      const discarded = container.querySelector<HTMLInputElement>('[aria-label="Include finding discarded"]')!;
      expect(discarded.disabled).toBe(false);
      expect(discarded.checked).toBe(false);
      expect(container.textContent).toContain("discarded evidence");
      await click(discarded);
      expect(onToggle).toHaveBeenCalledExactlyOnceWith("discarded");
      await click(next);
      expect(container.querySelector<HTMLInputElement>('[aria-label="Include finding blocked"]')!.disabled).toBe(true);
      expect(container.textContent).toContain("Invalid anchor");
      await click(tab(container, "Original AI summary"));
      expect(container.querySelector('[role="tabpanel"]')?.textContent).toContain("Original summary");
      expect(container.querySelector(".finding-candidate")).toBeNull();
      expect(next.disabled).toBe(true);
      await click(tab(container, "Discarded by AI"));
      expect(container.querySelector<HTMLInputElement>('[aria-label="Include finding discarded"]')!.checked).toBe(true);
      expect(container.textContent).toContain("Manual override");
      expect(container.querySelectorAll('[role="tabpanel"]')).toHaveLength(1);
      expect(onSave).not.toHaveBeenCalled();
      expect(onReset).not.toHaveBeenCalled();
    });
  });

  it("supports keyboard navigation and labels the active panel", async () => {
    await interact({}, async (container) => {
      tab(container, "Eligible findings").focus();
      for (const [key, label] of [
        ["ArrowRight", "Discarded by AI"],
        ["End", "Original AI summary"],
        ["ArrowRight", "Eligible findings"],
        ["ArrowLeft", "Original AI summary"],
        ["Home", "Eligible findings"],
      ]) {
        await act(async () =>
          document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true })),
        );
        const selected = tab(container, label);
        expect(document.activeElement).toBe(selected);
        expect(selected.tabIndex).toBe(0);
        expect(selected.getAttribute("aria-selected")).toBe("true");
        const panel = container.querySelector('[role="tabpanel"]')!;
        expect(selected.getAttribute("aria-controls")).toBe(panel.id);
        expect(panel.getAttribute("aria-labelledby")).toBe(selected.id);
      }
    });
  });

  it("keeps questions optional and exposes unmapped and pending evidence in their own tabs", async () => {
    const question = {
      ...candidate("question", "kept"),
      kind: "question" as const,
      body: "[Question] Confirm rollout",
      selected: false,
      defaultSelected: false,
    };
    await interact(
      {
        view: {
          ...view,
          audit: {
            ...view.audit!,
            candidates: [
              candidate("kept", "kept"),
              question,
              candidate("unknown", "unmapped"),
              candidate("pending", "pending"),
            ],
          },
        },
      },
      async (container) => {
        expect(container.textContent).toContain("1 findings · 0 questions selected");
        expect(container.textContent).toContain("cannot alone justify Request changes");
        await click(tab(container, "Questions — optional publication"));
        expect(container.querySelector<HTMLInputElement>('[aria-label="Include question question"]')!.checked).toBe(
          false,
        );
        await click(tab(container, "Decision unavailable"));
        expect(container.textContent).toContain("unknown evidence");
        await click(tab(container, "Awaiting final review"));
        expect(container.textContent).toContain("pending evidence");
      },
    );
  });

  it("keeps historical final output and explains missing discard history", () => {
    const html = render({ view: { ...view, editable: false, audit: null } });
    expect(html).toContain("Legacy final output");
    expect(html).toContain('data-finding-group="unavailable"><summary>');
    expect(html).toContain("History unavailable");
    expect(html).not.toContain("0 discarded");
    expect(html).not.toContain("Save selection");
  });

  it("does not ask for confirmation when no candidates were generated", () => {
    const html = render({ view: { ...view, audit: { ...view.audit!, candidates: [] } }, selectedIds: [] });
    expect(html).toContain("Choose a review decision below");
    expect(html).not.toContain("Save the empty selection");
    expect(html).not.toContain("Confirm findings");
    expect(html).not.toContain("finding-selection-actions");
  });

  it.each([{ conflict: true }, { busy: true }, { error: "Audit failed" }, { view: { ...view, editable: false } }])(
    "blocks editing across tabs for unavailable or read-only states %j",
    async (overrides) => {
      await interact(overrides, async (container) => {
        expect(container.querySelector<HTMLInputElement>('[aria-label="Include finding kept"]')!.disabled).toBe(true);
        await click(tab(container, "Discarded by AI"));
        expect(container.querySelector<HTMLInputElement>('[aria-label="Include finding discarded"]')!.disabled).toBe(
          true,
        );
      });
    },
  );
});
