// README feature-gallery screenshots: tabs, dim-inactive-blocks, carry, queries
// + query builder, and PDF highlights (text + area). Headless Chromium over the
// mock backend. Each shot is isolated so one failure doesn't abort the rest.
// Usage (after `source scripts/env.sh && npm run build`):
//   node scripts/shot-features.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5203;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });

const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try { const r = await fetch(url); if (r.ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

const shot = async (page, name, fn, opts = {}) => {
  try {
    await fn();
    await page.screenshot({ path: `${OUT}/${name}.png`, ...opts });
    console.log("OK   ", name);
  } catch (e) {
    console.log("FAIL ", name, String(e).split("\n")[0]);
  }
};

const reset = async (page) => {
  await page.keyboard.press("Escape").catch(() => {});
  await page.keyboard.press("Escape").catch(() => {});
  // back to Journals (first tab)
  const t = page.locator(".tab").first();
  if (await t.count()) await t.click().catch(() => {});
  await sleep(200);
};

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1200, height: 820 }, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(500);

  // expand the sidebar "ALL PAGES" section so .nav-page rows exist (the first
  // .nav-item is the section toggle, mirroring scripts/screenshot.mjs)
  const expandPages = async () => {
    await page.locator(".nav-item").first().click().catch(() => {});
    await sleep(250);
  };

  // --- 1. Tabs: background tabs + one pinned ------------------------------
  await shot(page, "feat-tabs", async () => {
    await expandPages();
    const np = page.locator(".nav-page");
    const count = await np.count();
    if (count) await np.nth(0).click({ button: "middle" }); // logseq-claude → bg tab
    await sleep(200);
    if (count > 1) await np.nth(1).click({ button: "middle" }); // kitchen-sink → bg tab
    await sleep(200);
    const tabs = page.locator(".tab");
    if ((await tabs.count()) > 1) await tabs.nth(1).dblclick(); // pin → sticky, sorts left
    await sleep(300);
  });

  // --- 2. Dim inactive blocks ---------------------------------------------
  await reset(page);
  await shot(page, "feat-dim", async () => {
    await page.keyboard.press("t");
    await page.keyboard.press("b"); // toggle dim-inactive-blocks
    await sleep(150);
    const blocks = page.locator(".block-content-wrapper");
    if (await blocks.count()) await blocks.nth(3).click(); // spotlight one
    await sleep(300);
  });
  // turn dim back off
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await page.keyboard.press("t");
  await page.keyboard.press("b");
  await sleep(150);

  // --- 3. Carry unfinished tasks ------------------------------------------
  await reset(page);
  await shot(page, "feat-carry", async () => {
    const btn = page.locator(".carry-btn").first();
    if (await btn.count()) await btn.hover();
    await sleep(250);
  }, { clip: { x: 470, y: 70, width: 730, height: 430 } });

  // --- 4a. Queries (rendered results) -------------------------------------
  await reset(page);
  await shot(page, "feat-query", async () => {
    const q = page.locator(".query-block, .macro-query, .query-result, .ref-block").first();
    if (await q.count()) await q.scrollIntoViewIfNeeded();
    await sleep(300);
  });

  // --- 4b. Query builder (chip bar) ---------------------------------------
  await reset(page);
  await shot(page, "feat-querybuilder", async () => {
    // edit an empty trailing block, insert via the slash command
    const blocks = page.locator(".block-content-wrapper");
    const n = await blocks.count();
    if (n) await blocks.nth(n - 1).click();
    await sleep(150);
    await page.keyboard.type("/query (visual");
    await sleep(300);
    const item = page.locator(".ac-item").first();
    if (await item.count()) await item.click();
    await sleep(400);
    const bar = page.locator(".qb-bar").first();
    if (await bar.count()) await bar.scrollIntoViewIfNeeded();
    await sleep(300);
  });

  // --- 5. PDF: a text highlight + an area (image) highlight ----------------
  await reset(page);
  await shot(page, "feat-pdf", async () => {
    await expandPages();
    await page.locator(".nav-page").first().click(); // logseq-claude has the PDF
    await sleep(300);
    await page.locator(".pdf-link").first().click();
    await page.waitForSelector(".pdf-page canvas", { timeout: 8000 }).catch(() => {});
    await sleep(1500);
    // text highlight: triple-click a line to select it, pick a color swatch
    const span = page.locator(".pdf-page .textLayer span").first();
    if (await span.count()) {
      await span.click({ clickCount: 3 });
      await sleep(300);
      const sw = page.locator(".pdf-color-swatch").first();
      if (await sw.count()) { await sw.click(); await sleep(300); }
    }
    // area highlight: Ctrl-drag a rectangle on the page
    const pg = page.locator(".pdf-page").first();
    const box = await pg.boundingBox();
    if (box) {
      const x0 = box.x + box.width * 0.16, y0 = box.y + box.height * 0.34;
      const x1 = box.x + box.width * 0.70, y1 = box.y + box.height * 0.52;
      await page.keyboard.down("Control");
      await page.mouse.move(x0, y0);
      await page.mouse.down();
      await page.mouse.move((x0 + x1) / 2, (y0 + y1) / 2, { steps: 6 });
      await page.mouse.move(x1, y1, { steps: 6 });
      await page.mouse.up();
      await page.keyboard.up("Control");
    }
    await sleep(600);
  });

  await browser.close();
  console.log("done");
} finally {
  server.kill("SIGTERM");
}
