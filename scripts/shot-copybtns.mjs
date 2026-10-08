// Verification-only shot for #24 copy buttons (inline code, links, code block).
// Forces the hover state visible so placement/appearance is checkable, plus a
// genuine hover to confirm the natural reveal works. Not a README shot.
// Usage: source scripts/env.sh && npm run build && node scripts/shot-copybtns.mjs
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5209;
const OUT = "screenshots";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

async function waitForServer(url, tries = 40) {
  for (let i = 0; i < tries; i++) {
    try { const r = await fetch(url); if (r.ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

async function clip(page, loc, w, h, path, padX = 20, padY = 24) {
  await loc.evaluate((el) => el.scrollIntoView({ block: "center" }));
  await sleep(250);
  const box = await loc.boundingBox();
  await page.screenshot({
    path,
    clip: { x: Math.max(0, box.x - padX), y: Math.max(0, box.y - padY), width: w, height: h },
  });
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1100, height: 900 }, deviceScaleFactor: 2 });
  page.on("pageerror", (e) => console.log("pageerror:", String(e).split("\n")[0]));

  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(500);
  // Scroll the whole feed so lazy IntersectionObserver block bodies all mount.
  for (let y = 0; y < 14; y++) { await page.mouse.wheel(0, 700); await sleep(120); }
  await sleep(400);

  console.log("inline-copy-wrap count:", await page.locator(".inline-copy-wrap").count());
  console.log("link-copy-wrap   count:", await page.locator(".link-copy-wrap").count());
  console.log("code-block       count:", await page.locator(".code-block").count());

  // (1) genuine hover on the first inline code — confirms natural reveal
  try {
    const code = page.locator(".inline-copy-wrap").first();
    await code.evaluate((el) => el.scrollIntoView({ block: "center" }));
    await sleep(200);
    await code.hover();
    await sleep(250);
    const box = await code.boundingBox();
    await page.screenshot({
      path: `${OUT}/copybtn-inline-hover.png`,
      clip: { x: Math.max(0, box.x - 20), y: Math.max(0, box.y - 24), width: 480, height: 80 },
    });
    console.log("OK    copybtn-inline-hover");
  } catch (e) { console.log("FAIL  inline-hover", String(e).split("\n")[0]); }

  // (2) force ALL copy buttons visible, then shoot inline, link, code block
  await page.addStyleTag({ content: ".copy-btn{opacity:1 !important;}" });
  await sleep(200);
  try { await clip(page, page.locator(".inline-copy-wrap").first(), 480, 80, `${OUT}/copybtn-inline-forced.png`); console.log("OK    inline-forced"); }
  catch (e) { console.log("FAIL  inline-forced", String(e).split("\n")[0]); }
  try { await clip(page, page.locator(".link-copy-wrap").first(), 560, 120, `${OUT}/copybtn-links-forced.png`); console.log("OK    links-forced"); }
  catch (e) { console.log("FAIL  links-forced", String(e).split("\n")[0]); }
  try {
    const cb = page.locator(".code-block").first();
    await cb.evaluate((el) => el.scrollIntoView({ block: "center" }));
    await sleep(250);
    await cb.screenshot({ path: `${OUT}/copybtn-codeblock-forced.png` });
    console.log("OK    codeblock-forced");
  } catch (e) { console.log("FAIL  codeblock-forced", String(e).split("\n")[0]); }

  await browser.close();
} finally {
  server.kill("SIGKILL");
}
