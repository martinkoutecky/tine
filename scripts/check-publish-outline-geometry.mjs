// Browser geometry regression for the exported Guide's outline connectors and
// inline block-embed root. Run after `npm run docs:build`.
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
const PAGE = path.join(ROOT, "website/demo/index.html");
const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu"] });
try {
  const page = await browser.newPage({ viewport: { width: 900, height: 900 } });
  await page.goto(pathToFileURL(PAGE).href);
  const result = await page.evaluate(() => {
    const nested = document.querySelector("ul.outline > li > ul");
    const child = nested?.querySelector(":scope > li");
    if (!nested || !child) return { error: "nested outline fixture missing" };
    const ulRect = nested.getBoundingClientRect();
    const liRect = child.getBoundingClientRect();
    const guide = getComputedStyle(nested, "::before");
    const bullet = getComputedStyle(child, "::before");
    const guideX = ulRect.left + Number.parseFloat(guide.left || "NaN") + Number.parseFloat(guide.borderLeftWidth || "0") / 2;
    const bulletX = liRect.left + Number.parseFloat(bullet.left || "NaN") + Number.parseFloat(bullet.width || "0") / 2;
    const embed = document.querySelector(".block-embed.single-root");
    const embedList = embed?.querySelector(":scope > ul.embed-outline");
    const embedItem = embedList?.querySelector(":scope > li");
    return {
      guideX,
      bulletX,
      delta: Math.abs(guideX - bulletX),
      guideContent: guide.content,
      embedFound: Boolean(embed && embedList && embedItem),
      embedBorder: embed ? getComputedStyle(embed).borderLeftWidth : null,
      embedGuideContent: embedList ? getComputedStyle(embedList, "::before").content : null,
      embedBulletContent: embedItem ? getComputedStyle(embedItem, "::before").content : null,
    };
  });
  if (result.error) throw new Error(result.error);
  if (result.guideContent === "none" || !(result.delta <= 1)) {
    throw new Error(`outline guide is not centered on its bullet: ${JSON.stringify(result)}`);
  }
  if (!result.embedFound || result.embedBorder !== "0px" || !["none", "normal"].includes(result.embedGuideContent) || !["none", "normal"].includes(result.embedBulletContent)) {
    throw new Error(`block embed still duplicates outline geometry: ${JSON.stringify(result)}`);
  }
  fs.mkdirSync(path.join(ROOT, "test-results"), { recursive: true });
  await page.screenshot({ path: path.join(ROOT, "test-results/publish-outline-geometry.png"), fullPage: true });
  console.log(`PASS: exported guide/bullet delta ${result.delta.toFixed(2)}px and embed has one root marker`);
} finally {
  await browser.close();
}
