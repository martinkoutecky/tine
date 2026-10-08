// Two README feature shots that need real navigation: built-in tabs (open a few
// via middle-click on page-ref links, pin one) and PDF highlights (text + area).
// Each runs in its own fresh page so state can't leak. Usage (after env.sh + build):
//   node scripts/shot-tabs-pdf.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5205;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
const wait = async (u, t = 40) => { for (let i = 0; i < t; i++) { try { const r = await fetch(u); if (r.ok) return; } catch {} await sleep(250); } throw new Error("no server"); };

try {
  await wait(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });

  // --- Tabs: middle-click two distinct page-refs → background tabs, pin one --
  {
    const page = await browser.newPage({ viewport: { width: 1200, height: 760 }, deviceScaleFactor: 2 });
    try {
      await page.goto(`http://localhost:${PORT}/`);
      await page.waitForSelector(".page-title", { timeout: 8000 });
      await sleep(400);
      // open a few background tabs via distinct page-ref links in the feed
      const refs = page.locator("a.page-ref");
      const seen = new Set();
      const n = await refs.count();
      let opened = 0;
      for (let i = 0; i < n && opened < 3; i++) {
        const t = ((await refs.nth(i).textContent()) || "").trim();
        if (seen.has(t) || !t) continue;
        seen.add(t);
        await refs.nth(i).click({ button: "middle" });
        opened++;
        await sleep(250);
      }
      // pin TWO of them (double-click → sticky, sorts left)
      for (const name of ["Tine", "block editor"]) {
        const tab = page.locator(".tab", { hasText: name }).first();
        if (await tab.count()) { await tab.dblclick(); await sleep(250); }
      }
      // activate Tine (richer page content than block editor)
      const lc = page.locator(".tab", { hasText: "Tine" }).first();
      if (await lc.count()) { await lc.click(); await sleep(400); }
      // The pin indicator is a 📌 emoji — fine in the app (WebKitGTK has an emoji
      // font) but a tofu box in headless Chromium here, so swap in an inline SVG
      // pin purely for the screenshot (no product change). Do this AFTER the last
      // click, since re-rendering the tab bar would restore the emoji spans.
      await page.$$eval(".tab-pin", (els) => {
        for (const e of els) {
          e.textContent = "";
          e.innerHTML =
            '<svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 17v5"/><path d="M9 10.8a2 2 0 0 1-1.1 1.8l-1.8.9A2 2 0 0 0 5 15.3V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.7a2 2 0 0 0-1.1-1.8l-1.8-.9A2 2 0 0 1 15 10.8V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/></svg>';
        }
      });
      await sleep(300);
      const tabs = page.locator(".tab");
      await page.screenshot({ path: `${OUT}/feat-tabs.png` });
      console.log("OK    feat-tabs (tabs:", await tabs.count(), ")");
    } catch (e) { console.log("FAIL  feat-tabs", String(e).split("\n")[0]); }
    await page.close();
  }

  // --- PDF: open from Tine, text highlight + area highlight --------
  {
    const page = await browser.newPage({ viewport: { width: 1280, height: 820 }, deviceScaleFactor: 2 });
    try {
      await page.goto(`http://localhost:${PORT}/`);
      await page.waitForSelector(".page-title", { timeout: 8000 });
      await sleep(400);
      // navigate to Tine via its page-ref link in the feed
      await page.locator("a.page-ref", { hasText: "Tine" }).first().click();
      await sleep(500);
      // open the PDF (chip class .pdf-link, fallback: any link to a .pdf asset)
      let link = page.locator(".pdf-link");
      if (!(await link.count())) link = page.locator('a[href$=".pdf"], a:has-text("sample.pdf")');
      await link.first().click();
      await page.waitForSelector(".pdf-page canvas", { timeout: 8000 }).catch(() => {});
      await sleep(1600);
      // text highlight: triple-click a line, pick a color
      const span = page.locator(".pdf-page .textLayer span").first();
      if (await span.count()) {
        await span.click({ clickCount: 3 });
        await sleep(300);
        const sw = page.locator(".pdf-color-swatch").first();
        if (await sw.count()) { await sw.click(); await sleep(300); }
      }
      // area highlight: Ctrl-drag a rectangle on the page
      const box = await page.locator(".pdf-page").first().boundingBox();
      if (box) {
        const x0 = box.x + box.width * 0.16, y0 = box.y + box.height * 0.36;
        const x1 = box.x + box.width * 0.72, y1 = box.y + box.height * 0.55;
        await page.keyboard.down("Control");
        await page.mouse.move(x0, y0);
        await page.mouse.down();
        await page.mouse.move((x0 + x1) / 2, (y0 + y1) / 2, { steps: 6 });
        await page.mouse.move(x1, y1, { steps: 8 });
        await page.mouse.up();
        await page.keyboard.up("Control");
      }
      await sleep(700);
      await page.screenshot({ path: `${OUT}/feat-pdf.png` });
      console.log("OK    feat-pdf");
    } catch (e) { console.log("FAIL  feat-pdf", String(e).split("\n")[0]); }
    await page.close();
  }

  await browser.close();
  console.log("done");
} finally {
  server.kill("SIGTERM");
}
