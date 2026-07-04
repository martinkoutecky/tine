// REAL end-to-end smoke test of the diff panel (NO fixture): opens the panel and
// clicks Run, so lsdoc-wasm parses and mldoc actually loads + runs in the Web
// Worker inside a real browser. Validates the whole runtime pipeline. Chromium is
// a strong proxy for Tine's WebKitGTK (the exact WebKitGTK check is the app run).
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5221;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
const wait = async (u, t = 40) => { for (let i = 0; i < t; i++) { try { const r = await fetch(u); if (r.ok) return; } catch {} await sleep(250); } throw new Error("no server"); };

const errors = [];
try {
  await wait(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => errors.push("pageerror: " + String(e).split("\n")[0]));
  page.on("console", (m) => { if (m.type() === "error") errors.push("console.error: " + m.text()); });
  page.on("worker", (w) => console.log("worker created:", w.url().split("/").pop()));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(400);
  await page.locator('button.icon-btn[title^="Settings"]').first().click();
  await page.waitForSelector(".settings-modal", { timeout: 3000 });
  await page.locator(".settings-nav-item", { hasText: "Help improve Tine" }).first().click();
  await page.waitForSelector(".improve-tab", { timeout: 3000 });

  console.log("clicking Run (real lsdoc + mldoc)…");
  await page.locator(".improve-run .btn-primary").click();
  await page.waitForSelector(".improve-report", { timeout: 25000 });
  await sleep(500);

  // Pull the rendered outcome out of the DOM.
  const summary = await page.evaluate(() => {
    const t = (sel) => document.querySelector(sel)?.textContent?.trim() || "";
    const rows = [...document.querySelectorAll(".improve-table tbody tr")].map((tr) =>
      [...tr.querySelectorAll("td")].map((td) => td.textContent.trim()).join(" | "),
    );
    const cards = [...document.querySelectorAll(".improve-card")].map((c) => c.querySelector("code")?.textContent || "?");
    return {
      scanned: t(".improve-report > .settings-hint"),
      clean: t(".improve-clean"),
      benchRows: rows,
      divergenceHeader: t(".improve-findings h4"),
      cards,
      notice: t(".improve-notice"),
    };
  });
  console.log("REPORT:", JSON.stringify(summary, null, 2));
  await page.evaluate(() => { const b = document.querySelector(".settings-pane-body"); if (b) b.scrollTop = b.scrollHeight; });
  await sleep(300);
  await page.screenshot({ path: `${OUT}/improve-realrun.png` });

  await browser.close();
} finally {
  server.kill("SIGTERM");
}
console.log(errors.length ? "\nERRORS:\n" + errors.join("\n") : "\nNo page/console errors. ✓");
process.exit(errors.some((e) => !e.includes("favicon")) ? 2 : 0);
