// Verification shot for GH #319 (og 22d): the full-screen lightbox image must keep
// its natural aspect ratio when the graph's custom.css fixes a width/height on every
// `img`. Loads the real app stylesheet through the vite dev server, injects the
// lightbox markup plus a hostile `img { width; height }` rule, and asserts the
// rendered box keeps the image's own 16:9 ratio. Writes screenshots/lightbox-custom-css.png.
// Usage: source scripts/env.sh && node scripts/shot-lightbox-custom-css.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5231;
mkdirSync("screenshots", { recursive: true });
const server = spawn("npx", ["vite", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
try {
  for (let i = 0; ; i++) {
    try { if ((await fetch(`http://localhost:${PORT}/`)).ok) break; } catch {}
    if (i > 80) throw new Error("dev server did not start");
    await sleep(250);
  }
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1100, height: 800 } });
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 15000 });
  const svg = "data:image/svg+xml," + encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="900"><rect width="1600" height="900" fill="#3b82f6"/><circle cx="800" cy="450" r="300" fill="#fbbf24"/></svg>');
  const box = await page.evaluate((src) => {
    const style = document.createElement("style");
    style.textContent = "img { width: 320px; height: 120px; }"; // hostile graph custom.css
    document.head.appendChild(style);
    const overlay = document.createElement("div");
    overlay.className = "lightbox-overlay";
    overlay.innerHTML = `<img class="lightbox-img" src="${src}" alt="">`;
    document.body.appendChild(overlay);
    return new Promise((resolve) => {
      const img = overlay.querySelector("img");
      const done = () => { const r = img.getBoundingClientRect(); resolve({ w: r.width, h: r.height }); };
      img.complete ? done() : (img.onload = done);
    });
  }, svg);
  await page.screenshot({ path: "screenshots/lightbox-custom-css.png" });
  await browser.close();
  const ratio = box.w / box.h;
  console.log(`lightbox image box ${box.w.toFixed(0)}x${box.h.toFixed(0)} ratio ${ratio.toFixed(3)} (want 1.778)`);
  if (Math.abs(ratio - 16 / 9) > 0.01) { console.error("FAIL: lightbox image distorted by custom css"); process.exitCode = 1; }
  else console.log("lightbox custom-css geometry OK");
} finally {
  server.kill();
}
