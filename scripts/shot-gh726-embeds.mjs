// GH #726: inspect host/source bullets for both embed forms before and after.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const out = path.resolve("artifacts/gh726-embeds", process.env.SHOT_PHASE || "after");
fs.mkdirSync(out, { recursive: true });
const port = 5276;
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "preview", "--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
try {
  for (let i = 0; ; i++) {
    try { if ((await fetch(`http://localhost:${port}`)).ok) break; } catch {}
    if (i > 60) throw new Error("preview did not start");
    await sleep(250);
  }
  browser = await chromium.launch();
  for (const viewport of [{ width: 1000, height: 850 }, { width: 390, height: 760 }]) {
    const page = await browser.newPage({ viewport });
    await page.goto(`http://localhost:${port}/?regressions`);
    await page.waitForSelector(".ls-block");
    for (const [kind, title, text] of [
      ["block", "Block embed regression", "Embedded grandchild"],
      ["page", "Page embed regression", "Source grandchild"],
    ]) {
      await page.keyboard.press("Control+k");
      await page.locator(".switcher-input").fill(title);
      await page.waitForFunction((title) => {
        if (document.querySelector('#switcher-results [role="status"]')) return false;
        const row = [...document.querySelectorAll(".switcher-row:not(.block-result)")]
          .find(row => row.querySelector(".switcher-name")?.textContent.trim() === title);
        if (!row) return false;
        row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
        return true;
      }, title);
      await page.getByText(text, { exact: true }).first().waitFor();
      await page.screenshot({ path: path.join(out, `${kind}-${viewport.width}.png`) });
    }
    await page.close();
  }
  console.log(`Screenshots: ${out}`);
} finally {
  await browser?.close();
  server.kill();
}
