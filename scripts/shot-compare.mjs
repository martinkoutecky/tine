// Screenshot the published comparison or a standalone generated preview.
import { chromium } from "playwright";
import { pathToFileURL } from "node:url";
import fs from "node:fs";
import path from "node:path";

const src = process.argv[2] || new URL("../website/compare.html", import.meta.url).pathname;
const out = path.resolve(process.argv[3] || "artifacts/compare-page");
fs.mkdirSync(out, { recursive: true });
const browser = await chromium.launch();
for (const width of [1200, 390]) {
  const page = await browser.newPage({ viewport: { width, height: 1000 } });
  await page.goto(pathToFileURL(path.resolve(src)).href, { waitUntil: "networkidle" });
  await page.screenshot({ path: `${out}/compare-${width}.png`, fullPage: true });
  await page.locator("table.cmp").first().screenshot({
    path: `${out}/compare-table-${width}.png`, style: ".nav { visibility: hidden; }",
  });
  console.log(JSON.stringify(await page.evaluate(() => ({
    width: innerWidth, documentWidth: document.documentElement.scrollWidth,
    tables: [...document.querySelectorAll(".tbl-wrap")].map((table) => ({
      visibleWidth: table.clientWidth, contentWidth: table.scrollWidth,
    })),
  }))));
  await page.close();
}
await browser.close();
console.log(`Wrote desktop and 390px comparison screenshots to ${out}`);
