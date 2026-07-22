#!/usr/bin/env node

// Compare native Tauri startup, not just Chromium page rendering. Run under an
// X server, normally:
//   xvfb-run -a dbus-run-session -- node scripts/bench-startup.mjs
// with TINE_STARTUP_BASELINE and TINE_STARTUP_CANDIDATE pointing at raw binaries.
// Runs are interleaved and use fresh app-data directories but one identical,
// deterministic graph derived from the public kitchen-sink fixture.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const IMMUTABLE_BASELINE = "v0.4.7";
const baselineCache = path.join(ROOT, "test-results", "startup-baseline", IMMUTABLE_BASELINE);
const defaultBaseline = path.join(baselineCache, "root", "usr", "bin", "tine");

async function ensureDefaultBaseline() {
  if (fs.existsSync(defaultBaseline)) return;
  fs.mkdirSync(baselineCache, { recursive: true });
  const deb = path.join(baselineCache, `Tine_${IMMUTABLE_BASELINE.slice(1)}_amd64.deb`);
  if (!fs.existsSync(deb)) {
    const url = `https://github.com/martinkoutecky/tine/releases/download/${IMMUTABLE_BASELINE}/Tine_${IMMUTABLE_BASELINE.slice(1)}_amd64.deb`;
    console.log(`downloading immutable startup baseline ${url}`);
    const response = await fetch(url);
    if (!response.ok) throw new Error(`baseline download failed: HTTP ${response.status}`);
    fs.writeFileSync(deb, Buffer.from(await response.arrayBuffer()));
  }
  const extracted = path.join(baselineCache, "root");
  fs.rmSync(extracted, { recursive: true, force: true });
  const result = spawnSync("dpkg-deb", ["-x", deb, extracted], { stdio: "inherit" });
  if (result.status !== 0 || !fs.existsSync(defaultBaseline)) throw new Error("could not extract immutable v0.4.7 startup baseline");
}

if (!process.env.TINE_STARTUP_BASELINE) await ensureDefaultBaseline();
const BASELINE = path.resolve(process.env.TINE_STARTUP_BASELINE || defaultBaseline);
const CANDIDATE = path.resolve(process.env.TINE_STARTUP_CANDIDATE || path.join(ROOT, "target/release/tine"));
const RUNS = Number(process.env.TINE_STARTUP_RUNS || 8);
const OUT = path.resolve(process.env.TINE_STARTUP_ARTIFACT_DIR || path.join(ROOT, "test-results/startup"));
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const WD = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";

if (!fs.existsSync(BASELINE)) throw new Error(`baseline binary does not exist: ${BASELINE}`);
if (!fs.existsSync(CANDIDATE)) throw new Error(`candidate binary does not exist: ${CANDIDATE}`);
if (!Number.isInteger(RUNS) || RUNS < 3) throw new Error("TINE_STARTUP_RUNS must be an integer >= 3");

fs.rmSync(OUT, { recursive: true, force: true });
fs.mkdirSync(OUT, { recursive: true });
const graph = path.join(OUT, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
const kitchen = fs.readFileSync(path.join(ROOT, "src/fixtures/kitchen-sink.md"), "utf8")
  .split("\n")
  .filter((line) => !/^\s*id::/i.test(line))
  .join("\n");
for (let i = 0; i < 80; i++) fs.writeFileSync(path.join(graph, "pages", `Kitchen-${String(i).padStart(3, "0")}.md`), kitchen);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(graph, "journals", `${journal}.md`), Array.from({ length: 120 }, (_, i) => `- Startup row ${i} [[Kitchen-${String(i % 80).padStart(3, "0")}]]`).join("\n") + "\n");

async function freePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const port = server.address().port;
      server.close(() => resolve(port));
    });
  });
}

async function trial(kind, binary, index, captureFrames) {
  const dir = path.join(OUT, `${kind}-${String(index).padStart(2, "0")}`);
  const xdg = path.join(dir, "xdg");
  for (const name of ["data", "config", "cache"]) fs.mkdirSync(path.join(xdg, name), { recursive: true });
  const driverPort = await freePort();
  const nativePort = await freePort();
  const env = {
    ...process.env,
    TINE_GRAPH: graph,
    XDG_DATA_HOME: path.join(xdg, "data"), XDG_CONFIG_HOME: path.join(xdg, "config"), XDG_CACHE_HOME: path.join(xdg, "cache"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
  };
  const logPath = path.join(dir, "tauri-driver.log");
  const log = fs.openSync(logPath, "w");
  // Each trial needs its own session bus. Tine is single-instance per bus; if a
  // previous WebKit process is still winding down, a shared bus can forward the
  // next launch into that old instance and create a false ~200 ms "startup".
  const td = spawn(process.env.DBUS_RUN_SESSION || "dbus-run-session", ["--", TD, "--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", WD], {
    env, stdio: ["ignore", log, log], detached: true,
  });
  await sleep(500);

  let browser;
  const started = performance.now();
  try {
    browser = await remote({
      hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "silent", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: binary } },
    });
    const sessionMs = performance.now() - started;
    const capture = async (name) => {
      // WebKitWebDriver's screenshot endpoint can itself expose incomplete
      // backing-store tiles while WebKitGTK is painting. Capture the actual X
      // display instead: this is the frame a user (or VNC client) can see.
      const screenshot = spawnSync("import", ["-window", "root", path.join(dir, `frame-${name}.png`)], { env });
      if (screenshot.status !== 0) throw new Error(`could not capture X display for ${name}`);
      const frameState = await browser.execute(() => ({
        elapsed: performance.now(),
        readyState: document.readyState,
        fonts: document.fonts?.status ?? "unknown",
        blocks: document.querySelectorAll(".ls-block").length,
        deferred: document.querySelectorAll(".ast-deferred").length,
        editors: document.querySelectorAll("textarea.block-editor").length,
        rootText: document.querySelector("#root")?.textContent?.trim().slice(0, 160) ?? "",
      }));
      fs.writeFileSync(path.join(dir, `frame-${name}.json`), JSON.stringify(frameState, null, 2) + "\n");
    };
    if (captureFrames) await capture("session");
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
    const contentMs = performance.now() - started;
    const state = await browser.execute(() => ({
      readyState: document.readyState,
      title: document.querySelector(".page-title")?.textContent ?? null,
      blocks: document.querySelectorAll(".ls-block").length,
      text: document.querySelector("#root")?.textContent?.trim().slice(0, 160) ?? "",
    }));
    if (captureFrames) {
      await capture("content");
      await sleep(100);
      await capture("plus-100ms");
      await sleep(200);
      await capture("plus-300ms");
      await sleep(700);
      await capture("plus-1000ms");
    }
    return { kind, index, sessionMs, contentMs, state };
  } finally {
    try { await browser?.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
    await sleep(250);
  }
}

function stats(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const quantile = (q) => sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * q) - 1)];
  const middle = Math.floor(sorted.length / 2);
  const median = sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
  return { min: sorted[0], median, p95: quantile(0.95), max: sorted.at(-1), samples: sorted };
}

console.log(`Native startup A/B: ${RUNS} measured runs per binary on ${os.platform()}-${os.arch()}`);
console.log(`baseline=${BASELINE}`);
console.log(`candidate=${CANDIDATE}`);
await trial("baseline-warmup", BASELINE, 0, false);
await trial("candidate-warmup", CANDIDATE, 0, false);

const records = [];
for (let pair = 0; pair < RUNS; pair++) {
  const order = pair % 2 === 0
    ? [["baseline", BASELINE], ["candidate", CANDIDATE]]
    : [["candidate", CANDIDATE], ["baseline", BASELINE]];
  for (const [kind, binary] of order) {
    const record = await trial(kind, binary, pair + 1, false);
    records.push(record);
    console.log(`${kind.padEnd(9)} #${pair + 1}: session=${record.sessionMs.toFixed(1)}ms content=${record.contentMs.toFixed(1)}ms blocks=${record.state.blocks}`);
  }
}
console.log("capturing one early-frame sequence per binary (excluded from timings)");
await trial("baseline-visual", BASELINE, 1, true);
await trial("candidate-visual", CANDIDATE, 1, true);

const summarize = (kind, field) => stats(records.filter((r) => r.kind === kind).map((r) => r[field]));
const summary = {
  schemaVersion: 1,
  platform: `${os.platform()}-${os.arch()}`,
  graph: { pages: 80, journalBlocks: 120, source: "src/fixtures/kitchen-sink.md" },
  binaries: { baseline: BASELINE, candidate: CANDIDATE },
  baseline: { sessionMs: summarize("baseline", "sessionMs"), contentMs: summarize("baseline", "contentMs") },
  candidate: { sessionMs: summarize("candidate", "sessionMs"), contentMs: summarize("candidate", "contentMs") },
  records,
};
summary.delta = {
  sessionMedianPct: (summary.candidate.sessionMs.median / summary.baseline.sessionMs.median - 1) * 100,
  contentMedianPct: (summary.candidate.contentMs.median / summary.baseline.contentMs.median - 1) * 100,
  sessionP95Pct: (summary.candidate.sessionMs.p95 / summary.baseline.sessionMs.p95 - 1) * 100,
  contentP95Pct: (summary.candidate.contentMs.p95 / summary.baseline.contentMs.p95 - 1) * 100,
};
fs.writeFileSync(path.join(OUT, "summary.json"), JSON.stringify(summary, null, 2) + "\n");
console.log(JSON.stringify({ baseline: summary.baseline, candidate: summary.candidate, delta: summary.delta }, null, 2));
console.log(`artifacts: ${OUT}`);
const regressed = Object.values(summary.delta).some((delta) => delta > 30);
if (regressed) {
  console.error("REGRESSED: a native startup median/p95 exceeds the immutable v0.4.7 baseline by more than 30%");
  process.exitCode = 1;
}
