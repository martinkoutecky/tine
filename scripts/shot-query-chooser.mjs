// GH #619 class 2: the query sheet's field chooser (and every other sheet dropdown) must never squeeze its rows and
// must scroll, at 6 / 40 / 41 / 300 keys and at phone width. Headless Chromium over the mock backend + Vite dev server
// (dev, not preview, so the page can import the live backend singleton and give the mock a property registry).
// Usage: node scripts/shot-query-chooser.mjs   (writes screenshots/gh619-chooser-*.png, exits non-zero on a violation)
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const PORT = 5291;
const OUT = process.env.OUT ?? "screenshots";
fs.mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
const failures = [];
const check = (ok, what) => { console.log(ok ? "ok  " : "FAIL", what); if (!ok) failures.push(what); };

async function waitForServer(url, tries = 240) {
  for (let i = 0; i < tries; i++) {
    try { const r = await fetch(url); if (r.ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

const rowsFor = (keys) => Array.from({ length: keys }, (_, i) => ({
  normalized_name: `prop-key-${String(i).padStart(4, "0")}`, cardinality: "one", observed_type: "text",
  count_blocks: 3, count_pages: 1, mismatch_count: 0, top_values: [["alpha", 2], ["beta", 1]],
}));

const SIZES = [[1100, 800, "wide"], [480, 700, "phone"], [900, 420, "short"]];
const url = (keys) => `http://localhost:${PORT}/scripts/harness/query-sheet.html?keys=${keys}`;

/** Measure the open popover (`.qs-menu`) and its list: nothing squeezed, nothing off-screen. */
const measure = (page) => page.evaluate(() => {
  const menu = document.querySelector(".qs-menu");
  if (!menu) return null;
  const r = menu.getBoundingClientRect();
  const list = menu.querySelector(".qs-options");
  const rows = [...menu.querySelectorAll(".qs-vocab-row, .qs-option")];
  const squeezed = rows.filter((e) => e.scrollHeight > e.clientHeight + 1 || e.clientHeight < 20).length;
  const first = rows[0]?.getBoundingClientRect().height ?? 0;
  const inSheet = !!menu.closest(".qs-sheet");
  return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, vw: innerWidth, vh: innerHeight, nRows: rows.length,
    squeezed, first, listClient: list?.clientHeight ?? 0, listScroll: list?.scrollHeight ?? 0, menuScroll: menu.scrollHeight, menuClient: menu.clientHeight, inSheet,
    rowHeights: [...new Set(rows.slice(0, 40).map((e) => Math.round(e.getBoundingClientRect().height)))] };
});

async function wheelScrolls(page) {
  // The scrollable thing under the pointer: the list when it overflows, else the popover itself.
  const target = await page.evaluate(() => {
    const menu = document.querySelector(".qs-menu"); const list = menu?.querySelector(".qs-options");
    const el = list && list.scrollHeight > list.clientHeight + 1 ? list : menu && menu.scrollHeight > menu.clientHeight + 1 ? menu : null;
    if (!el) return null;
    const r = el.getBoundingClientRect();
    el.dataset.wheelProbe = "1";
    return { x: r.left + r.width / 2, y: r.top + Math.min(r.height / 2, 60) };
  });
  if (!target) return "not-scrollable";
  await page.mouse.move(target.x, target.y);
  await page.mouse.wheel(0, 240);
  await sleep(250);
  const top = await page.evaluate(() => document.querySelector("[data-wheel-probe]").scrollTop);
  return top > 0 ? "scrolled" : "stuck";
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  for (const [w, h, label] of SIZES) {
    for (const keys of [3, 20, 40, 41, 300]) {
      const page = await browser.newPage({ viewport: { width: w, height: h } });
      page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
      await page.goto(url(keys));
      await page.waitForSelector(".qs-gear");
      await page.click(".qs-gear");
      await page.waitForSelector(".qs-sheet");
      await page.click(".qs-sheet .qs-add");
      await page.waitForSelector(".qs-menu .qs-vocab-row");
      await sleep(500);
      const m = await measure(page);
      const tag = `chooser ${label} ${w}x${h} keys=${keys}`;
      check(m.squeezed === 0, `${tag}: no squeezed rows (${m.nRows} rows, heights ${m.rowHeights})`);
      check(m.first >= 40, `${tag}: first row >= 40px (${Math.round(m.first)})`);
      check(m.bottom <= m.vh + 1 && m.top >= -1 && m.right <= m.vw + 1 && m.left >= -1, `${tag}: popover inside the viewport (${[m.top, m.bottom, m.left, m.right].map(Math.round)} in ${m.vw}x${m.vh})`);
      const wheel = await wheelScrolls(page);
      check(wheel !== "stuck", `${tag}: wheel ${wheel}`);
      if (keys >= 20) check(wheel === "scrolled", `${tag}: a long list scrolls with the wheel`);
      // keyboard follow: the active row stays inside the visible list
      await page.focus(".qs-menu-filter"); // as a user who clicked the filter; the key handler lives on the popover
      for (let i = 0; i < 12; i++) await page.keyboard.press("ArrowDown");
      await sleep(250);
      const vis = await page.evaluate(() => {
        const list = document.querySelector(".qs-menu .qs-options"); const act = document.querySelector(".qs-menu .qs-option.active");
        if (!list || !act) return { ok: false, why: "no active row" };
        const a = act.getBoundingClientRect(), l = list.getBoundingClientRect();
        return { ok: a.top >= l.top - 1 && a.bottom <= l.bottom + 1, a: [a.top, a.bottom], l: [l.top, l.bottom] };
      });
      check(vis.ok, `${tag}: the active row follows the arrow keys into view ${JSON.stringify(vis)}`);
      if (keys === 41 || keys === 300 || keys === 3) await page.screenshot({ path: `${OUT}/gh619-chooser-${label}-${keys}.png` });
      await page.close();
    }
  }

  // Every other opener in the sheet (find, field, operator, value ...): the popover fits and nothing is squeezed.
  for (const [w, h, label] of SIZES) {
    const probe = await browser.newPage({ viewport: { width: w, height: h } });
    await probe.goto(url(41)); await probe.waitForSelector(".qs-gear"); await probe.click(".qs-gear"); await probe.waitForSelector(".qs-sheet");
    const count = await probe.evaluate(() => document.querySelectorAll(".qs-sheet button[aria-expanded]").length);
    await probe.close();
    for (let i = 0; i < count; i++) {
      const page = await browser.newPage({ viewport: { width: w, height: h } });
      await page.goto(url(41)); await page.waitForSelector(".qs-gear"); await page.click(".qs-gear"); await page.waitForSelector(".qs-sheet");
      const name = await page.evaluate((i) => { const b = document.querySelectorAll(".qs-sheet button[aria-expanded]")[i]; return (b.getAttribute("aria-label") || b.textContent || "").trim().slice(0, 24); }, i);
      await page.locator(".qs-sheet button[aria-expanded]").nth(i).click();
      await sleep(500);
      const m = await measure(page);
      const tag = `opener#${i} "${name}" ${label} ${w}x${h}`;
      if (!m) { console.log("note ", tag, "opens no .qs-menu"); await page.close(); continue; }
      check(m.squeezed === 0, `${tag}: no squeezed rows`);
      check(m.bottom <= m.vh + 1 && m.top >= -1 && m.right <= m.vw + 1 && m.left >= -1, `${tag}: popover inside the viewport (${[m.top, m.bottom, m.left, m.right].map(Math.round)} in ${m.vw}x${m.vh})`);
      const wheel = await wheelScrolls(page);
      check(wheel !== "stuck", `${tag}: wheel ${wheel}`);
      if (label === "short") await page.screenshot({ path: `${OUT}/gh619-opener-${i}-${label}.png` });
      await page.close();
    }
  }
  await browser.close();
} finally {
  server.kill();
}
if (failures.length) { console.log("FAILED:", failures.length); process.exit(1); }
console.log("DONE");
