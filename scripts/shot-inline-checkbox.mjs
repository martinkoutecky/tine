// Real Block + shipped CSS: compare list hierarchy against the outline indent.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const out = path.resolve(process.argv[2] || "artifacts/inline-checkbox/browser");
fs.mkdirSync(out, { recursive: true });
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
    await page.goto(`http://localhost:${port}/scripts/fixtures/inline-checkbox.html`);
    await page.locator(".block-checkbox").nth(1).waitFor();
    await page.evaluate(value => document.documentElement.setAttribute("data-theme", value), mode);
    await page.screenshot({ path: `${out}/${mode}.png` });
    const geometry = await page.evaluate(() => {
      const boxes = [...document.querySelectorAll(".block-checkbox")];
      const parent = document.querySelector('[data-block-id="list"] > .block-main .block-content');
      const child = document.querySelector('[data-block-id="child"] .block-content');
      return { listStep: boxes[1].getBoundingClientRect().left - boxes[0].getBoundingClientRect().left,
        outlineStep: child.getBoundingClientRect().left - parent.getBoundingClientRect().left };
    });
    observations.push({ mode, ...geometry });
    fs.writeFileSync(`${out}/observations.json`, JSON.stringify(observations, null, 2));
    // A readable hierarchy comparable to ordinary child blocks, without prescribing exact pixels.
    if (geometry.listStep < geometry.outlineStep * 0.75) throw new Error(`Nested checklist hierarchy is nearly flush: ${JSON.stringify(geometry)}`);
    await page.locator(".block-checkbox").nth(1).click();
    if (await page.locator("textarea.block-editor").count()) throw new Error("Checkbox entered edit");
    if (await page.locator('.block-checkbox[aria-checked="true"]').count() !== 1) throw new Error("Checkbox did not toggle independently");
    await page.close();
  }
  console.log(JSON.stringify(observations));
} finally {
  await browser?.close();
  try { process.kill(-server.pid, "SIGTERM"); } catch {}
}
