// Landing-site screenshots (website/img/). Headless Chromium over the mock
// backend, same as the README shots. Produces three images the site needs that
// the README set doesn't cover well:
//   web-dark.png        — journals feed in dark mode (theme toggle)
//   web-focus-dim.png   — focus mode WITH dim-inactive-blocks actually showing
//   web-capture.png     — quick-capture window incl. the new page-title field
//
// Note on dim: `dim-mode` only renders while a block is being edited
// (App.tsx: `dimInactiveBlocks() && editingId()`), and entering focus mode
// auto-enables dim — so we must NOT toggle `t b` again, and we must click a
// block into edit mode to light up the spotlight.
//
// Usage:  source scripts/env.sh && npm run build && node scripts/shot-website.mjs
//         then copy the chosen files into website/img/ (see docs/SCREENSHOTS.md).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5198;
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

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });

  // --- 1. Dark mode (full chrome journals feed) --------------------------
  {
    const ctx = await browser.newContext({ viewport: { width: 1180, height: 820 }, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
    await page.goto(`http://localhost:${PORT}/`);
    await page.waitForSelector(".page-title", { timeout: 8000 });
    await sleep(400);
    for (const c of await page.locator(".toast-close").all()) await c.click().catch(() => {});
    await page.click('[title^="Toggle theme"]').catch(() => {});
    await sleep(500);
    for (const c of await page.locator(".toast-close").all()) await c.click().catch(() => {});
    await sleep(150);
    await page.screenshot({ path: `${OUT}/web-dark.png` });
    console.log("OK    web-dark");
    await ctx.close();
  }

  // --- 2. Focus mode + dim (one block spotlit) ---------------------------
  {
    const ctx = await browser.newContext({ viewport: { width: 1180, height: 860 }, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
    await page.goto(`http://localhost:${PORT}/`);
    await page.waitForSelector(".page-title", { timeout: 8000 });
    await sleep(400);
    for (const c of await page.locator(".toast-close").all()) await c.click().catch(() => {});
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");
    await sleep(100);
    await page.keyboard.press("t");
    await page.keyboard.press("f"); // focus mode → auto-enables dim
    await sleep(250);
    // Edit a substantive prose block so it's spotlit amid the dimmed surface.
    const target = page.getByText("aiming for a", { exact: false }).first();
    if (await target.count()) await target.click();
    else {
      const b = page.locator(".block-content-wrapper");
      if (await b.count()) await b.nth(1).click();
    }
    await sleep(450);
    await page.screenshot({ path: `${OUT}/web-focus-dim.png` });
    console.log("OK    web-focus-dim");
    await ctx.close();
  }

  // --- 2b. Dim with chrome (README dim.png): dim on, sidebar/topbar visible -
  {
    const ctx = await browser.newContext({ viewport: { width: 1180, height: 820 }, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
    await page.goto(`http://localhost:${PORT}/`);
    await page.waitForSelector(".page-title", { timeout: 8000 });
    await sleep(400);
    for (const c of await page.locator(".toast-close").all()) await c.click().catch(() => {});
    await page.keyboard.press("t");
    await page.keyboard.press("b"); // dim inactive blocks (no focus → chrome stays)
    await sleep(200);
    const target = page.getByText("aiming for a", { exact: false }).first();
    if (await target.count()) await target.click();
    else {
      const b = page.locator(".block-content-wrapper");
      if (await b.count()) await b.nth(1).click();
    }
    await sleep(450);
    await page.screenshot({ path: `${OUT}/web-dim.png` });
    console.log("OK    web-dim");
    await ctx.close();
  }

  // --- 3. Quick-capture window (with the page-title field) ----------------
  {
    const ctx = await browser.newContext({ viewport: { width: 720, height: 480 }, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
    await page.goto(`http://localhost:${PORT}/capture.html`);
    await page.waitForSelector(".capture-shell textarea", { timeout: 8000 });
    await page.addStyleTag({
      content: `
        html, body { background: #e9eef5 !important; height: 100%; }
        #capture-root {
          width: 560px; margin: 56px auto 0; background: var(--bg-primary, #fff);
          border-radius: 12px; box-shadow: 0 18px 50px rgba(20,30,60,.28), 0 2px 6px rgba(20,30,60,.12);
          overflow: visible;
        }
      `,
    });
    // Empty state first — just the page-title hint and an empty bullet, so the
    // site can show the window before and after you type.
    await sleep(300);
    await page.screenshot({ path: `${OUT}/web-capture-empty.png`, clip: { x: 40, y: 28, width: 640, height: 176 } });
    console.log("OK    web-capture-empty");
    // The root bullet starts in edit mode — type the note first…
    const ta = page.locator(".capture-shell textarea").first();
    await ta.click();
    await page.keyboard.type("Gödel, Escher, Bach ", { delay: 8 });
    await page.keyboard.type("#books ", { delay: 8 });
    await sleep(150);
    // …then give it a page title via the new field at the top of the window.
    const title = page.locator(".capture-title");
    await title.click();
    await page.keyboard.type("Reading list", { delay: 8 });
    await sleep(350);
    // Clip to the floating window card (+ its shadow) so there's no dead space.
    await page.screenshot({ path: `${OUT}/web-capture.png`, clip: { x: 40, y: 28, width: 640, height: 248 } });
    console.log("OK    web-capture");
    await ctx.close();
  }

  await browser.close();
  console.log("done");
} finally {
  server.kill("SIGTERM");
}
