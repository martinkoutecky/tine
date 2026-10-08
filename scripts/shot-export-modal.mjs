// Screenshot the Copy/Export modal in Rendered (default) and Source modes on a
// markup-heavy block (typographic glyphs, bold, links) — verifies the Content
// toggle and that the preview differs between modes.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5196;
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}
try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 900, height: 900 } });
  const errors = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });
  // Fresh options each run (the modal persists the last-used content mode).
  await page.evaluate(() => localStorage.removeItem("tine.exportOptions"));
  // Give the first block markup-heavy content, then open its context menu → export.
  await page.evaluate(() => {
    const ta = document.querySelector("textarea.block-editor");
    if (ta) return; // not editing — fine, we use the rendered row below
  });
  const first = page.locator(".ls-block .block-content").first();
  await first.click(); // enters edit (mousedown+up via playwright)
  await sleep(400);
  await page.keyboard.press("Control+a");
  await page.keyboard.type("**bold** then a -> b and x -- y with [[Page Link]] here");
  await page.keyboard.press("Escape");
  await sleep(400);
  await page.locator(".ls-block .block-content").first().click({ button: "right", position: { x: 12, y: 10 } });
  await sleep(500);
  await page.screenshot({ path: "screenshots/export-modal-debug.png" });
  const menuTexts = await page.evaluate(() =>
    [...document.querySelectorAll("body *")].filter((e) => /Copy \/ export/.test(e.textContent || "") && e.children.length === 0).map((e) => e.className + "|" + e.textContent)
  );
  console.log("menu candidates:", JSON.stringify(menuTexts).slice(0, 300));
  const item = page.getByText("Copy / export as").first();
  await item.click();
  await page.waitForSelector(".export-modal", { timeout: 3000 });
  await sleep(300);
  await page.screenshot({ path: "screenshots/export-modal-rendered.png" });
  await page.getByRole("button", { name: "Source" }).click();
  await sleep(300);
  await page.screenshot({ path: "screenshots/export-modal-source.png" });
  const preview = await page.locator(".export-preview").inputValue();
  console.log("source preview:", JSON.stringify(preview));
  console.log(errors.length ? "ERRORS:\n" + errors.join("\n") : "no console errors");
  await browser.close();
  server.kill("SIGKILL");
  process.exit(0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
