import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const out = resolve(process.argv[2] || "artifacts/bullet-threading/screenshots");
mkdirSync(out, { recursive: true });
const port = 20000 + Math.floor(Math.random() * 20000);
const server = spawn("npx", ["vite", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
const observations = [];
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (attempt === 80) throw new Error("Vite did not start");
    await delay(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  const page = await browser.newPage({ viewport: { width: 800, height: 500 } });
  await page.goto(`http://localhost:${port}/scripts/fixtures/bullet-threading.html`, { timeout: 120_000 });
  await page.locator('[data-block-id="focus"] > .block-main .block-content').click();
  await page.locator("textarea.block-editor").waitFor();
  if (process.env.THREAD_BASELINE_CSS) {
    await page.evaluate(css => {
      document.querySelector('style[data-vite-dev-id$="/src/styles/app.css"]').textContent = css;
    }, readFileSync(process.env.THREAD_BASELINE_CSS, "utf8"));
  }
  for (const theme of ["light", "dark"]) {
    for (const config of [
      { name: "all-1", display: "all", thickness: 1 },
      { name: "active-1", display: "active", thickness: 1 },
      { name: "all-0", display: "all", thickness: 0 },
      { name: "all-4", display: "all", thickness: 4 },
      { name: "custom", display: "all", thickness: 4, color: "#e05070" },
      { name: "levels", display: "all", thickness: 4, "color-by-level": true },
    ]) {
      await page.evaluate(({ theme, config }) => {
        document.documentElement.setAttribute("data-theme", theme);
        window.setThreadSettings({ color: "", "color-by-level": false, ...config });
      }, { theme, config });
      await page.screenshot({ path: `${out}/${theme}-${config.name}.png` });
      const state = await page.evaluate(() => {
        const row = document.querySelector('[data-block-id="focus"] > .block-main');
        const line = getComputedStyle(row, "::before");
        const root = document.querySelector('[data-block-id="root"] > .block-main');
        const vertical = getComputedStyle(root, "::after");
        const unrelated = getComputedStyle(document.querySelector('[data-block-id="last"] > .block-main'), "::before");
        const bullet = row.querySelector(".bullet");
        const dot = bullet.getBoundingClientRect();
        const box = row.getBoundingClientRect();
        return { elbow: line.borderBottomColor, width: line.borderBottomWidth, display: line.display,
          vertical: vertical.borderLeftColor, unrelated: unrelated.borderBottomColor, unrelatedDisplay: unrelated.display,
          endpointY: box.top + parseFloat(line.height), bulletY: dot.top + dot.height / 2,
          bulletColor: getComputedStyle(bullet).backgroundColor };
      });
      observations.push({ theme, config, ...state });
      if (config.thickness === 0) {
        if (state.display !== "none") throw new Error("Zero thickness did not hide lines");
      } else {
        if (state.width !== `${config.thickness}px`) throw new Error("Thickness not mapped");
        if (Math.abs(state.endpointY - state.bulletY) > config.thickness) throw new Error("Elbow misses bullet");
        if (config.display === "all" && state.elbow === state.unrelated) throw new Error("Active path not distinct in all mode");
        if (config.display === "active" && state.unrelatedDisplay !== "none") throw new Error("Active mode shows unrelated elbows");
        if (config.color && state.elbow !== "rgb(224, 80, 112)") throw new Error("Custom color not applied");
      }
    }
  }
  for (const theme of ["nord", "solarized", "gruvbox", "soft"]) {
    for (const mode of ["light", "dark"]) {
      await page.evaluate(({ theme, mode }) => {
        document.documentElement.setAttribute("data-theme", mode);
        window.setThreadTheme(theme);
        window.setThreadSettings({ display: "all", thickness: 4, "color-by-level": true });
      }, { theme, mode });
      await page.screenshot({ path: `${out}/${theme}-${mode}.png` });
    }
  }
  await page.evaluate(() => {
    window.setThreadTheme("");
    window.setThreadSettings({ display: "active", thickness: 4, color: "" });
  });
  await page.locator('[data-block-id="last"] > .block-main .block-content').click();
  const transferred = await page.locator('[data-block-id="focus"] > .block-main').evaluate(el => getComputedStyle(el, "::before").display);
  if (transferred !== "none") throw new Error("Focus move retained the previous active ancestry");
  await page.screenshot({ path: `${out}/focus-transferred.png` });
  await page.evaluate(() => {
    const children = document.querySelector('[data-block-id="root"] > .block-children-container > .block-children');
    const list = document.createElement("div"); list.className = "outline-list";
    for (const id of ["before", null, "parent", "last"]) {
      const window = document.createElement("div"); window.className = "outline-window";
      if (id) window.append(document.querySelector(`[data-block-id="${id}"]`));
      else { const spacer = document.createElement("div"); spacer.setAttribute("aria-hidden", "true"); spacer.style.height = "45px"; window.append(spacer); }
      list.append(window);
    }
    children.append(list);
  });
  await page.locator('[data-block-id="focus"] > .block-main .block-content').click();
  const spacer = await page.locator('.outline-window > [aria-hidden="true"]').evaluate(el => {
    const line = getComputedStyle(el, "::before");
    return { display: line.display, width: line.borderLeftWidth, color: line.borderLeftColor };
  });
  if (spacer.display === "none" || spacer.width !== "4px") throw new Error("Window placeholder interrupts active path");
  await page.screenshot({ path: `${out}/windowed.png` });
  const windowPath = await page.evaluate(() => ({
    preceding: getComputedStyle(document.querySelector('[data-block-id="before"]'), "::after").display,
    elbow: getComputedStyle(document.querySelector('[data-block-id="parent"] > .block-main'), "::before").display,
    elbowContent: getComputedStyle(document.querySelector('[data-block-id="parent"] > .block-main'), "::before").content,
  }));
  if (windowPath.preceding === "none" || windowPath.elbow === "none" || windowPath.elbowContent === "none") throw new Error("Window wrappers interrupt the ancestry path");
  const mobile = await browser.newPage({ viewport: { width: 500, height: 500 }, isMobile: true, hasTouch: true });
  await mobile.goto(`http://localhost:${port}/scripts/fixtures/bullet-threading.html`, { timeout: 120_000 });
  await mobile.locator('[data-block-id="focus"] > .block-main .block-content').click();
  await mobile.evaluate(() => {
    document.documentElement.setAttribute("data-theme", "light");
    window.setThreadSettings({ display: "all", thickness: 4 });
  });
  await mobile.screenshot({ path: `${out}/mobile.png` });
  const mobilePath = await mobile.locator('[data-block-id="root"] > .block-main').evaluate(el => {
    const line = getComputedStyle(el, "::after");
    const dot = el.querySelector(".bullet").getBoundingClientRect();
    return { lineX: el.getBoundingClientRect().left + parseFloat(line.left) + parseFloat(line.borderLeftWidth) / 2, dotX: dot.left + dot.width / 2 };
  });
  if (Math.abs(mobilePath.lineX - mobilePath.dotX) > 1) throw new Error("Mobile path misses parent bullet");
  await mobile.close();
  writeFileSync(`${out}/observations.json`, JSON.stringify(observations, null, 2) + "\n");
  console.log(`Passed ${observations.length} screenshot configurations`);
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
}
