import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
// @ts-expect-error -- plain ESM script without type declarations
import { absoluteLinks, faqEntries } from "./build.mjs";

const site = (file: string) => resolve("site", file);
const html = readFileSync(site("index.html"), "utf8");
const origin = "https://opencodereview.run";

function structuredData() {
  const json = html.match(/<script type="application\/ld\+json" id="structured-data">([\s\S]*?)<\/script>/)?.[1];
  return JSON.parse(json ?? "null");
}

describe("landing page SEO", () => {
  it("has generated files that match their sources", () => {
    const result = spawnSync(process.execPath, ["scripts/site/build.mjs", "--check"], { encoding: "utf8" });
    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
  });

  it("publishes structured data whose FAQ is the visible FAQ", () => {
    const graph = structuredData()["@graph"];
    expect(graph.map((node: { "@type": string }) => node["@type"])).toEqual([
      "Organization",
      "WebSite",
      "SoftwareApplication",
      "FAQPage",
    ]);
    const faq = faqEntries(html);
    expect(faq.length).toBe(html.match(/<details>/g)?.length);
    expect(graph[3].mainEntity.map((q: { name: string }) => q.name)).toEqual(
      faq.map((entry: { question: string }) => entry.question),
    );
  });

  it("serves every file the head and the generated files point to", () => {
    const head = html.slice(0, html.indexOf("</head>"));
    const references = [
      ...head.matchAll(
        /(?:href|content)="(?:https:\/\/opencodereview\.run\/)?([\w.-]+\.(?:png|svg|ico|webmanifest|txt|css))/g,
      ),
      ...readFileSync(site("site.webmanifest"), "utf8").matchAll(/"src": "\/([^"]+)"/g),
      ...readFileSync(site("sitemap.xml"), "utf8").matchAll(/<image:loc>https:\/\/opencodereview\.run\/([^<]+)/g),
    ].map((match) => match[1]);
    expect(references.length).toBeGreaterThan(8);
    for (const file of references) expect(existsSync(site(file)), file).toBe(true);
  });

  it("lets crawlers in and points them at the sitemap", () => {
    const robots = readFileSync(site("robots.txt"), "utf8");
    expect(robots).not.toMatch(/Disallow:\s*\/\s*$/m);
    expect(robots).toContain(`Sitemap: ${origin}/sitemap.xml`);
    expect(readFileSync(site("sitemap.xml"), "utf8")).toMatch(/<lastmod>\d{4}-\d{2}-\d{2}<\/lastmod>/);
  });

  it("keeps llms.txt in the llmstxt.org shape with absolute links", () => {
    const llms = readFileSync(site("llms.txt"), "utf8");
    expect(llms).toMatch(/^# OpenCodeReview\n\n> /);
    for (const [, url] of llms.matchAll(/\]\(([^)]+)\)/g)) expect(url).toMatch(/^https:\/\//);
    expect(llms).toContain(`${origin}/llms-full.txt`);
  });

  it("points llms-full.txt images at raw files and links at GitHub pages", () => {
    const repo = "https://github.com/BridgLabs/opencodereview";
    const markdown = "![Queue](../site/a.png) [![Badge](https://x.test/b.svg)](../LICENSE) [Reviews](reviews.md#scope)";
    expect(absoluteLinks(markdown, "docs/x.md")).toBe(
      `![Queue](${repo}/raw/main/site/a.png) [![Badge](https://x.test/b.svg)](${repo}/blob/main/LICENSE) [Reviews](${repo}/blob/main/docs/reviews.md#scope)`,
    );
  });
});
