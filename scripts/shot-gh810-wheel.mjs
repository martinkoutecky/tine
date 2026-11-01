// GH #810: real mouse-wheel scrolling past expanded block embeds must not throw the blocks in view back the
// other way. Needs real X11 wheel events: only those drive Chromium's compositor smooth scrolling, the path that
// fails. CDP wheel events and scrollBy({behavior:"smooth"}) pass on the broken code, as does WebKitGTK.
// Usage: npm run build && xvfb-run -a -s "-screen 0 1400x1600x24" node scripts/shot-gh810-wheel.mjs
// Needs xdotool and the full Chromium (not the headless shell) with its system libraries.
// Sampling: on every animation frame, the most common on-screen movement of the blocks seen in both frames.
import { chromium } from "playwright";
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = Number(process.env.GH810_PORT || 5288);
const OUT = process.env.E2E_ARTIFACT_DIR || "artifacts/gh810-wheel";
const LIMIT = 40; // px moved against the scroll direction in one frame
const NOTCHES = 3;
const MAX_STEPS = 70;
const W = 1134, H = 1398, TOP = 140;
mkdirSync(OUT, { recursive: true });
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
let failure = null;
try {
  for (let i = 0; i < 80; i++) {
    try { if ((await fetch(`http://localhost:${PORT}/`)).ok) break; } catch {}
    await sleep(250);
  }
  browser = await chromium.launch({
    headless: false,
    args: ["--no-sandbox", "--disable-dev-shm-usage", "--disable-gpu", "--disable-crash-reporter",
      "--enable-smooth-scrolling", "--window-position=0,0", `--window-size=${W},${H + TOP}`],
  });
  const page = await browser.newPage({ viewport: { width: W, height: H } });
  await page.goto(`http://localhost:${PORT}/?embedscroll`);
  await page.waitForSelector(".ls-block", { timeout: 30_000 });
  await page.keyboard.press("Control+k");
  await page.locator(".switcher-input").fill("Embed scroll");
  await page.waitForTimeout(400);
  await page.locator(".switcher-row", { hasText: "Embed scroll" }).filter({ hasNotText: "source" }).first().click();
  await page.waitForFunction(() => document.querySelector(".main-content .embed-block .ls-block"), null, { timeout: 30_000 });
  await page.waitForTimeout(1500);
  const windows = await page.evaluate(() => document.querySelectorAll(".main-content [data-outline-window]").length);
  if (windows < 4) throw new Error(`the fixture did not window (${windows} outline windows)`);
  await page.evaluate(() => {
    const scroller = document.querySelector(".main-content");
    const rec = window.__gh810 = { phase: "", against: [], frames: 0, running: true };
    let previous = null;
    const tick = () => {
      const seen = new Map(), counts = new Map(), box = scroller.getBoundingClientRect();
      for (const row of scroller.querySelectorAll(".ls-block > .block-main")) {
        const id = row.parentElement.getAttribute("data-block-id") || row.textContent.slice(0, 40);
        const n = (counts.get(id) || 0) + 1;
        counts.set(id, n);
        const r = row.getBoundingClientRect();
        if (r.bottom >= box.top && r.top <= box.bottom) seen.set(`${id}#${n}`, r.top);
      }
      if (previous) {
        const moves = new Map();
        for (const [k, y] of seen) if (previous.has(k)) { const d = Math.round(y - previous.get(k)); moves.set(d, (moves.get(d) || 0) + 1); }
        let move = 0, best = 0;
        for (const [d, c] of moves) if (c > best) { best = c; move = d; }
        if ((rec.phase === "down" ? 1 : -1) * move > 6) rec.against.push({ phase: rec.phase, scrollTop: Math.round(scroller.scrollTop), move });
      }
      rec.frames++;
      previous = seen;
      if (rec.running) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  const position = () => page.evaluate(() => { const s = document.querySelector(".main-content"); return { top: s.scrollTop, height: s.scrollHeight, client: s.clientHeight }; });
  execFileSync("xdotool", ["mousemove", String(Math.round(W / 2)), String(Math.round(H / 2) + TOP)]);
  const passes = [];
  for (const phase of ["down", "up", "down", "up"]) {
    await page.evaluate((p) => { window.__gh810.phase = p; }, phase);
    let steps = 0;
    for (; steps < MAX_STEPS; steps++) {
      const s = await position();
      if (phase === "down" ? s.top + s.client >= s.height - 2 : s.top <= 0) break;
      execFileSync("xdotool", ["click", "--repeat", String(NOTCHES), "--delay", "30", phase === "down" ? "5" : "4"]);
      await page.waitForTimeout(280);
    }
    await page.waitForTimeout(600);
    passes.push({ phase, steps });
  }
  const rec = await page.evaluate(() => { window.__gh810.running = false; return window.__gh810; });
  const jumps = rec.against.filter((j) => Math.abs(j.move) >= LIMIT);
  const result = { windows, frames: rec.frames, passes, against: rec.against.length, jumps };
  writeFileSync(path.join(OUT, "result.json"), JSON.stringify(result, null, 1));
  console.log(JSON.stringify({ ...result, jumps: jumps.length, firstJump: jumps[0] }));
  if (passes.every((p) => p.steps < 3)) failure = "the page did not scroll";
  else if (passes.some((p) => p.steps >= MAX_STEPS)) failure = `a pass never reached its end: ${JSON.stringify(passes)}`;
  if (!failure && jumps.length) failure = `${jumps.length} frame(s) moved the blocks ${LIMIT} px or more against the scroll direction`;
} catch (error) {
  failure = String(error);
} finally {
  await browser?.close().catch(() => {});
  try { process.kill(-server.pid); } catch {}
}
if (failure) { console.error(`FAIL: ${failure}`); process.exit(1); }
console.log("PASS: real wheel scrolling past expanded embeds moved no block against the scroll direction");
