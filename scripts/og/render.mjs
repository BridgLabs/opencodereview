// Renders the share image (scripts/og/og.html → site/og.png, 1200x630) and the PNG icons
// (site/favicon.svg → apple-touch-icon, manifest icons, favicon.ico) with Playwright's Chromium.
// Usage: node scripts/og/render.mjs
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium } from "playwright";

const here = path.dirname(fileURLToPath(import.meta.url));
const site = path.join(here, "../../site");
const sprite = await readFile(path.join(here, "sprite.svg"), "utf8");

const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, deviceScaleFactor: 1 });
  await page.goto(pathToFileURL(path.join(here, "og.html")).href);
  await page.evaluate((svg) => document.body.insertAdjacentHTML("afterbegin", svg), sprite);
  await page.evaluate(() => document.fonts.ready);
  await page.waitForLoadState("networkidle");
  await page.screenshot({ path: path.join(site, "og.png"), type: "png" });
  console.log("wrote site/og.png");
  await page.close();
  await renderIcons(browser);
} finally {
  await browser.close();
}

async function renderIcons(browser) {
  const svg = await readFile(path.join(site, "favicon.svg"), "utf8");
  const icons = [
    // The touch icon sits on the canvas colour because iOS fills transparency with black.
    { file: "apple-touch-icon.png", size: 180, background: "#111111", inset: 0.14 },
    { file: "icon-192.png", size: 192, background: "#111111", inset: 0.14 },
    { file: "icon-512.png", size: 512, background: "#111111", inset: 0.14 },
    { file: "favicon.ico", size: 32, background: "transparent", inset: 0 },
  ];
  for (const { file, size, background, inset } of icons) {
    const page = await browser.newPage({ viewport: { width: size, height: size }, deviceScaleFactor: 1 });
    const pad = Math.round(size * inset);
    await page.setContent(
      `<style>*{margin:0}body{width:${size}px;height:${size}px;background:${background};display:grid;place-items:center}svg{width:${size - pad * 2}px;height:${size - pad * 2}px}</style>${svg}`,
    );
    const png = await page.screenshot({ type: "png", omitBackground: background === "transparent" });
    await page.close();
    await writeFile(path.join(site, file), file.endsWith(".ico") ? ico(png, size) : png);
    console.log(`wrote site/${file}`);
  }
}

/** Wraps one PNG in an ICO header, which every browser accepts as favicon.ico. */
function ico(png, size) {
  const header = Buffer.alloc(22);
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(1, 4); // one image
  header.writeUInt8(size, 6);
  header.writeUInt8(size, 7);
  header.writeUInt16LE(1, 10); // colour planes
  header.writeUInt16LE(32, 12); // bits per pixel
  header.writeUInt32LE(png.length, 14);
  header.writeUInt32LE(22, 18); // image data offset
  return Buffer.concat([header, png]);
}
