// GH #761/#762: real sidebar and Settings in the disposable browser sample graph.
// Usage: node scripts/shot-mac-polish.mjs before|after
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";

const phase = process.argv[2];
if (!["before", "after"].includes(phase)) throw new Error("Expected before|after");
const out = `artifacts/mac-polish/${phase}`;
mkdirSync(out, { recursive: true });
const port = 23000 + Math.floor(Math.random() * 10000);
const server = spawn("npx", ["vite", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
try {
  for (let attempt = 0; ; attempt++) {
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (attempt === 80) throw new Error("Vite did not start");
    await delay(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  const observations = [];
  for (const platform of (phase === "after" ? ["linux", "mac"] : ["linux"])) for (const mode of ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 2 });
    if (platform === "mac") await page.addInitScript(() => Object.defineProperty(navigator, "platform", { get: () => "MacIntel" }));
    await page.goto(`http://localhost:${port}/`);
    await page.locator(".graph-switch-btn").waitFor();
    await page.evaluate(async () => {
      const { setFavorites } = await import("/src/favorites.ts");
      const { setRecentPages } = await import("/src/ui.ts");
      setFavorites([{ name: "Tine", kind: "page" }]);
      setRecentPages([{ name: "Tine", kind: "page" }]);
    });
    await page.evaluate((mode) => document.documentElement.setAttribute("data-theme", mode), mode);
    await page.evaluate(() => document.fonts.ready);
    const label = `${platform === "mac" ? "mac-" : ""}${mode}`;
    await page.screenshot({ path: `${out}/${label}.png` });
    observations.push(await page.evaluate(() => [...document.querySelectorAll(".nav-toggle-caret, .graph-switch-caret, .workspace-switcher-caret")].map((el) => ({
      class: el.className, svg: !!el.querySelector("svg"), text: el.textContent,
      width: el.getBoundingClientRect().width, height: el.getBoundingClientRect().height,
      font: getComputedStyle(el.parentElement).fontFamily,
    }))));
    if (phase === "after") {
      await page.keyboard.press(platform === "mac" ? "Meta+," : "Control+,");
      await page.locator(".settings-modal").waitFor();
      await page.getByRole("button", { name: "Keyboard shortcuts", exact: true }).click();
      const row = page.locator(".help-shortcut-row").filter({ hasText: "ui/open-settings" });
      await row.waitFor();
      const text = await row.innerText();
      if (!text.includes("t s") || !text.includes("or") || !text.includes(",")) throw new Error(`Missing shortcut alias: ${text}`);
      await row.screenshot({ path: `${out}/${label}-settings-shortcuts.png` });
    }
    await page.close();
  }
  writeFileSync(`${out}/observations.json`, JSON.stringify(observations, null, 2) + "\n");
  console.log(`Captured ${phase} light/dark sidebar${phase === "after" ? " and shortcut rows" : ""}.`);
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
}
