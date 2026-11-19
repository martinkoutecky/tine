// GH #129 / bullet-threading #2: real display scaling, including a scrolled
// layout origin. Compare ink at the join with the same stroke beside the join.
// Usage: node scripts/shot-bullet-threading-dpr.mjs [artifactDir]
// THREAD_SWEEP=1 runs the full thickness and palette sweep in light mode.
import { chromium } from "playwright";
import { createServer } from "vite";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const out = resolve(process.argv[2] ?? "target/artifacts/fix-129-beta/dpr");
mkdirSync(out, { recursive: true });
const fixture = "scripts/fixtures/bullet-threading.html";
const server = await createServer({
  cacheDir: `${out}/vite-cache`,
  optimizeDeps: { entries: [fixture] },
  server: { port: 0, strictPort: false, hmr: false },
});
let browser;
const evidence = [];
const failures = [];
const sweep = process.env.THREAD_SWEEP === "1";
const zooms = process.env.THREAD_ZOOM ? [Number(process.env.THREAD_ZOOM)] : [1, 1.25];
const scales = process.env.THREAD_DPR ? [Number(process.env.THREAD_DPR)] : sweep ? [1, 1.25, 1.5, 1.75, 2] : [1, 1.25, 1.5, 1.75];
const themes = sweep ? ["light"] : ["light", "dark"];
const colors = sweep ? ["#e05070", "", "levels"] : [""];
const widths = sweep ? Array.from({ length: 21 }, (_, i) => i / 2) : [1, 2];
try {
  await server.listen();
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  for (const dpr of scales) for (const zoom of zooms) {
    const page = await browser.newPage({ viewport: { width: 800, height: 500 }, deviceScaleFactor: dpr });
    page.on("pageerror", error => console.error(error));
    // Worktree node_modules may be a symlink outside Vite's serving allowlist.
    await page.route("**/InterVariable.woff2", route => route.fulfill({
      body: readFileSync("node_modules/inter-ui/variable/InterVariable.woff2"), contentType: "font/woff2",
    }));
    await page.goto(`${server.resolvedUrls.local[0]}${fixture}`, { waitUntil: "commit" });
    await page.locator("main .ls-block").first().waitFor();
    if (process.env.THREAD_BASELINE_CSS) {
      await page.evaluate(css => {
        document.querySelector('style[data-vite-dev-id$="/src/styles/app.css"]').textContent = css;
      }, readFileSync(process.env.THREAD_BASELINE_CSS, "utf8"));
    }
    await page.evaluate(zoom => {
      document.documentElement.style.zoom = String(zoom);
      // A 1 CSS px scroll/placement change moves a join onto a half device px
      // at 150%. Integer-origin rows can conceal the defect completely.
      document.querySelector("main").style.paddingTop = "41px";
    }, zoom);
    await page.locator('[data-block-id="focus"] > .block-main .block-content').click();
    await page.locator("textarea.block-editor").waitFor();
    await page.evaluate(() => document.fonts.ready);
    for (const theme of themes) for (const color of colors) for (const display of ["active", "all"]) for (const thickness of widths) {
      await page.evaluate(({ theme, settings }) => {
        document.documentElement.dataset.theme = theme;
        window.setThreadSettings(settings);
      }, { theme, settings: { display, thickness, color: color === "levels" ? "" : color, "color-by-level": color === "levels" } });
      const name = `${dpr}-${zoom}-${theme}-${display}-${thickness}${color === "levels" ? "-levels" : color ? "-custom" : ""}`;
      const ids = sweep ? ["parent", "middle", "focus"] : display === "active"
        ? ["parent", "middle", "deep", "repeat", "focus"]
        : ["before", "parent", "middle", "deep", "repeat", "focus", "after", "last", "ordinary", "unrelated-child"];
      const geometry = await page.evaluate(({ zoom, ids }) => {
        const rows = ids.map(id => {
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
            focused: el.parentElement.matches(":focus-within"),
            continuationVisible: continuation.display !== "none" && continuation.content !== "none",
            top: box.top + zoom * parseFloat(style.top), bottom: box.top + zoom * (parseFloat(style.top) + parseFloat(style.height)),
            bulletY: bullet.top + bullet.height / 2, width: zoom * parseFloat(style.borderLeftWidth), display: style.display };
        });
        return { dpr: devicePixelRatio, rows };
      }, { zoom, ids });
      if (geometry.dpr !== dpr) throw new Error(`${name}: incorrect deviceScaleFactor`);
      for (const row of geometry.rows) {
        if (thickness === 0) {
          if (row.display !== "none") failures.push(`${name}/${row.id}: zero thickness is visible`);
        } else {
          if (Math.abs(row.x - row.parentX) * dpr > .05) failures.push(`${name}/${row.id}: child rail jogs ${(row.x - row.parentX) * dpr} device px from parent stub`);
          if (Math.abs(row.stubTop - row.parentBulletY) * dpr > .05) failures.push(`${name}/${row.id}: parent stub misses bullet`);
          if (row.top > row.sourceBottom) failures.push(`${name}/${row.id}: gap after source segment`);
          if (row.continuationVisible && row.radius > 0) failures.push(`${name}/${row.id}: straight continuation crosses rounded elbow`);
          if (!sweep && row.radius !== (row.continuationVisible ? 0 : 6)) failures.push(`${name}/${row.id}: wrong corner shape`);
          if (row.top > row.joinY) failures.push(`${name}/${row.id}: gap in connector geometry`);
          if (Math.abs(row.bottom - row.bulletY - zoom * thickness / 2) > .1) failures.push(`${name}/${row.id}: elbow misses bullet`);
        }
      }
      const shot = await page.screenshot({ path: `${out}/${name}.png` });
      const ink = thickness === 0 ? [] : await page.evaluate(async ({ src, rows, dpr, zoom }) => {
        const image = new Image(); image.src = src; await image.decode();
        const canvas = document.createElement("canvas"); canvas.width = image.width; canvas.height = image.height;
        const ctx = canvas.getContext("2d"); ctx.drawImage(image, 0, 0);
        const pixels = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
        return rows.map(row => {
          const left = Math.floor(row.x * dpr) - 2;
          const right = Math.ceil((row.x + row.width) * dpr) + 2;
          const contrast = (x, y) => {
            const background = 4 * (y * canvas.width + left - 2);
            const k = 4 * (y * canvas.width + x);
            return Math.abs(pixels[background] - pixels[k]) + Math.abs(pixels[background + 1] - pixels[k + 1]) + Math.abs(pixels[background + 2] - pixels[k + 2]);
          };
          const rail = y => Array.from({ length: right - left + 1 }, (_, i) => contrast(left + i, y));
          const mass = y => rail(y).reduce((total, pixel) => total + pixel, 0);
          const first = Math.floor((Math.min(row.joinY, row.top) - zoom) * dpr);
          const last = Math.ceil((row.joinY + zoom) * dpr);
          const branchY = Math.floor(row.bulletY * dpr);
          const beforeBranch = Math.floor((row.bulletY - 5 * zoom) * dpr);
          const fork = row.id === "ordinary" ? { peak: 0, arm: 0, rail: Math.max(...rail(first - 2)) } : null;
          if (fork) for (let y = branchY - Math.ceil(row.width * dpr); y <= branchY + Math.ceil(row.width * dpr); y++) {
            fork.peak = Math.max(fork.peak, ...rail(y));
            fork.arm = Math.max(fork.arm, contrast(right + 2, y));
          }
          return { id: row.id, first, above: mass(first - 2), below: mass(last + 2),
            join: Array.from({ length: last - first + 1 }, (_, i) => mass(first + i)), fork,
            activeRail: row.focused && row.radius === 0 ? mass(beforeBranch) : null };
        });
      }, { src: `data:image/png;base64,${shot.toString("base64")}`, rows: geometry.rows, dpr, zoom });
      for (const { id, above, below, join, fork, activeRail } of ink) {
        // A palette transition can have different ink on either side of a join.
        // Allow 15% beyond either straight stroke for channel quantization.
        if (!above || !below || Math.min(...join) < .85 * Math.min(above, below) || Math.max(...join) > 1.15 * Math.max(above, below)) {
          failures.push(`${name}/${id}: seam ink ${join.join(", ")} versus straight ink ${above}, ${below}`);
        }
        if (!color && activeRail !== null && activeRail < .85 * above) failures.push(`${name}/${id}: active rail fades before its branch (${activeRail} versus ${above})`);
        if (fork && (!fork.arm || fork.peak > 1.15 * Math.max(fork.rail, fork.arm))) {
          failures.push(`${name}/${id}: T junction ink ${fork.peak} versus rail ${fork.rail} and arm ${fork.arm}`);
        }
      }
      evidence.push({ name, geometry, ink });
    }
    await page.close();
  }
  writeFileSync(`${out}/observations.json`, JSON.stringify({ evidence, failures }, null, 2) + "\n");
  console.log(`${evidence.length} fractional-DPR screenshots; ${failures.length} failures`);
  if (failures.length) throw new Error(failures.slice(0, 12).join("\n"));
} finally {
  await browser?.close();
  await server.close();
}
