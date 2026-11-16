// GH #849: an idle page with many linked references must reach a fixed point. Chromium, the engine of the
// reporter's Windows WebView2: opens the hub, scrolls to the middle of its references, optionally runs a Ctrl+F,
// then does nothing and records every change of scrollTop, scrollHeight, per-row heights and DOM mutations.
// Usage: npm run build && node scripts/shot-gh849-idle.mjs [seconds]   (GH849_FIND=0 skips the find step)
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = Number(process.env.GH849_PORT || 5289);
const OUT = process.env.E2E_ARTIFACT_DIR || "artifacts/gh849-idle";
const SECONDS = Number(process.argv[2] || 60);
const SETTLE_MS = 6000;
const FIND = process.env.GH849_FIND !== "0";
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore", detached: true });
let browser, failure = null;
// The failure this guards against starves the page's main thread, so a stuck evaluate must fail the run too.
const watchdog = setTimeout(() => { console.error("FAIL: the page stopped answering (main thread busy)"); try { process.kill(-server.pid); } catch {} process.exit(1); }, 90_000 + SECONDS * 1000);
try {
  for (let i = 0; i < 80; i++) {
    try { if ((await fetch(`http://localhost:${PORT}/`)).ok) break; } catch {}
    await sleep(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1100, height: 800 } });
  await page.goto(`http://localhost:${PORT}/?manyrefs`);
  await page.waitForSelector(".ls-block", { timeout: 30_000 });
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("Hub");
  await page.waitForTimeout(400);
  await page.locator(".switcher-row", { hasText: "Hub" }).first().click();
  await page.locator(".linked-references .references-header").click(); // a section this long starts folded
  await page.waitForSelector(".linked-references .reference-blocks", { timeout: 30_000 });
  const groups = await page.locator(".reference-blocks").count();
  if (groups < 90) throw new Error(`the fixture shows ${groups} reference groups, expected 93`);
  // Walk down in steps, as a reader does, so groups load in order: FULL walks to the end and comes back to the
  // middle (more pages visited than the working set holds), otherwise it stops halfway.
  const FULL = process.env.GH849_WALK === "full";
  for (let i = 0; i < 400; i++) {
    const f = await page.evaluate(() => { const s = document.querySelector(".main-content"); s.scrollTop += 1200; return (s.scrollTop + s.clientHeight) / s.scrollHeight; });
    await page.waitForTimeout(100);
    if (f > (FULL ? 0.995 : 0.5)) break;
  }
  if (FULL) await page.evaluate(() => { const s = document.querySelector(".main-content"); s.scrollTop = s.scrollHeight / 2; });
  if (FIND) {
    await page.waitForTimeout(800);
    await page.keyboard.press("Control+f");
    await page.locator(".inpage-find-bar input").first().fill("Foreign");
    await page.waitForTimeout(800);
    for (let i = 0; i < Number(process.env.GH849_STEPS || 3); i++) { await page.keyboard.press("Enter"); await page.waitForTimeout(500); }
  }
  await page.waitForTimeout(SETTLE_MS);
  await page.evaluate(() => {
    const s = document.querySelector(".main-content");
    const rec = window.__idle = { frames: 0, events: [], mutations: 0, sample: [], loaded: 0, running: true };
    const heights = () => { const m = new Map(); for (const el of s.querySelectorAll(".reference-blocks")) m.set(el.dataset.inpageFindSurface, Math.round(el.getBoundingClientRect().height * 10) / 10); return m; };
    let last = { top: s.scrollTop, height: s.scrollHeight, rows: heights() };
    new MutationObserver((r) => {
      rec.mutations += r.length;
      if (rec.sample.length < 40) for (const m of r.slice(0, 3)) rec.sample.push({ t: Math.round(performance.now()), type: m.type, target: (m.target.className || m.target.nodeName || "").toString().slice(0, 60), added: m.addedNodes.length, removed: m.removedNodes.length });
    }).observe(s, { childList: true, subtree: true, characterData: true, attributes: true });
    const tick = () => {
      rec.frames++;
      const rows = heights(), changed = [];
      for (const [k, h] of rows) if (last.rows.get(k) !== h) changed.push([k.split(":").pop(), last.rows.get(k), h]);
      if (s.scrollTop !== last.top || s.scrollHeight !== last.height || changed.length) {
        if (rec.events.length < 400) rec.events.push({ t: Math.round(performance.now()), top: s.scrollTop, height: s.scrollHeight, changed: changed.slice(0, 5) });
      }
      last = { top: s.scrollTop, height: s.scrollHeight, rows };
      if (rec.running) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  await page.waitForTimeout(SECONDS * 1000);
  const rec = await page.evaluate(() => { window.__idle.running = false; const s = document.querySelector(".main-content"); return { ...window.__idle, top: s.scrollTop, height: s.scrollHeight, raw: document.querySelectorAll(".main-content .ast-deferred").length }; });
  await page.screenshot({ path: path.join(OUT, "idle.png") });
  writeFileSync(path.join(OUT, "result.json"), JSON.stringify(rec, null, 1));
  console.log(JSON.stringify({ frames: rec.frames, events: rec.events.length, mutations: rec.mutations, top: rec.top, height: rec.height, first: rec.events.slice(0, 4) }));
  if (rec.events.length || rec.mutations) failure = `the idle page kept changing: ${rec.events.length} geometry changes, ${rec.mutations} DOM mutations`;
} catch (error) {
  failure = String(error);
} finally {
  clearTimeout(watchdog);
  await browser?.close().catch(() => {});
  try { process.kill(-server.pid); } catch {}
}
if (failure) { console.error(`FAIL: ${failure}`); process.exit(1); }
console.log("PASS: the idle page reached a fixed point");
