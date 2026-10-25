import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const parse = (file: string) => {
  const source = readFileSync(file, "utf8");
  return { source, document: new DOMParser().parseFromString(source, "image/svg+xml") };
};
const mascot = parse("assets/mascot.svg");
const favicon = parse("assets/favicon.svg");

describe("OpenCodeReview mascot SVG", () => {
  it.each([["mascot", mascot], ["favicon", favicon]] as const)("the %s is accessible, self-contained, square vector artwork", (_, { source, document }) => {
    const root = document.documentElement;
    expect(document.querySelector("parsererror")).toBeNull();
    expect(root.getAttribute("viewBox")).toBe("0 0 128 128");
    expect(root.getAttribute("role")).toBe("img");
    for (const id of root.getAttribute("aria-labelledby")!.split(" ")) expect(document.getElementById(id)?.textContent).toBe("OpenCodeReview mascot");
    expect(root.querySelector("image, script, foreignObject, text")).toBeNull();
    expect(source).not.toMatch(/(?:href\s*=|@import|data:image)/i);
  });

  it("shares the bubble with the favicon and keeps the favicon still", () => {
    const bubble = (doc: Document) => doc.querySelector('[data-part="bubble"]')?.getAttribute("d");
    expect(bubble(favicon.document)).toBeTruthy();
    expect(bubble(favicon.document)).toBe(bubble(mascot.document));
    expect(favicon.document.querySelector("style, animate")).toBeNull();
  });

  it("namespaces every id, class and keyframe under ocr-mascot so copies can be renamed", () => {
    const root = mascot.document.documentElement;
    expect(root.id).toBe("ocr-mascot");
    expect(root.getAttribute("data-motion")).toBe("none");
    for (const [, name] of mascot.source.matchAll(/@keyframes ([\w-]+)/g)) expect(name.startsWith("ocr-mascot-")).toBe(true);
    for (const name of ["eyes", "eye-left", "eye-right"]) expect(root.querySelector(`[data-part="${name}"]`)).not.toBeNull();
  });

  it("only moves when motion is allowed", () => {
    const style = mascot.document.querySelector("style")!.textContent!;
    const outside = style.replace(/@media \(prefers-reduced-motion: no-preference\) \{[\s\S]*?\n {4}\}/, "");
    expect(style).toContain("prefers-reduced-motion: no-preference");
    expect(outside).not.toMatch(/animation:/);
  });
});
