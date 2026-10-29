// Production SeenHeader layout in page, split-pane and sidebar scroll shells.
// Native routed-page/git proof: e2e-ui-sticky-selection.mjs. Run with Vite active.
import { chromium } from "playwright";
import fs from "node:fs";
const out = "artifacts/ui-sticky/browser";
fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(`${out}/fixture.html`, '<!doctype html><html><body style="margin:0"><div id="fixture"></div><script type="module" src="/scripts/fixtures/ui-sticky.tsx"></script></body></html>');
const browser = await chromium.launch({ args: ["--no-sandbox"] });
const observations = [];
try {
  for (const width of [1280, 390]) for (const theme of ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width, height: 900 } });
    await page.goto(`${process.env.TINE_URL || "http://localhost:5181"}/${out}/fixture.html?theme=${theme}&panes=${width === 1280 ? "all" : "one"}`);
    await page.locator("[data-seen-header]").first().waitFor();
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: `${out}/${width}-${theme}-title.png` });
    await page.evaluate(() => document.querySelectorAll(".main-content, .right-sidebar").forEach(el => { el.scrollTop = 400; }));
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const geometry = await page.evaluate(() => [...document.querySelectorAll("[data-seen-header]")].map(header => {
      const pane = header.closest(".main-content, .right-sidebar"), r = header.getBoundingClientRect(), p = pane.getBoundingClientRect();
      return { headerTop: r.top, paneTop: p.top, left: r.left, right: r.right, paneLeft: p.left, paneRight: p.right,
        padding: parseFloat(getComputedStyle(pane).scrollPaddingTop), height: r.height };
    }));
    observations.push({ width, theme, geometry });
    await page.screenshot({ path: `${out}/${width}-${theme}-scrolled.png` });
    for (const g of geometry) if (Math.abs(g.headerTop - g.paneTop) > 3 || g.left < g.paneLeft || g.right > g.paneRight || g.padding < g.height)
      throw Error(`Sticky notice or reveal space failed: ${JSON.stringify({ width, theme, g })}`);
    await page.close();
  }
  console.log(`PASS sticky notice: main/split/sidebar shells and mobile width, light/dark; ${out}`);
} finally {
  fs.writeFileSync(`${out}/geometry.json`, JSON.stringify(observations, null, 2));
  await browser.close();
}
