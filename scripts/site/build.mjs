// Writes the generated parts of the landing page from the files they describe, so they never drift:
//   - the JSON-LD block in site/index.html (its FAQPage comes from the page's own FAQ section)
//   - site/llms-full.txt, the README and docs as one Markdown file for language models (not committed)
//   - the lastmod date in site/sitemap.xml
// The Site workflow runs it before every deploy. `--check` only verifies the committed JSON-LD, so a
// docs change never needs a site rebuild in its pull request.
// Usage: node scripts/site/build.mjs [--check]
import { execFileSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "../..");
const origin = "https://opencodereview.run";
const repo = "https://github.com/BridgLabs/opencodereview";
const docs = ["architecture", "configuration", "reviews", "indexing", "skills", "upgrading", "development"];

const decode = (text) =>
  text
    .replace(/<[^>]+>/g, "")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();

/** The FAQ entries exactly as the page shows them. */
export function faqEntries(html) {
  const entries = [...html.matchAll(/<details><summary>([\s\S]*?)<\/summary><p>([\s\S]*?)<\/p><\/details>/g)];
  return entries.map(([, question, answer]) => ({ question: decode(question), answer: decode(answer) }));
}

export function structuredData(html) {
  const organization = {
    "@type": "Organization",
    "@id": `${origin}/#organization`,
    name: "BRIDG LABS LTDA",
    url: origin,
    logo: `${origin}/icon-512.png`,
    sameAs: ["https://github.com/BridgLabs"],
  };
  const website = {
    "@type": "WebSite",
    "@id": `${origin}/#website`,
    name: "OpenCodeReview",
    url: `${origin}/`,
    inLanguage: "en",
    publisher: { "@id": `${origin}/#organization` },
  };
  const application = {
    "@type": "SoftwareApplication",
    "@id": `${origin}/#software`,
    name: "OpenCodeReview",
    url: `${origin}/`,
    description:
      "Self-hosted, source-available AI code review for GitHub pull requests. Reviews run on the Claude Code, Codex or Gemini CLI with the account you already have, or on OpenCode's free models.",
    applicationCategory: "DeveloperApplication",
    operatingSystem: "Linux, macOS, Windows (Docker)",
    image: `${origin}/og.png`,
    screenshot: `${origin}/app-reviews.png`,
    codeRepository: repo,
    license: `${repo}/blob/main/LICENSE`,
    isAccessibleForFree: true,
    offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
    featureList: [
      "Reviews every GitHub pull request with Claude Code, Codex, Gemini CLI or OpenCode",
      "Uses the Claude, ChatGPT or Google account you already have, or free models",
      "Fails over between several AI accounts when one hits its usage limit",
      "Indexes each repository for callers, dependencies, tests and convention files",
      "Checks every finding against the changed code before publishing it",
      "Runs the AI CLI in isolated Docker containers on your own machine",
    ],
    publisher: { "@id": `${origin}/#organization` },
  };
  const faq = {
    "@type": "FAQPage",
    "@id": `${origin}/#faq`,
    mainEntity: faqEntries(html).map(({ question, answer }) => ({
      "@type": "Question",
      name: question,
      acceptedAnswer: { "@type": "Answer", text: answer },
    })),
  };
  return { "@context": "https://schema.org", "@graph": [organization, website, application, faq] };
}

function withStructuredData(html) {
  // `<` is escaped so no answer text can close the script element.
  const json = JSON.stringify(structuredData(html), null, 2).replace(/</g, "\\u003c").replace(/\n/g, "\n  ");
  return html.replace(
    /(<script type="application\/ld\+json" id="structured-data">)[\s\S]*?(<\/script>)/,
    (_, open, close) => `${open}\n  ${json}\n  ${close}`,
  );
}

/** Rewrites relative links in a repository Markdown file to absolute GitHub URLs. */
function absoluteLinks(markdown, file) {
  const dir = path.posix.dirname(file);
  return markdown.replace(/(!?)\]\((?!https?:|#|mailto:)([^)\s]+)\)/g, (_, bang, target) => {
    const resolved = path.posix.normalize(path.posix.join(dir, target));
    const base = bang ? `${repo}/raw/main` : `${repo}/blob/main`;
    return `${bang}](${base}/${resolved})`;
  });
}

async function llmsFull() {
  const parts = [
    "# OpenCodeReview — full documentation",
    "",
    `> The README and the documentation of OpenCodeReview (${repo}), joined into one Markdown file for language models. Summary and links: ${origin}/llms.txt`,
  ];
  for (const file of ["README.md", ...docs.map((name) => `docs/${name}.md`)]) {
    // The README opens with a centred HTML logo, which says nothing in plain text.
    const markdown = (await readFile(path.join(root, file), "utf8")).replace(/^<p align="center">[\s\S]*?<\/p>\s*/, "");
    parts.push("", "---", "", `Source: ${repo}/blob/main/${file}`, "", absoluteLinks(markdown, file).trim());
  }
  return `${parts.join("\n")}\n`;
}

function lastCommitDate() {
  try {
    return execFileSync("git", ["log", "-1", "--format=%cs"], { cwd: root, encoding: "utf8" }).trim();
  } catch {
    return new Date().toISOString().slice(0, 10);
  }
}

const check = process.argv.includes("--check");
const outputs = [{ file: "site/index.html", build: async (current) => withStructuredData(current) }];
if (!check) {
  outputs.push(
    { file: "site/llms-full.txt", build: async () => llmsFull() },
    {
      file: "site/sitemap.xml",
      build: async (current) => current.replace(/<lastmod>[^<]*<\/lastmod>/, `<lastmod>${lastCommitDate()}</lastmod>`),
    },
  );
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const stale = [];
  for (const { file, build } of outputs) {
    const target = path.join(root, file);
    const current = await readFile(target, "utf8").catch(() => "");
    const next = await build(current);
    if (next === current) continue;
    if (check) stale.push(file);
    else {
      await writeFile(target, next);
      console.log(`wrote ${file}`);
    }
  }
  if (stale.length) {
    console.error(`Out of date: ${stale.join(", ")}. Run: pnpm site:build`);
    process.exit(1);
  }
}
