// Real Block renderer and shipped CSS; compare alignment semantically, no exact pixels.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const out = resolve(process.argv[2]);
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
  for (const mode of ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width: 900, height: 650 } });
    await page.goto(`http://localhost:${port}/scripts/fixtures/editor-bugs.html`);
    await page.locator(".md-hr").waitFor({ state: "attached" });
    await page.evaluate(value => document.documentElement.setAttribute("data-theme", value), mode);
    await page.screenshot({ path: `${out}/${mode}.png` });
    observations.push(await page.evaluate(mode => {
      const hr = document.querySelector(".md-hr");
      const bullet = hr.closest(".ls-block").querySelector(".bullet");
      const rule = hr.getBoundingClientRect();
      const dot = bullet.getBoundingClientRect();
      return { mode, ruleY: rule.top + rule.height / 2, bulletY: dot.top + dot.height / 2 };
    }, mode));
    await page.locator('[data-block-id="b3"] .block-checkbox').nth(1).click();
    if (await page.locator("textarea.block-editor").count()) throw new Error("Checkbox entered edit");
    if (await page.locator('[data-block-id="b3"] .block-checkbox[aria-checked="true"]').count() !== 1) throw new Error("Checkbox did not toggle");
    await page.close();
  }
  writeFileSync(`${out}/observations.json`, JSON.stringify(observations, null, 2) + "\n");
  console.log(JSON.stringify(observations));
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
}
