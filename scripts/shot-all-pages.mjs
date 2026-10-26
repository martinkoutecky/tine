// All pages layout + native-control check (GH #335, GH #681; catalog UI-ALL-PAGES-GRID-001).
//
// Headless Chromium over the mock backend, the full view opened from the sidebar's
// "All pages" entry. For each of {journals off, on} x {viewport wide, narrow} x {palettes}:
//   * the header and every row have as many grid tracks as rendered cells, the header cells
//     share one y, and every cell of a row lies inside that row's box (a cell without a track
//     wraps into an implicit second grid row, onto the next row's first letter);
//   * the journal-date column exists only while "Show journals" is on;
//   * the "Show journals" checkbox is not a white box on a dark surface, and a dark mode keeps
//     `color-scheme: dark` under every bundled theme.
// Exits non-zero on the first failed assertion. Screenshots go to screenshots/all-pages-*.png.
//
// Usage (after `source scripts/env.sh && npm run build`):
//   node scripts/shot-all-pages.mjs
// SHOT_DIST=<dir> previews another build (used to prove the checks fail on pre-fix code).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5211;
const OUT = "screenshots";
const DIST = process.env.SHOT_DIST;
mkdirSync(OUT, { recursive: true });
// Run vite directly (not through npx) so killing the child really stops the server.
const server = spawn(
  process.execPath,
  ["node_modules/vite/bin/vite.js", "preview", "--port", String(PORT), "--strictPort", ...(DIST ? ["--outDir", DIST] : [])],
  { stdio: "ignore" },
);
const wait = async (u, t = 60) => { for (let i = 0; i < t; i++) { try { const r = await fetch(u); if (r.ok) return; } catch {} await sleep(250); } throw new Error("no server"); };

const themeCss = (id) => readFileSync(`src/styles/themes/${id}.css`, "utf8");
// A dark palette under the LIGHT mode attribute: what a user-installed or custom-CSS dark theme
// looks like to the engine, with no `data-theme="dark"` to key `color-scheme` on.
const DARK_PALETTE_IN_LIGHT_MODE = `html[data-theme="light"] {
  --bg-primary: #1a1b1e; --bg-secondary: #25262b; --bg-tertiary: #2f3035; --bg-quaternary: #3a3b40;
  --text-primary: #ecedf0; --text-title: #ecedf0; --text-secondary: #a0a2a8; --text-muted: #7a7c82;
  --border-color: #34363c; --accent: #6cb0ff; --link-color: #6cb0ff;
}`;
const PALETTES = [
  { name: "light", mode: "light", css: "", dark: false },
  { name: "dark", mode: "dark", css: "", dark: true },
  { name: "dark-palette-in-light-mode", mode: "light", css: DARK_PALETTE_IN_LIGHT_MODE, dark: true, colorSchemeFollowsMode: true },
  ...["nord", "solarized", "gruvbox", "soft"].map((id) => ({ name: `${id}-dark`, mode: "dark", css: themeCss(id), dark: true })),
];
const WIDTHS = [{ name: "wide", width: 1280 }, { name: "narrow", width: 420 }];

const failures = [];
const check = (ok, message) => { if (!ok) { failures.push(message); console.log("FAIL  " + message); } };

async function layout(page) {
  return page.evaluate(() => {
    const box = (el) => { const r = el.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height, r: r.right, b: r.bottom }; };
    const visible = (el) => getComputedStyle(el).display !== "none";
    const tracks = (el) => getComputedStyle(el).gridTemplateColumns.split(" ").length;
    const head = document.querySelector(".all-pages-head");
    const headCells = [...head.children].filter(visible);
    const rows = [...document.querySelectorAll(".all-pages-row")].map((row) => ({
      box: box(row), tracks: tracks(row),
      cells: [...row.children].filter(visible).map((c) => ({ cls: c.className, box: box(c) })),
    }));
    const checkbox = document.querySelector(".all-pages-journals input");
    return {
      head: { tracks: tracks(head), cells: headCells.map((c) => ({ cls: c.className, box: box(c) })) },
      rows,
      dayCols: document.querySelectorAll(".all-pages-day-col").length,
      dayCells: document.querySelectorAll(".all-pages-row .all-pages-day").length,
      checkboxScheme: checkbox ? getComputedStyle(checkbox).colorScheme : null,
      checkbox: checkbox ? box(checkbox) : null,
    };
  });
}

/** Mean RGB of a clip of the page, decoded in the page itself (no PNG library needed). */
async function meanColor(page, clip) {
  const png = await page.screenshot({ clip });
  return page.evaluate(async (b64) => {
    const img = new Image();
    img.src = `data:image/png;base64,${b64}`;
    await img.decode();
    const canvas = document.createElement("canvas");
    canvas.width = img.width; canvas.height = img.height;
    const ctx = canvas.getContext("2d");
    ctx.drawImage(img, 0, 0);
    const d = ctx.getImageData(0, 0, img.width, img.height).data;
    let r = 0, g = 0, b = 0;
    const n = d.length / 4;
    for (let i = 0; i < d.length; i += 4) { r += d[i]; g += d[i + 1]; b += d[i + 2]; }
    return [r / n, g / n, b / n];
  }, png.toString("base64"));
}
const lum = ([r, g, b]) => 0.2126 * r + 0.7152 * g + 0.0722 * b;

try {
  await wait(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  for (const palette of PALETTES) {
    for (const size of WIDTHS) {
      const context = await browser.newContext({ viewport: { width: size.width, height: 800 }, deviceScaleFactor: 1 });
      await context.addInitScript((mode) => { try { localStorage.setItem("logseq-claude.theme", mode); } catch {} }, palette.mode);
      const page = await context.newPage();
      page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
      await page.goto(`http://localhost:${PORT}/`);
      await page.waitForSelector(".page-title", { timeout: 8000 });
      await sleep(300);
      if (palette.css) await page.addStyleTag({ content: palette.css });
      // The sidebar's "All pages" header opens the full view (the section toggle only expands its list).
      // At narrow width the sidebar starts closed.
      if (!(await page.locator(".nav-all-pages-open").first().isVisible())) {
        await page.locator('button[title^="Toggle sidebar"]').first().click();
        await sleep(300);
      }
      await page.locator(".nav-all-pages-open").first().click();
      await page.waitForSelector(".all-pages-open", { timeout: 4000 });
      await sleep(200);

      for (const journals of [false, true]) {
        const box = page.locator(".all-pages-journals input");
        if ((await box.isChecked()) !== journals) await box.click();
        await sleep(250);
        const tag = `${palette.name}/${size.name}/journals-${journals ? "on" : "off"}`;
        const l = await layout(page);
        const cells = l.head.cells.length;
        const narrow = size.width <= 640;
        check(l.rows.length > 0, `${tag}: some rows are rendered`);
        // The journal-date column exists only while journals are listed (and never at narrow width).
        check(journals ? narrow || l.dayCols === 1 : l.dayCols === 0, `${tag}: journal-date header count is ${l.dayCols}`);
        check(journals ? narrow || l.dayCells === l.rows.length : l.dayCells === 0, `${tag}: journal-date cells ${l.dayCells} for ${l.rows.length} rows`);
        // One grid track per rendered cell, header and rows alike.
        check(l.head.tracks === cells, `${tag}: header has ${l.head.tracks} tracks for ${cells} cells`);
        for (const [i, row] of l.rows.entries()) {
          check(row.tracks === row.cells.length, `${tag}: row ${i} has ${row.tracks} tracks for ${row.cells.length} cells`);
          check(row.cells.length === cells, `${tag}: row ${i} has ${row.cells.length} cells, header ${cells}`);
          // Every cell lies inside its own row's box.
          for (const c of row.cells) {
            const inside = c.box.y >= row.box.y - 1 && c.box.b <= row.box.b + 1 && c.box.x >= row.box.x - 1 && c.box.r <= row.box.r + 1;
            check(inside, `${tag}: row ${i} cell .${c.cls} (y ${c.box.y.toFixed(0)}..${c.box.b.toFixed(0)}) is outside its row (y ${row.box.y.toFixed(0)}..${row.box.b.toFixed(0)})`);
          }
        }
        // The header cells share one line.
        const centers = l.head.cells.map((c) => c.box.y + c.box.h / 2);
        check(Math.max(...centers) - Math.min(...centers) <= 1, `${tag}: header cells sit on different lines (${centers.map((y) => y.toFixed(0)).join(", ")})`);

        if (palette.dark && !palette.colorSchemeFollowsMode) {
          check(l.checkboxScheme === "dark", `${tag}: color-scheme under a dark mode is ${l.checkboxScheme}`);
        }
        if (palette.dark) {
          // Unchecked "Show journals": the box interior must not be a white square on the dark surface.
          if (!journals) {
            const c = l.checkbox;
            const inner = await meanColor(page, { x: c.x + 4, y: c.y + 4, width: Math.max(1, c.w - 8), height: Math.max(1, c.h - 8) });
            check(lum(inner) < 110, `${tag}: checkbox interior is light (mean luminance ${lum(inner).toFixed(0)})`);
          }
        }
        if (journals) await page.screenshot({ path: `${OUT}/all-pages-${palette.name}-${size.name}.png` });
      }
      console.log(`done  ${palette.name}/${size.name}`);
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
console.log("OK    all-pages layout");
