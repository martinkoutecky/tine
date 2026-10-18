// Self-verification shots for the action-button rule (D-20261018-UI1/UI2): primary at
// the trailing end, Cancel just left of it, auxiliary actions at the leading end.
// Date picker (no Today button; Done only), the Copy/export dialog, the CSS editor
// ([Reload][Save] ... [Cancel][Save & close]) and the two disk-vs-mine bars
// ([Keep mine][Use disk version]); desktop and phone width.
// The bars are not reachable in the mock, so their markup is injected with the
// components' own class names (DOM order is pinned by DiskOrMineChoice.test.tsx;
// the wrapping behaviour of the real DialogActions is asserted by
// scripts/shot-action-placement-layout.mjs).
// Usage: npm run build && node scripts/shot-action-placement.mjs   (needs scripts/env.sh for Chromium)
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5297;
const OUT = process.env.SHOT_DIR || "/tmp/shot-action-placement";
mkdirSync(OUT, { recursive: true });
const server = spawn("./node_modules/.bin/vite", ["preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function waitForServer(url) {
  for (let i = 0; i < 80; i++) {
    try { if ((await fetch(url)).ok) return; } catch { /* not up yet */ }
    await sleep(250);
  }
  throw new Error("server did not start");
}
const order = (page, selector) => page.evaluate((s) => [...document.querySelectorAll(`${s} button`)].map((b) => b.textContent.trim()), selector);

async function session(browser, phone) {
  const page = await browser.newPage({
    viewport: phone ? { width: 390, height: 780 } : { width: 1200, height: 800 },
    deviceScaleFactor: 2, hasTouch: phone,
  });
  page.on("console", (m) => { if (process.env.SHOT_DEBUG && m.type() === "error") console.log("console:", m.text().slice(0, 200)); });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 8000 });
  await page.evaluate(() => document.querySelector(".toast-close")?.click());
  await sleep(300);
  return page;
}
async function shotBox(page, selector, name, pad = 12) {
  const box = await page.locator(selector).first().boundingBox();
  await page.screenshot({ path: `${OUT}/${name}.png`, clip: { x: Math.max(0, box.x - pad), y: Math.max(0, box.y - pad), width: box.width + 2 * pad, height: box.height + 2 * pad } });
  console.log(`wrote ${OUT}/${name}.png`);
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu"] });
  for (const phone of [false, true]) {
    const tag = phone ? "phone" : "desktop";
    const page = await session(browser, phone);

    // Date picker: Done only, today's cell marked.
    await page.locator(".ls-block .block-content").first().click();
    await sleep(300);
    const ta = page.locator("textarea.block-editor").first();
    await ta.waitFor({ timeout: 4000 });
    await ta.press("End");
    await ta.type("/scheduled", { delay: 20 });
    await sleep(400);
    await page.keyboard.press("Enter");
    await page.locator(".date-picker").waitFor({ timeout: 5000 });
    console.log(tag, "date picker buttons:", JSON.stringify(await order(page, ".dp-foot")), "today cells:", await page.locator(".dp-cell.today").count());
    await shotBox(page, ".date-picker", `${tag}-datepicker`);
    await page.keyboard.press("Escape");
    await sleep(200);
    await page.keyboard.press("Escape");

    // Copy / export dialog.
    await page.locator(".ls-block .block-content").first().click({ button: "right", position: { x: 12, y: 10 } });
    await sleep(400);
    await page.getByText("Copy / export as").first().click();
    await page.waitForSelector(".export-modal", { timeout: 4000 });
    console.log(tag, "export dialog buttons:", JSON.stringify(await order(page, ".export-modal .dialog-actions")));
    await shotBox(page, ".export-modal", `${tag}-export-dialog`);
    await page.keyboard.press("Escape");
    await sleep(300);

    // CSS editor: the mock's binding is never "current", so openCssEditor cannot run
    // in a browser; inject the component's markup (jsdom pins the real order).
    await page.evaluate(() => {
      document.body.insertAdjacentHTML("beforeend", `
        <div class="modal-overlay" id="shot-css"><div class="settings-modal" role="dialog" aria-label="CSS editor"
          style="flex-direction:column;padding:20px;gap:12px"><h2 style="margin:0">logseq/custom.css</h2>
          <textarea style="width:100%;min-height:25vh;font-family:monospace;flex:1;box-sizing:border-box">:root { --x: 1 }</textarea>
          <div class="dialog-actions"><span class="dialog-actions-aux"><button class="settings-btn">Reload</button><button class="settings-btn">Save</button></span>
            <span class="dialog-actions-main"><button class="settings-btn">Cancel</button><button class="settings-btn settings-btn-primary">Save &amp; close</button></span></div></div></div>`);
    });
    await shotBox(page, '#shot-css .settings-modal', `${tag}-css-editor`, 6);
    await page.close();

    // Disk-vs-mine bars, with the components' class names.
    const bars = await session(browser, phone);
    await bars.evaluate(() => {
      const host = document.createElement("div");
      host.style.cssText = "position:fixed;left:0;right:0;top:0;z-index:99999;background:var(--bg-primary);padding:8px";
      host.id = "shot-bars";
      host.innerHTML = `
        <div class="conflict-banner"><span class="conflict-msg">Project Plan has unsaved edits and the file changed on disk.</span>
          <span class="conflict-actions"><button class="conflict-btn">Keep mine</button><button class="conflict-btn">Use disk version</button></span></div>
        <div class="external-change-bar" role="status"><span class="external-change-text">This page changed on disk. Tine is still displaying the version you were reading.</span>
          <span class="external-change-actions"><button class="settings-btn">Keep mine</button><button class="settings-btn">Use disk version</button></span></div>`;
      document.body.append(host);
    });
    await shotBox(bars, "#shot-bars", `${tag}-disk-vs-mine`, 0);
    await bars.close();
  }
  await browser.close();
  server.kill("SIGKILL");
  process.exit(0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
