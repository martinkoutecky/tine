// GH #129: ordinary guides, elbow geometry and palette on real Block rows.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const out = resolve(process.argv[2] ?? "target/artifacts/fix-129-guides/after");
mkdirSync(out, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
const evidence = [], failures = [];
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (attempt === 100) throw new Error("Vite did not start");
    await delay(200);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  for (const dpr of [1, 1.5]) {
    const page = await browser.newPage({ viewport: { width: 1000, height: 700 }, deviceScaleFactor: dpr });
    await page.route("**/InterVariable.woff2", route => route.fulfill({ body: readFileSync("node_modules/inter-ui/variable/InterVariable.woff2"), contentType: "font/woff2" }));
    await page.goto(`http://localhost:${port}/scripts/fixtures/bullet-threading.html`);
    if (process.env.THREAD_BASELINE_CSS) {
      await page.evaluate(css => {
        document.querySelector('style[data-vite-dev-id$="/src/styles/app.css"]').textContent = css;
      }, readFileSync(process.env.THREAD_BASELINE_CSS, "utf8"));
    }
    await page.locator('[data-block-id="focus"] > .block-main .block-content').click();
    await page.locator("textarea.block-editor").waitFor();
    await page.evaluate(() => document.fonts.ready);
    for (const theme of ["light", "dark"]) for (const display of ["active", "all"]) {
      const name = `${theme}-${display}-${dpr}`;
      await page.evaluate(({ theme, display }) => {
        document.documentElement.dataset.theme = theme;
        window.setThreadSettings({ display, thickness: 2, "color-by-level": true });
      }, { theme, display });
      const facts = await page.evaluate(() => {
        const row = id => document.querySelector(`[data-block-id="${id}"]`);
        const guideStyle = el => parseFloat(getComputedStyle(el).borderLeftWidth) > 0 ? getComputedStyle(el) : getComputedStyle(el, "::before");
        const guide = guideStyle(row("last").querySelector(":scope > .block-children-container > .block-children"));
        const probe = document.createElement("span");
        probe.style.color = "var(--guide-color)";
        row("last").append(probe);
        const ordinary = getComputedStyle(probe).color;
        probe.remove();
        const colors = ["root", "parent", "middle", "deep", "repeat"].map(id => {
          const el = row(id).querySelector(":scope > .block-main");
          return getComputedStyle(el, "::after").borderLeftColor;
        });
        const last = row("last"), line = getComputedStyle(last, "::after");
        const box = last.getBoundingClientRect();
        const elbow = last.querySelector(":scope > .block-main");
        const curve = getComputedStyle(elbow, "::before");
        return { dpr: devicePixelRatio, guide: guide.borderLeftColor, ordinary, colors,
          guideWidth: parseFloat(guide.borderLeftWidth),
          parentsDrawn: [...document.querySelectorAll(".plugin-thread-lines:not(.collapsed) > .block-main")].every(el => getComputedStyle(el, "::after").display !== "none"),
          ancestryGuide: guideStyle(row("root").querySelector(":scope > .block-children-container > .block-children")).borderLeftColor,
          straightVisible: line.display !== "none" && line.content !== "none" && line.borderLeftColor !== "rgba(0, 0, 0, 0)",
          straightBottom: box.bottom - parseFloat(line.bottom), elbowBottom: elbow.getBoundingClientRect().top + parseFloat(curve.top) + parseFloat(curve.height) };
      });
      await page.screenshot({ path: `${out}/${name}.png` });
      if (facts.dpr !== dpr) failures.push(`${name}: incorrect DPR`);
      if (display === "active" && facts.guide !== facts.ordinary) failures.push(`${name}: unrelated ordinary guide hidden (${facts.guide}, expected ${facts.ordinary})`);
      if (display === "active" && facts.guideWidth === 0) failures.push(`${name}: ordinary guide has no painted width`);
      if (display === "active" && facts.ancestryGuide !== facts.ordinary) failures.push(`${name}: ordinary ancestry guide hidden below active path`);
      if (display === "all" && facts.straightVisible && facts.straightBottom > facts.elbowBottom + 1) failures.push(`${name}: straight guide continues beneath last elbow`);
      if (display === "all" && !facts.parentsDrawn) failures.push(`${name}: expanded parent loses its downward line`);
      if (new Set(facts.colors.slice(0, 4)).size !== 4 || facts.colors[0] !== facts.colors[4]) failures.push(`${name}: hierarchy palette does not cycle four depths`);
      if (display === "all") {
        await page.locator('[data-block-id="root"] > .block-children-container > .block-children-left-border').hover();
        const hoveredGuide = await page.locator('[data-block-id="root"] > .block-children-container > .block-children').evaluate(el => (parseFloat(getComputedStyle(el).borderLeftWidth) > 0 ? getComputedStyle(el) : getComputedStyle(el, "::before")).borderLeftColor);
        if (hoveredGuide !== "rgba(0, 0, 0, 0)") failures.push(`${name}: hover duplicates the threaded guide`);
        await page.mouse.move(950, 650);
      }
      evidence.push({ name, ...facts });
    }
    await page.close();
  }
  writeFileSync(`${out}/observations.json`, JSON.stringify({ evidence, failures }, null, 2) + "\n");
  console.log(JSON.stringify({ cases: evidence.length, failures }, null, 2));
  if (failures.length) process.exitCode = 1;
} finally {
  await browser?.close();
  process.kill(-server.pid, "SIGTERM");
}
