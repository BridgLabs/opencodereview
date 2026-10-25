import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import mascotUrl from "../../assets/mascot.svg";
import { BrandMark } from "./BrandMark";

const parse = (html: string) => new DOMParser().parseFromString(html, "text/html");

describe("OpenCodeReview branding", () => {
  it.each([
    ["sm", 36],
    ["md", 64],
    ["lg", 128],
  ] as const)("renders the square %s mascot as decoration", (size, pixels) => {
    const document = parse(renderToStaticMarkup(<BrandMark size={size} />));
    const mark = document.querySelector(`.brand-mark.${size}`)!;
    expect(mark.getAttribute("aria-hidden")).toBe("true");
    const image = mark.querySelector("img")!;
    expect(image.getAttribute("src")).toBe(mascotUrl);
    expect(image.getAttribute("alt")).toBe("");
    expect(image.getAttribute("width")).toBe(String(pixels));
    expect(image.getAttribute("height")).toBe(String(pixels));
  });

  it("uses the mascot favicon on every entry page", () => {
    for (const file of ["index.html", "assets/onboarding.html"]) {
      const document = parse(readFileSync(file, "utf8"));
      expect(document.querySelector('link[rel="icon"]')?.getAttribute("href")).toBe("/assets/favicon.svg");
      expect(document.querySelector('link[rel="icon"]')?.getAttribute("type")).toBe("image/svg+xml");
      expect(document.querySelector("title")?.textContent).toContain("OpenCodeReview");
    }
    expect(parse(readFileSync("assets/onboarding.html", "utf8")).querySelector(".logo img")?.getAttribute("src")).toBe(
      "/assets/mascot.svg",
    );
  });
});
