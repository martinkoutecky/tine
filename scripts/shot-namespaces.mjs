// Namespaces view layout check (GH #335, D-20261101-L5; catalog UI-NAMESPACES-FULL-VIEW).
//
// Headless Chromium over the mock backend (its Formula1 namespace tree), the full view
// opened from the entry inside the sidebar's expanded Namespaces section. For each of
// {light, dark} x {viewport wide, narrow}:
//   * the sidebar keeps its inline tree after the view opens (wide);
//   * the header and every row have two grid tracks and every cell lies inside its row;
//   * Collapse all leaves the top-level rows, Expand all shows the deepest level, and
//     the size column is right-aligned and non-empty for a namespace;
//   * a filter lists each match with the path above its name (no toggles);
//   * the filter box and buttons stay inside the viewport (no horizontal scroll).
// Exits non-zero on the first failed check. Screenshots go to screenshots/namespaces-*.png.
//
// Usage (after `source scripts/env.sh && npm run build`):
//   node scripts/shot-namespaces.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5212;
const OUT = "screenshots";
const DIST = process.env.SHOT_DIST;
mkdirSync(OUT, { recursive: true });
const server = spawn(
  process.execPath,
  ["node_modules/vite/bin/vite.js", "preview", "--port", String(PORT), "--strictPort", ...(DIST ? ["--outDir", DIST] : [])],
  { stdio: "ignore" },
);
const wait = async (u, t = 60) => { for (let i = 0; i < t; i++) { try { const r = await fetch(u); if (r.ok) return; } catch {} await sleep(250); } throw new Error("no server"); };
const failures = [];
const check = (ok, message) => { if (!ok) { failures.push(message); console.log("FAIL  " + message); } };

const labels = (page) => page.evaluate(() => [...document.querySelectorAll(".ns-view-open")].map((el) => el.title));
const geometry = (page) => page.evaluate(() => {
  const box = (el) => { const r = el.getBoundingClientRect(); return { x: r.left, y: r.top, r: r.right, b: r.bottom, w: r.width }; };
  const tracks = (el) => getComputedStyle(el).gridTemplateColumns.split(" ").length;
  const head = document.querySelector(".ns-view .all-pages-head");
  const rows = [...document.querySelectorAll(".ns-view-row")].map((row) => ({
    box: box(row), tracks: tracks(row), cells: [...row.children].map((c) => box(c)),
  }));
  const count = document.querySelector(".ns-view-row .ns-view-count");
  const view = document.querySelector(".ns-view");
  return {
    headTracks: tracks(head), rows,
    countAlign: count ? getComputedStyle(count).textAlign : null,
    overflowX: document.documentElement.scrollWidth - document.documentElement.clientWidth,
    controls: [...document.querySelectorAll(".ns-view .all-pages-controls > *")].map(box),
    viewBox: box(view),
    innerWidth: window.innerWidth,
  };
});

try {
  await wait(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  for (const mode of ["light", "dark"]) {
    for (const size of [{ name: "wide", width: 1280 }, { name: "narrow", width: 420 }]) {
      const tag = `${mode}/${size.name}`;
      const context = await browser.newContext({ viewport: { width: size.width, height: 800 }, deviceScaleFactor: 1 });
      await context.addInitScript((m) => { try { localStorage.setItem("logseq-claude.theme", m); } catch {} }, mode);
      const page = await context.newPage();
      page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
      await page.goto(`http://localhost:${PORT}/`);
      await page.waitForSelector(".page-title", { timeout: 8000 });
      await sleep(300);
      if (!(await page.locator(".nav-all-pages-open").first().isVisible())) {
        await page.locator('button[title^="Toggle sidebar"]').first().click();
        await sleep(300);
      }
      await page.locator(".nav-section-header", { hasText: "NAMESPACES" }).first().click();
      await page.waitForSelector(".nav-namespaces-open", { timeout: 4000 });
      check(await page.locator(".ns-tree").count() === 1, `${tag}: the sidebar keeps its inline tree`);
      await page.locator(".nav-namespaces-open").first().click();
      await page.waitForSelector(".ns-view-open", { timeout: 4000 });
      await sleep(200);

      let l = await geometry(page);
      check(l.rows.length > 0, `${tag}: rows rendered`);
      check(l.headTracks === 2, `${tag}: header has ${l.headTracks} tracks`);
      for (const [i, row] of l.rows.entries()) {
        check(row.tracks === 2 && row.cells.length === 2, `${tag}: row ${i} has ${row.tracks} tracks for ${row.cells.length} cells`);
        for (const c of row.cells) check(c.y >= row.box.y - 1 && c.b <= row.box.b + 1 && c.x >= row.box.x - 1 && c.r <= row.box.r + 1, `${tag}: row ${i} cell outside its row`);
      }
      check(l.overflowX <= 0, `${tag}: page scrolls horizontally by ${l.overflowX}px`);
      check(l.controls.every((c) => c.r <= l.innerWidth + 1), `${tag}: a control runs past the viewport`);
      check(l.countAlign === "right" || l.countAlign === "end", `${tag}: size column text-align is ${l.countAlign}`);

      await page.locator(".ns-view-action", { hasText: "Collapse all" }).click();
      await sleep(150);
      const collapsed = await labels(page);
      check(collapsed.length >= 1 && collapsed.every((n) => !n.includes("/")), `${tag}: collapse all leaves top-level rows (${collapsed.join(", ")})`);
      await page.locator(".ns-view-action", { hasText: "Expand all" }).click();
      await sleep(150);
      const expanded = await labels(page);
      check(expanded.some((n) => n.split("/").length >= 3), `${tag}: expand all reaches the deepest level (${expanded.join(", ")})`);
      const formulaSize = await page.evaluate(() => [...document.querySelectorAll(".ns-view-row")].find((r) => r.querySelector(".ns-view-open")?.title === "Formula1")?.querySelector(".ns-view-count")?.textContent);
      check(Boolean(formulaSize) && Number(formulaSize) > 0, `${tag}: Formula1 shows its size (${formulaSize})`);
      await page.screenshot({ path: `${OUT}/namespaces-tree-${mode}-${size.name}.png` });

      await page.fill(".all-pages-filter", "grand prix");
      await sleep(200);
      const hits = await labels(page);
      check(hits.length >= 3 && hits.every((n) => n.toLowerCase().includes("grand prix")), `${tag}: filter lists ${hits.length} full paths`);
      const prefixes = await page.evaluate(() => [...document.querySelectorAll(".ns-view-prefix")].map((el) => el.textContent));
      check(prefixes.length === hits.length && prefixes.every((p) => p.endsWith("/")), `${tag}: every match shows its path above the name`);
      check(await page.locator(".ns-view-toggle").count() === 0, `${tag}: no tree toggles while filtering`);
      await page.screenshot({ path: `${OUT}/namespaces-filter-${mode}-${size.name}.png` });
      console.log(`done  ${tag}`);
      await context.close();
    }
  }
  await browser.close();
} finally {
  server.kill("SIGTERM");
}
if (failures.length) {
  console.log(`\n${failures.length} check(s) failed`);
  process.exit(1);
}
console.log("OK    namespaces layout");
