// Focused screenshots for the README's "highlights" gallery. Each shot is
// isolated in try/catch so one failure doesn't abort the rest. Skips the
// PDF flow (slow/can hang headless). Usage: node scripts/shot-readme.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5193;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });

const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], {
  stdio: "inherit",
});

async function waitForServer(url, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try {
      const r = await fetch(url);
      if (r.ok) return;
    } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

const shot = async (page, name, fn) => {
  try {
    await fn();
    await page.screenshot({ path: `${OUT}/${name}.png` });
    console.log("OK   ", name);
  } catch (e) {
    console.log("FAIL ", name, String(e).split("\n")[0]);
  }
};

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1180, height: 820 }, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(500);

  // --- Tabs: open a couple of pages in background tabs + pin one ----------
  await shot(page, "rm-tabs", async () => {
    await page.locator(".nav-item").first().click().catch(() => {});
    await sleep(150);
    const navPages = page.locator(".nav-page");
    if (await navPages.count()) {
      await navPages.nth(0).click({ button: "middle" });
      await sleep(150);
      if ((await navPages.count()) > 1) await navPages.nth(1).click({ button: "middle" });
      await sleep(150);
    }
    const tabs = page.locator(".tab");
    if ((await tabs.count()) > 1) await tabs.nth(1).dblclick();
    await sleep(250);
  });

  // --- Focus mode + dim inactive blocks -----------------------------------
  await shot(page, "rm-focus-dim", async () => {
    // make sure we're not in an editor so the `t f` / `t b` sequences fire
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");
    await sleep(100);
    await page.keyboard.press("t");
    await page.keyboard.press("f"); // focus mode
    await sleep(150);
    await page.keyboard.press("t");
    await page.keyboard.press("b"); // dim inactive blocks
    await sleep(150);
    // give it an active block to spotlight
    const blocks = page.locator(".block-content");
    if (await blocks.count()) await blocks.nth(1).click();
    await sleep(300);
  });
  // leave focus/dim mode so later shots are normal
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await page.keyboard.press("t");
  await page.keyboard.press("f");
  await page.keyboard.press("t");
  await page.keyboard.press("b");
  await sleep(150);

  // --- Quick-capture window (rendered standalone) -------------------------
  const cap = await browser.newPage({ viewport: { width: 620, height: 240 }, deviceScaleFactor: 2 });
  await shot(cap, "rm-quick-capture", async () => {
    await cap.goto(`http://localhost:${PORT}/capture.html`);
    await cap.waitForSelector(".capture-shell textarea", { timeout: 8000 });
    await sleep(300);
    await cap.locator(".capture-shell textarea").click();
    await cap.keyboard.type("TODO call the dentist ");
    await cap.keyboard.type("#health ");
    await sleep(300);
  });
  await cap.close();

  await browser.close();
  console.log("done");
} finally {
  server.kill("SIGTERM");
}
