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
      const refs = page.locator("a.page-ref");
      const seen = new Set();
      const n = await refs.count();
      let opened = 0;
      for (let i = 0; i < n && opened < 2; i++) {
        const t = ((await refs.nth(i).textContent()) || "").trim();
        if (seen.has(t)) continue;
        seen.add(t);
        await refs.nth(i).click({ button: "middle" });
        opened++;
        await sleep(250);
      }
      const tabs = page.locator(".tab");
      if ((await tabs.count()) > 1) await tabs.nth(1).dblclick(); // pin → sticky, sorts left
      await sleep(300);
      await page.screenshot({ path: `${OUT}/feat-tabs.png` });
      console.log("OK    feat-tabs (tabs:", await tabs.count(), ")");
    } catch (e) { console.log("FAIL  feat-tabs", String(e).split("\n")[0]); }
    await page.close();
  }

  // --- PDF: open from logseq-claude, text highlight + area highlight --------
  {
    const page = await browser.newPage({ viewport: { width: 1280, height: 820 }, deviceScaleFactor: 2 });
    try {
      await page.goto(`http://localhost:${PORT}/`);
      await page.waitForSelector(".page-title", { timeout: 8000 });
      await sleep(400);
      // navigate to logseq-claude via its page-ref link in the feed
      await page.locator("a.page-ref", { hasText: "logseq-claude" }).first().click();
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
