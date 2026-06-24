// Dedicated quick-capture screenshot. The capture window is a *frameless* OS
// mini-window (capture.html); the mock can only render its web content, so we
// (a) drive the real editor to open the slash menu — proving the capture window
// has the full autocomplete/slash UI, not a dumb text box — and (b) add the
// drop-shadow + rounded corners the window manager gives the real floating
// window, so the shot reads as a window rather than a bare div.
// Usage: node scripts/shot-capture.mjs   (source scripts/env.sh first)
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5197;
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

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  // Transparent page so the window card's shadow shows against the README.
  const page = await browser.newPage({
    viewport: { width: 720, height: 460 },
    deviceScaleFactor: 2,
  });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));

  await page.goto(`http://localhost:${PORT}/capture.html`);
  await page.waitForSelector(".capture-shell textarea", { timeout: 8000 });

  // Frame the frameless window: center a fixed-width card with the shadow/radius
  // the real WM draws, on a soft backdrop so the card reads as floating.
  await page.addStyleTag({
    content: `
      html, body { background: #e9eef5 !important; height: 100%; }
      #capture-root {
        width: 560px; margin: 60px auto 0; background: var(--bg-primary, #fff);
        border-radius: 12px; box-shadow: 0 18px 50px rgba(20,30,60,.28), 0 2px 6px rgba(20,30,60,.12);
        overflow: visible;
      }
    `,
  });

  const ta = page.locator(".capture-shell textarea");
  await ta.click();
  await page.keyboard.type("Idea: ship the v0.1 release post ", { delay: 8 });
  await page.keyboard.type("/"); // open the slash menu
  await page.waitForSelector(".autocomplete", { timeout: 4000 }).catch(() => {});
  await sleep(350);

  await page.screenshot({ path: `${OUT}/rm-quick-capture.png` });
  console.log("OK    rm-quick-capture");

  await browser.close();
  console.log("done");
} finally {
  server.kill("SIGTERM");
}
