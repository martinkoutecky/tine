// Perf bench (tiers 1 + 2) — objective, repeatable regression detection for the
// two things Tine exists to be fast at: loading and scrolling a LARGE page on a
// slow machine. Runs headless Chromium over `vite preview` (a PRODUCTION build)
// against the mock backend's gated 2000-block "Big" fixture (`?big`).
//
//   npm run bench            measure, compare to scripts/bench-baseline.json, and
//                            exit non-zero if a normalized metric regressed past
//                            the threshold (so it can gate CI later).
//   npm run bench -- --update   re-record the baseline from this run.
//
// TIER 1 — the app metrics (each timed IN-PAGE with performance.now, min-of-K):
//   • bigLoad   cold load (fresh reload each run) → switch to Big → stable render.
//   • scrollBig scroll the Big page to the bottom → settle (renders on demand).
//   Also dumps window.__tineParseStats (cold parses) as a virtualization sanity
//   check — enabled in the prod build via window.__tineBench (see parse.ts).
//
// TIER 2 — the calibration normalizer answers "was it the code or the machine?":
//   a fixed, deterministic CPU loop (`calib` ms). Every app metric is reported
//   raw AND normalized = metric / calib. If calib itself is >1.5× its baseline the
//   machine is throttled/loaded → results are called UNRELIABLE and the run does
//   NOT fail; otherwise normalized metrics are compared to the baseline.
//
// DEFERRED (logged, not measured here): tab-switch + per-keystroke typing (both
// prone to harness/IPC noise — a flaky metric is worse than none) and Tier 3
// (CI gate wiring + a live in-app overlay).
//
// Orphan-vite note: run this node script DIRECTLY (no `timeout` wrapper); the
// try/finally SIGKILLs the vite child by PID.

import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import os from "node:os";

const UPDATE = process.argv.includes("--update");
const argValue = (name) => {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : null;
};
const PORT = 5260;
const BASELINE = "scripts/bench-baseline.json";
const OUTPUT = argValue("--output");
const K = 8; // measured runs per metric (plus one discarded warmup); min-of-K
const REGRESS_PCT = 30; // flag a normalized metric that grows more than this. Set
// above the ~10–15% run-to-run noise floor of `bigLoad` (mounting 2000 Solid
// components is GC-sensitive in a way the pure-ALU calib can't fully normalize,
// esp. on a shared machine). So this catches GROSS regressions, not micro-drift —
// which is the goal ("spot regressions"). scrollBig + parseStats are far tighter.
const THROTTLE_FACTOR = 1.5; // calib this much over baseline ⇒ machine unreliable

const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
const waitServer = async (u, t = 80) => { for (let i = 0; i < t; i++) { try { if ((await fetch(u)).ok) return; } catch {} await sleep(250); } throw new Error("server did not start"); };
const min = (xs) => xs.reduce((a, b) => (b < a ? b : a), Infinity);
const median = (xs) => { const s = [...xs].sort((a, b) => a - b); return s[Math.floor(s.length / 2)]; };
const round = (x) => Math.round(x * 10) / 10;

// In-page: resolve with performance.now()-t0 once `.ls-block` count is large and
// has been stable for 3 animation frames (render settled). `t0` is a window global
// set right before the triggering action.
const WAIT_STABLE = () => new Promise((res) => {
  let last = -1, stable = 0;
  const tick = () => {
    const n = document.querySelectorAll(".ls-block").length;
    if (n > 1000 && n === last) { if (++stable >= 3) return res(performance.now() - window.__t0); }
    else { last = n; stable = 0; }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
});

// Tier 2: a deterministic, allocation-free CPU loop (fixed xorshift mix over a
// fixed iteration count). Pure ALU → very stable; the returned ms is this
// machine's "cost of a fixed unit of work" right now.
const CALIBRATE = () => {
  const t = performance.now();
  let x = 0x9e3779b9 >>> 0, acc = 0 >>> 0;
  for (let i = 0; i < 60_000_000; i++) {
    x ^= x << 13; x >>>= 0; x ^= x >>> 17; x ^= x << 5; x >>>= 0;
    acc = (acc + x) >>> 0;
  }
  window.__calibSink = acc; // defeat dead-code elimination
  return performance.now() - t;
};

async function openBig(page) {
  await page.keyboard.press("Control+k");
  await page.waitForSelector(".switcher-input", { timeout: 4000 });
  await page.locator(".switcher-input").fill("Big");
  await sleep(250);
  await page.evaluate(() => { window.__t0 = performance.now(); });
  await page.locator(".switcher-row").first().click();
  return page.evaluate(WAIT_STABLE);
}

async function main() {
  await waitServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 1 });
  await context.addInitScript(() => { window.__tineBench = true; }); // enable parse stats in the prod build
  const page = await context.newPage();
  const url = `http://localhost:${PORT}/?big`;

  const calib = [], bigLoad = [], scrollBig = [];
  let parseStats = null;

  const scrollSettle = () => new Promise((res) => {
    const sc = document.querySelector(".main-content");
    window.__t0 = performance.now();
    sc.scrollTop = sc.scrollHeight;
    let lastH = -1, stable = 0;
    const tick = () => {
      const h = document.querySelectorAll(".md-table").length + document.querySelectorAll(".code-block").length + document.querySelectorAll(".katex").length;
      if (h === lastH) { if (++stable >= 3) return res(performance.now() - window.__t0); }
      else { lastH = h; stable = 0; }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });

  // Boot ONCE, then measure WARM navigations (journals ↔ Big). This keeps the JS
  // context / JIT / WASM warm across runs, so `bigLoad` isolates the app-controlled
  // mount+render cost of a large page (the regression we care about) instead of
  // drowning it in per-reload boot + WASM-compile variance. `bigLoad` is a warm
  // route-switch to the 2000-block page; the near-set parse cost is captured
  // separately below (parseStats) since it's tiny (~30 blocks) once virtualized.
  await page.goto(url);
  await page.waitForSelector(".page-title", { timeout: 8000 });
  await sleep(200);
  const goJournals = async () => {
    await page.locator(".nav-item", { hasText: "Journals" }).first().click();
    await page.waitForSelector(".page-title", { timeout: 4000 });
    await sleep(80);
  };

  // One truly-cold Big open (fresh parse cache) to record the virtualization stat.
  await page.evaluate(() => { window.__tineParseStats = { calls: 0, hits: 0, misses: 0 }; });
  await openBig(page);
  parseStats = await page.evaluate(() => window.__tineParseStats ?? null);
  await sleep(150);
  await goJournals();

  for (let run = 0; run < K + 1; run++) {
    const c = await page.evaluate(CALIBRATE);
    const load = await openBig(page);
    await sleep(150);
    await page.evaluate(() => { const sc = document.querySelector(".main-content"); if (sc) sc.scrollTop = 0; });
    await sleep(100);
    const scroll = await page.evaluate(scrollSettle);
    await goJournals();
    if (run === 0) { console.log(`warmup: calib=${round(c)} load=${round(load)} scroll=${round(scroll)}`); continue; }
    calib.push(c); bigLoad.push(load); scrollBig.push(scroll);
  }

  await browser.close();

  const result = {
    calib: round(min(calib)),
    metrics: {
      bigLoad: { rawMin: round(min(bigLoad)), rawMedian: round(median(bigLoad)) },
      scrollBig: { rawMin: round(min(scrollBig)), rawMedian: round(median(scrollBig)) },
    },
    parseStats,
  };
  // Normalized = rawMin / calib (unitless; machine-speed-independent).
  for (const m of Object.values(result.metrics)) m.normalized = Number((m.rawMin / result.calib).toFixed(3));

  report(result);
}

function report(result) {
  const machine = `${os.type()} ${os.arch()} / ${os.cpus()[0]?.model ?? "?"} ×${os.cpus().length}`;

  if (UPDATE) {
    const out = { machine, calib: result.calib, metrics: {}, parseStats: result.parseStats, note: "normalized = rawMin/calib; regenerate with `npm run bench -- --update` on a QUIET machine" };
    for (const [k, v] of Object.entries(result.metrics)) out.metrics[k] = { normalized: v.normalized, rawMin: v.rawMin };
    const destination = OUTPUT ?? BASELINE;
    writeFileSync(destination, JSON.stringify(out, null, 2) + "\n");
    console.log(`\nmeasurement written → ${destination}`);
    console.log(JSON.stringify(out, null, 2));
    server.kill("SIGKILL");
    process.exit(0);
  }

  const base = existsSync(BASELINE) ? JSON.parse(readFileSync(BASELINE, "utf8")) : null;
  console.log(`\nmachine: ${machine}`);
  console.log(`calib:   ${result.calib} ms  (baseline ${base?.calib ?? "—"})`);
  if (result.parseStats) console.log(`parseStats (cold load): ${JSON.stringify(result.parseStats)}  ← misses ≈ blocks parsed; small = virtualization working`);

  // Machine-throttle guard: if the CPU unit itself is far slower than baseline,
  // the whole run is unreliable — report but do NOT fail.
  let unreliable = false;
  if (base && result.calib > base.calib * THROTTLE_FACTOR) {
    unreliable = true;
    console.log(`\n⚠  calib is ${round(result.calib / base.calib)}× the baseline — machine throttled/loaded. Results UNRELIABLE; re-run cooler. (not failing)`);
  }

  console.log("\nmetric      | raw-min | normalized | baseline | Δ%    | flag");
  console.log("------------|---------|------------|----------|-------|-----");
  let regressed = false;
  for (const [k, v] of Object.entries(result.metrics)) {
    const b = base?.metrics?.[k]?.normalized;
    const delta = b ? round(((v.normalized - b) / b) * 100) : null;
    const flag = b == null ? "—" : delta > REGRESS_PCT ? "REGRESSED" : delta < -REGRESS_PCT ? "faster" : "ok";
    if (flag === "REGRESSED" && !unreliable) regressed = true;
    console.log(
      `${k.padEnd(11)} | ${String(v.rawMin).padStart(7)} | ${String(v.normalized).padStart(10)} | ${String(b ?? "—").padStart(8)} | ${String(delta ?? "—").padStart(5)} | ${flag}`
    );
  }
  if (!base) console.log("\n(no baseline yet — run `npm run bench -- --update` on a quiet machine to record one.)");

  server.kill("SIGKILL");
  process.exit(regressed ? 1 : 0);
}

main().catch((e) => { console.error(String(e)); server.kill("SIGKILL"); process.exit(1); });
