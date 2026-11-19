// GH #129 / bullet-threading #2: real display scaling, including a scrolled
// layout origin. Compare ink at the join with the same stroke beside the join.
// Usage: node scripts/shot-bullet-threading-dpr.mjs [artifactDir]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const out = resolve(process.argv[2] ?? "target/artifacts/fix-129-beta/dpr");
mkdirSync(out, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
const evidence = [];
const failures = [];
const zoom = Number(process.env.THREAD_ZOOM ?? 1);
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (attempt === 100) throw new Error("Vite did not start");
    await delay(200);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  for (const dpr of process.env.THREAD_DPR ? [Number(process.env.THREAD_DPR)] : [1, 1.25, 1.5, 1.75, 2]) {
    const page = await browser.newPage({ viewport: { width: 800, height: 500 }, deviceScaleFactor: dpr });
    // Worktree node_modules may be a symlink outside Vite's serving allowlist.
    await page.route("**/InterVariable.woff2", route => route.fulfill({
      body: readFileSync("node_modules/inter-ui/variable/InterVariable.woff2"), contentType: "font/woff2",
    }));
    await page.goto(`http://localhost:${port}/scripts/fixtures/bullet-threading.html`);
    await page.locator("main .ls-block").first().waitFor();
    if (process.env.THREAD_BASELINE_CSS) {
      await page.evaluate(css => {
        document.querySelector('style[data-vite-dev-id$="/src/styles/app.css"]').textContent = css;
      }, readFileSync(process.env.THREAD_BASELINE_CSS, "utf8"));
    }
    await page.evaluate(zoom => {
      document.documentElement.dataset.theme = "light";
      document.documentElement.style.zoom = String(zoom);
      // A 1 CSS px scroll/placement change moves a join onto a half device px
      // at 150%. Integer-origin rows can conceal the defect completely.
      document.querySelector("main").style.paddingTop = "41px";
    }, zoom);
    await page.locator('[data-block-id="focus"] > .block-main .block-content').click();
    await page.locator("textarea.block-editor").waitFor();
    await page.evaluate(() => document.fonts.ready);
    for (const color of ["#e05070", "", "levels"]) for (const display of ["active", "all"]) for (const thickness of Array.from({ length: 21 }, (_, i) => i / 2)) {
      await page.evaluate(settings => window.setThreadSettings(settings), { display, thickness, color: color === "levels" ? "" : color, "color-by-level": color === "levels" });
      const name = `${dpr}-${display}-${thickness}${color === "levels" ? "-levels" : color ? "" : "-theme"}`;
      const geometry = await page.evaluate(zoom => {
        const rows = ["parent", "middle", "focus"].map(id => {
          const el = document.querySelector(`[data-block-id="${id}"] > .block-main`);
          const box = el.getBoundingClientRect();
          const style = getComputedStyle(el, "::before");
          const bullet = el.querySelector(".bullet").getBoundingClientRect();
          const parent = el.closest(".block-children-container").parentElement.querySelector(":scope > .block-main");
          const stub = getComputedStyle(parent, "::after");
          const continuation = getComputedStyle(el.parentElement, "::after");
          const parentBullet = parent.querySelector(".bullet").getBoundingClientRect();
          const previous = el.parentElement.previousElementSibling;
          return { id, joinY: box.top, x: box.left + zoom * parseFloat(style.left),
            parentX: parent.getBoundingClientRect().left + zoom * parseFloat(stub.left),
            stubTop: parent.getBoundingClientRect().top + zoom * parseFloat(stub.top),
            parentBulletY: parentBullet.top + parentBullet.height / 2,
            sourceBottom: previous?.matches(".ls-block") ? previous.getBoundingClientRect().bottom : parent.getBoundingClientRect().bottom,
            radius: parseFloat(style.borderBottomLeftRadius),
            continuationVisible: continuation.display !== "none" && continuation.content !== "none",
            top: box.top + zoom * parseFloat(style.top), bottom: box.top + zoom * (parseFloat(style.top) + parseFloat(style.height)),
            bulletY: bullet.top + bullet.height / 2, width: zoom * parseFloat(style.borderLeftWidth), display: style.display };
        });
        return { dpr: devicePixelRatio, rows };
      }, zoom);
      if (geometry.dpr !== dpr) throw new Error(`${name}: incorrect deviceScaleFactor`);
      for (const row of geometry.rows) {
        if (thickness === 0) {
          if (row.display !== "none") failures.push(`${name}/${row.id}: zero thickness is visible`);
        } else {
          if (Math.abs(row.x - row.parentX) * dpr > .05) failures.push(`${name}/${row.id}: child rail jogs ${(row.x - row.parentX) * dpr} device px from parent stub`);
          if (Math.abs(row.stubTop - row.parentBulletY) * dpr > .05) failures.push(`${name}/${row.id}: parent stub misses bullet`);
          if (row.top > row.sourceBottom) failures.push(`${name}/${row.id}: gap after source segment`);
          if (row.continuationVisible && row.radius > 0) failures.push(`${name}/${row.id}: straight continuation crosses rounded elbow`);
          if (row.top > row.joinY) failures.push(`${name}/${row.id}: gap in connector geometry`);
          if (Math.abs(row.bottom - row.bulletY - zoom * thickness / 2) > .1) failures.push(`${name}/${row.id}: elbow misses bullet`);
        }
      }
      const shot = await page.screenshot({ path: `${out}/${name}.png` });
      const ink = thickness === 0 ? [] : await page.evaluate(async ({ src, rows, dpr }) => {
        const image = new Image(); image.src = src; await image.decode();
        const canvas = document.createElement("canvas"); canvas.width = image.width; canvas.height = image.height;
        const ctx = canvas.getContext("2d"); ctx.drawImage(image, 0, 0);
        const pixels = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
        return rows.map(row => {
          const mass = y => {
            let total = 0;
            for (let x = Math.floor(row.x * dpr) - 2; x <= Math.ceil((row.x + row.width) * dpr) + 2; x++) {
              const k = 4 * (y * canvas.width + x);
              // Stroke on white: green-channel ink is linear in coverage.
              total += (255 - pixels[k + 1]) / (255 - 80);
            }
            return total;
          };
          const y = Math.floor(row.joinY * dpr);
          const adjacent = (mass(y - 3) + mass(y + 3)) / 2;
          return { id: row.id, adjacent, join: mass(y), ratio: mass(y) / adjacent };
        });
      }, { src: `data:image/png;base64,${shot.toString("base64")}`, rows: geometry.rows, dpr });
      for (const row of ink) if (row.ratio < .85) failures.push(`${name}/${row.id}: screenshot seam retains ${(100 * row.ratio).toFixed(1)}% of adjacent stroke ink`);
      evidence.push({ name, geometry, ink });
    }
    await page.close();
  }
  writeFileSync(`${out}/observations.json`, JSON.stringify({ evidence, failures }, null, 2) + "\n");
  console.log(`${evidence.length} fractional-DPR screenshots; ${failures.length} failures`);
  if (failures.length) throw new Error(failures.slice(0, 12).join("\n"));
} finally {
  await browser?.close();
  process.kill(-server.pid, "SIGTERM");
}
