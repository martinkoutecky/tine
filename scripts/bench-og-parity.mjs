#!/usr/bin/env node
// Native Linux parity bench. Runs both copied binaries through tauri-driver in
// separate session buses; every measured trial has a private copy of its graph.
import fs from "node:fs";
import crypto from "node:crypto";
import path from "node:path";
import os from "node:os";
import net from "node:net";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { generateRealisticGraph } from "./generate-realistic-graph.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const OUT = path.resolve(process.env.TINE_OG_BENCH_OUT || path.join(ROOT, "test-results/og-bench"));
const OG = path.resolve(process.env.TINE_OG_BENCH_OG || path.join(ROOT, "target/release/tine"));
const MASTER = path.resolve(process.env.TINE_OG_BENCH_MASTER || path.join(os.homedir(), "research/tine"));
const MASTER_BUILD = path.resolve(process.env.TINE_OG_BENCH_MASTER_BUILD_JSON || path.join(os.homedir(), "research/tine.build.json"));
const ANON = path.resolve(process.env.TINE_OG_BENCH_ANON || path.join(os.homedir(), "research/logseq-anonymized"));
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const WD = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
const RUNS = Number(process.env.TINE_OG_BENCH_RUNS || 5);
const TRIAL_TIMEOUT_OVERRIDE = process.env.TINE_OG_BENCH_TRIAL_TIMEOUT_MS;
const SUMMARY_ONLY = process.env.TINE_OG_BENCH_SUMMARIZE_ONLY === "1";
const MARKER = "ogbenchneedle543";
if (process.platform !== "linux") throw new Error("bench-og-parity is Linux only");
if (!process.env.DISPLAY && !SUMMARY_ONLY) throw new Error("start under xvfb-run -a");
if (!Number.isInteger(RUNS) || RUNS < (process.env.TINE_OG_BENCH_PILOT === "1" ? 1 : 5)) throw new Error("stability budget requires at least 5 runs");
for (const file of [OG, MASTER]) if (!fs.existsSync(file)) throw new Error(`missing binary: ${file}`);
if (!fs.existsSync(ANON)) throw new Error(`missing anonymized graph: ${ANON}`);
const digest = (file) => crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
const masterBuild = JSON.parse(fs.readFileSync(MASTER_BUILD, "utf8"));
// The og binary's revision comes from its deploy receipt, and only when the receipt
// matches the exact bytes being measured.
function ogRevision() {
  try {
    const receipt = JSON.parse(fs.readFileSync(`${OG}.build.json`, "utf8"));
    return receipt.appSha256 === digest(OG) ? receipt.sourceRevision : "unrecorded (receipt does not match binary)";
  } catch { return "unrecorded (no receipt)"; }
}
if (!masterBuild.sourceRevision.startsWith("ddf408c55") || digest(MASTER) !== masterBuild.appSha256) {
  throw new Error("master reference revision or binary SHA-256 does not match tine.build.json");
}

fs.mkdirSync(OUT, { recursive: true });
const binaries = {};
const binarySha256 = {};
for (const [kind, source] of [["og", OG], ["master", MASTER]]) {
  const dest = path.join(OUT, "binaries", kind);
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  fs.copyFileSync(source, dest);
  fs.chmodSync(dest, 0o755);
  binaries[kind] = dest;
  binarySha256[kind] = digest(dest);
}

const allCorpora = {
  "2k": { pages: 1400, journals: 600 },
  "10k": { pages: 7000, journals: 3000 },
  anonymized: null,
};
const selectedCorpora = process.env.TINE_OG_BENCH_CORPORA?.split(",") || Object.keys(allCorpora);
const corpora = Object.fromEntries(selectedCorpora.map((name) => {
  if (!(name in allCorpora)) throw new Error(`unknown corpus ${name}`);
  return [name, allCorpora[name]];
}));
for (const [name, shape] of Object.entries(corpora)) {
  const dest = path.join(OUT, "corpora", name);
  if (fs.existsSync(path.join(dest, ".og-bench-ready"))) continue;
  fs.mkdirSync(dest, { recursive: true });
  if (shape) await generateRealisticGraph({ root: dest, ...shape, seed: 543 });
  else fs.cpSync(ANON, dest, { recursive: true });
  fs.mkdirSync(path.join(dest, "pages"), { recursive: true });
  fs.writeFileSync(path.join(dest, "pages/Bench Hub.md"), `- ${MARKER} editable block\n`);
  fs.writeFileSync(path.join(dest, "pages/Bench Save.md"), "- save target\n");
  for (let i = 0; i < 200; i++) {
    fs.writeFileSync(path.join(dest, "pages", `Bench Ref ${String(i).padStart(3, "0")}.md`),
      `- linked [[Bench Hub]]\n- unlinked Bench Hub mention ${i}\n`);
  }
  fs.writeFileSync(path.join(dest, ".og-bench-ready"), "seed=543; fixture refs=200\n");
}

async function port() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const value = server.address().port;
      server.close(() => resolve(value));
    });
  });
}

function quantile(values, fraction) {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)];
}
function stats(values) {
  const sorted = [...values].sort((a, b) => a - b);
  return { median: sorted[Math.floor(sorted.length / 2)], min: sorted[0], max: sorted.at(-1), spread: sorted.at(-1) - sorted[0], samples: values };
}
async function paint(browser) {
  // A timer queued inside rAF runs after that rendering opportunity. A second
  // rAF would add an entire extra frame to the typing latency budget.
  await browser.executeAsync((done) => requestAnimationFrame(() => setTimeout(done, 0)));
}

async function startProbe(browser) {
  return browser.execute(() => {
    const p = { mode: "raf-gap", journey: "open", events: [], typing: [], last: performance.now() };
    const support = typeof PerformanceObserver === "undefined" ? [] : PerformanceObserver.supportedEntryTypes ?? [];
    if (support.includes("longtask")) {
      p.mode = "longtask";
      const observer = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) if (entry.duration > 100) p.events.push({ journey: p.journey, ms: entry.duration });
      });
      observer.observe({ entryTypes: ["longtask"] });
      p.observer = observer;
    } else {
      const tick = (now) => {
        const gap = now - p.last;
        if (gap > 100) p.events.push({ journey: p.journey, ms: gap });
        p.last = now;
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    }
    document.addEventListener("input", (event) => {
      if (!(event.target instanceof HTMLTextAreaElement) || !event.target.classList.contains("block-editor")) return;
      const at = performance.now();
      p.lastEditEpoch = Date.now();
      requestAnimationFrame(() => setTimeout(() => p.typing.push(performance.now() - at), 0));
    }, true);
    window.__ogBenchProbe = p;
    return p.mode;
  });
}
async function journey(browser, name) {
  await browser.execute((value) => { window.__ogBenchProbe.journey = value; window.__ogBenchProbe.last = performance.now(); }, name);
}
async function probeResults(browser) {
  return browser.execute(() => ({
    mode: window.__ogBenchProbe.mode,
    events: window.__ogBenchProbe.events,
    typing: window.__ogBenchProbe.typing,
    lastEditEpoch: window.__ogBenchProbe.lastEditEpoch,
  }));
}
function rssFor(binary, graph) {
  const target = fs.realpathSync(binary);
  for (const pid of fs.readdirSync("/proc").filter((item) => /^\d+$/.test(item))) {
    try {
      if (fs.realpathSync(`/proc/${pid}/exe`) !== target) continue;
      if (!fs.readFileSync(`/proc/${pid}/environ`, "utf8").includes(`TINE_GRAPH=${graph}\0`)) continue;
      const status = fs.readFileSync(`/proc/${pid}/status`, "utf8");
      const kib = Number(status.match(/^VmRSS:\s+(\d+)\s+kB/m)?.[1]);
      if (Number.isFinite(kib)) return kib * 1024;
    } catch { /* process exited */ }
  }
  return null;
}
async function openSwitcher(browser, query) {
  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout: 10000 });
  await input.setValue(query);
  return input;
}
// Open an EXISTING page from the switcher: wait until the highlighted row is exactly
// that page's row (never the Create row, which can lead before search results land),
// the caller presses Enter. Checked in one round trip; no element handle held across re-renders.
async function waitForActivePageRow(browser, name) {
  await browser.waitUntil(async () => browser.execute((wanted) => {
    const row = document.querySelector('.switcher-row[role="option"].active');
    return !!row && ["page", "journal"].includes(row.querySelector(".switcher-kind")?.textContent.trim())
      && row.querySelector(".switcher-name")?.textContent.trim() === wanted;
  }, name), { timeout: 30000, interval: 50 });
}
async function trial(kind, corpus, index) {
  const trialTimeoutMs = TRIAL_TIMEOUT_OVERRIDE ? Number(TRIAL_TIMEOUT_OVERRIDE) : corpus === "10k" ? 120000 : 60000;
  const dir = path.join(OUT, "trials", `${corpus}-${kind}-${String(index).padStart(2, "0")}`);
  const graph = path.join(dir, "graph");
  fs.mkdirSync(dir, { recursive: true });
  fs.cpSync(path.join(OUT, "corpora", corpus), graph, { recursive: true, force: true });
  const xdg = path.join(dir, "xdg");
  for (const name of ["data", "config", "cache"]) fs.mkdirSync(path.join(xdg, name), { recursive: true });
  const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(xdg, "data"), XDG_CONFIG_HOME: path.join(xdg, "config"), XDG_CACHE_HOME: path.join(xdg, "cache"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
  const driverPort = await port();
  const nativePort = await port();
  const log = fs.openSync(path.join(dir, "driver.log"), "w");
  const td = spawn(process.env.DBUS_RUN_SESSION || "dbus-run-session", ["--", TD, "--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", WD], { env, stdio: ["ignore", log, log], detached: true });
  let browser;
  const result = { kind, corpus, index, loadAvgBefore: os.loadavg()[0], metrics: {}, longTasks: {}, journeyFailures: {}, failure: null };
  await sleep(500);
  const started = performance.now();
  const runJourney = async () => {
    browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "silent", connectionRetryCount: 1, connectionRetryTimeout: 120000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: binaries[kind] } } });
    result.probeMode = await startProbe(browser);
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 120000 });
    await paint(browser);
    result.metrics.openMs = performance.now() - started;

    await journey(browser, "search");
    let t = performance.now();
    await openSwitcher(browser, MARKER);
    await browser.$(".switcher-row.block-result").waitForExist({ timeout: 30000 });
    await paint(browser);
    result.metrics.searchMs = performance.now() - t;
    // The first graph-wide search cannot finish before the background load.
    // Sample here so rssAfterOpenBytes describes a loaded graph while openMs
    // still measures first paint and searchMs still measures search at launch.
    result.metrics.rssAfterOpenBytes = corpus === "10k" ? rssFor(binaries[kind], graph) : null;
    await browser.keys(["Escape"]);

    await journey(browser, "openPage");
    await openSwitcher(browser, "Bench Hub");
    await waitForActivePageRow(browser, "Bench Hub");
    t = performance.now();
    await browser.keys(["Enter"]);
    await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Bench Hub", { timeout: 30000 });
    await paint(browser);
    result.metrics.openPageMs = performance.now() - t;

    await journey(browser, "linkedReferences");
    t = performance.now();
    await browser.waitUntil(async () => {
      const count = await browser.$(".linked-references .references-count").getText().catch(() => "");
      return Number(count) >= 200;
    }, { timeout: 60000 });
    await paint(browser);
    result.metrics.linkedReferencesMs = performance.now() - t;

    await journey(browser, "unlinkedReferences");
    t = performance.now();
    await browser.waitUntil(async () => {
      const count = await browser.$(".unlinked-references .references-count").getText().catch(() => "");
      return Number(count) >= 200;
    }, { timeout: 60000 });
    await paint(browser);
    result.metrics.unlinkedReferencesMs = performance.now() - t;

    await openSwitcher(browser, "Bench Save");
    await waitForActivePageRow(browser, "Bench Save");
    await browser.keys(["Enter"]);
    await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Bench Save", { timeout: 30000 });
    await paint(browser);
    const pageFile = path.join(graph, "pages/Bench Save.md");

    await journey(browser, "typing");
    result.stage = "typing";
    await browser.$(".ls-block .block-content").click();
    const editor = await browser.$("textarea.block-editor");
    await editor.waitForExist({ timeout: 10000 });
    for (const key of "abcdefghij") await browser.keys([key]);
    await browser.waitUntil(async () => (await probeResults(browser)).typing.length >= 10, { timeout: 10000 });
    const typing = (await probeResults(browser)).typing.slice(-10);
    result.metrics.typingP50Ms = quantile(typing, 0.5);
    result.metrics.typingP95Ms = quantile(typing, 0.95);

    await journey(browser, "save");
    result.stage = "save";
    await browser.keys(["Z"]);
    const editEpoch = (await probeResults(browser)).lastEditEpoch;
    const saveDeadline = Date.now() + 30000;
    while (Date.now() < saveDeadline) {
      if (fs.readFileSync(pageFile, "utf8").includes("abcdefghijZ")) {
        result.metrics.saveMs = Date.now() - editEpoch;
        break;
      }
      if (await browser.$(".conflict-banner").isExisting()) {
        result.journeyFailures.save = "save conflict: page changed on disk";
        break;
      }
      await sleep(100);
    }
    if (!result.metrics.saveMs && !result.journeyFailures.save) result.journeyFailures.save = "edit did not reach disk within 30s";
    result.metrics.rssAfterJourneysBytes = corpus === "10k" ? rssFor(binaries[kind], graph) : null;

    const probe = await probeResults(browser);
    result.probeMode = probe.mode;
    for (const name of ["open", "search", "openPage", "linkedReferences", "unlinkedReferences", "rename", "typing", "save"]) result.longTasks[name] = [];
    for (const event of probe.events) (result.longTasks[event.journey] ??= []).push(event.ms);
    result.longTaskMaxMs = Object.fromEntries(Object.entries(result.longTasks).map(([name, values]) => [name, values.length ? Math.max(...values) : 0]));
    result.longTaskOver500 = Object.values(result.longTaskMaxMs).some((value) => value > 500);
    result.budgetViolations = [];
    if (result.longTaskOver500) result.budgetViolations.push("UI frame gap over 500 ms");
    if (result.metrics.typingP95Ms >= 16) result.budgetViolations.push("typing p95 is not under 16 ms");
  };
  let timedOut = false;
  let watchdog;
  try {
    await Promise.race([
      runJourney(),
      new Promise((_, reject) => {
        watchdog = setTimeout(() => {
          timedOut = true;
          spawnSync("import", ["-window", "root", path.join(dir, "timeout.png")], { env, timeout: 5000 });
          reject(new Error(`trial exceeded ${trialTimeoutMs} ms`));
        }, trialTimeoutMs);
      }),
    ]);
  } catch (error) {
    result.failure = { kind: "ambiguous", stage: result.stage ?? "open", message: String(error).slice(0, 800) };
    if (corpus === "10k") result.metrics.rssAfterJourneysBytes = rssFor(binaries[kind], graph);
    if (!timedOut) spawnSync("import", ["-window", "root", path.join(dir, "failure.png")], { env, timeout: 5000 });
  } finally {
    clearTimeout(watchdog);
    if (Object.keys(result.journeyFailures).length) {
      if (!timedOut) spawnSync("import", ["-window", "root", path.join(dir, "journey-failure.png")], { env, timeout: 5000 });
    }
    if (!timedOut && browser) try { await Promise.race([browser.deleteSession(), sleep(5000)]); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
    await sleep(250);
  }
  await renameTrial(result, corpus, kind, index);
  result.longTaskMaxMs = Object.fromEntries(Object.entries(result.longTasks).map(([name, values]) => [name, values.length ? Math.max(...values) : 0]));
  result.longTaskOver500 = Object.values(result.longTaskMaxMs).some((value) => value > 500);
  result.budgetViolations = [];
  if (result.longTaskOver500) result.budgetViolations.push("UI frame gap over 500 ms");
  if (result.metrics.typingP95Ms >= 16) result.budgetViolations.push("typing p95 is not under 16 ms");
  result.loadAvgAfter = os.loadavg()[0];
  fs.writeFileSync(path.join(dir, "result.json"), JSON.stringify(result, null, 2) + "\n");
  return result;
}

async function renameTrial(result, corpus, kind, index) {
  const dir = path.join(OUT, "trials", `${corpus}-${kind}-${String(index).padStart(2, "0")}`, "rename");
  const graph = path.join(dir, "graph");
  fs.mkdirSync(dir, { recursive: true });
  fs.cpSync(path.join(OUT, "corpora", corpus), graph, { recursive: true });
  const xdg = path.join(dir, "xdg");
  for (const name of ["data", "config", "cache"]) fs.mkdirSync(path.join(xdg, name), { recursive: true });
  const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(xdg, "data"), XDG_CONFIG_HOME: path.join(xdg, "config"), XDG_CACHE_HOME: path.join(xdg, "cache"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
  const driverPort = await port();
  const nativePort = await port();
  const log = fs.openSync(path.join(dir, "driver.log"), "w");
  const td = spawn(process.env.DBUS_RUN_SESSION || "dbus-run-session", ["--", TD, "--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", WD], { env, stdio: ["ignore", log, log], detached: true });
  let browser;
  let watchdog;
  let timedOut = false;
  const trialTimeoutMs = TRIAL_TIMEOUT_OVERRIDE ? Number(TRIAL_TIMEOUT_OVERRIDE) : corpus === "10k" ? 120000 : 60000;
  await sleep(500);
  const run = async () => {
    browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "silent", connectionRetryCount: 1, connectionRetryTimeout: 120000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: binaries[kind] } } });
    await browser.$(".ls-block, .page-title").waitForExist({ timeout: 120000 });
    const mode = await startProbe(browser);
    result.probeMode ??= mode;
    await openSwitcher(browser, "Bench Hub");
    await waitForActivePageRow(browser, "Bench Hub");
    await browser.keys(["Enter"]);
    await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Bench Hub", { timeout: 30000 });
    await journey(browser, "rename");
    await browser.execute(() => {
      const title = document.querySelector("h1.page-title");
      title?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, view: window }));
    });
    await browser.$(".page-title-input").waitForExist({ timeout: 10000 });
    const started = performance.now();
    await browser.execute((name) => {
      const input = document.querySelector(".page-title-input");
      input.focus();
      input.value = name;
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    }, "Bench Hub Renamed");
    const renamed = path.join(graph, "pages/Bench Hub Renamed.md");
    const old = path.join(graph, "pages/Bench Hub.md");
    const deadline = Date.now() + 60000;
    while (Date.now() < deadline) {
      if (fs.existsSync(renamed) && !fs.existsSync(old)) {
        await paint(browser);
        result.metrics.rename200Ms = performance.now() - started;
        break;
      }
      if (await browser.$(".conflict-banner").isExisting()) {
        result.journeyFailures.rename = "save conflict: page changed on disk";
        break;
      }
      await sleep(100);
    }
    if (!result.metrics.rename200Ms && !result.journeyFailures.rename) result.journeyFailures.rename = "rename did not reach disk within 60s";
    if (corpus === "10k") {
      result.metrics.rssAfterRenameBytes = rssFor(binaries[kind], graph);
      const readings = [result.metrics.rssAfterJourneysBytes, result.metrics.rssAfterRenameBytes].filter(Number.isFinite);
      result.metrics.rssAfterJourneysBytes = readings.length ? Math.max(...readings) : null;
    }
    const probe = await probeResults(browser);
    result.longTasks.rename = probe.events.filter((event) => event.journey === "rename").map((event) => event.ms);
  };
  try {
    await Promise.race([
      run(),
      new Promise((_, reject) => {
        watchdog = setTimeout(() => {
          timedOut = true;
          spawnSync("import", ["-window", "root", path.join(dir, "timeout.png")], { env, timeout: 5000 });
          reject(new Error(`rename trial exceeded ${trialTimeoutMs} ms`));
        }, trialTimeoutMs);
      }),
    ]);
  } catch (error) {
    result.journeyFailures.rename = String(error).slice(0, 300);
    if (!timedOut) spawnSync("import", ["-window", "root", path.join(dir, "failure.png")], { env, timeout: 5000 });
  } finally {
    clearTimeout(watchdog);
    if (result.journeyFailures.rename && !timedOut) spawnSync("import", ["-window", "root", path.join(dir, "journey-failure.png")], { env, timeout: 5000 });
    if (!timedOut && browser) try { await Promise.race([browser.deleteSession(), sleep(5000)]); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    fs.closeSync(log);
    await sleep(250);
  }
}

const results = [];
if (SUMMARY_ONLY) {
  for (const corpus of Object.keys(corpora)) {
    for (let i = 1; i <= RUNS; i++) {
      for (const kind of ["og", "master"]) {
        results.push(JSON.parse(fs.readFileSync(path.join(OUT, "trials", `${corpus}-${kind}-${String(i).padStart(2, "0")}`, "result.json"), "utf8")));
      }
    }
  }
} else {
  for (const corpus of Object.keys(corpora)) {
    for (let i = 1; i <= RUNS; i++) {
      for (const kind of (i % 2 ? ["og", "master"] : ["master", "og"])) {
        const result = await trial(kind, corpus, i);
        results.push(result);
        console.log(`${corpus} ${kind} #${i}: ${result.failure ? `FAIL ${result.failure.message}` : JSON.stringify(result.metrics)}`);
      }
    }
  }
}
const metrics = ["openMs", "openPageMs", "typingP50Ms", "typingP95Ms", "saveMs", "searchMs", "linkedReferencesMs", "unlinkedReferencesMs", "rename200Ms", "rssAfterOpenBytes", "rssAfterJourneysBytes", "rssAfterRenameBytes"];
const table = ["# og vs master native parity bench", "", "Five runs per metric. Cells are median [min, max] in ms (RSS in MiB).", "", "| Corpus | Metric | og | master | og vs master |", "|---|---|---:|---:|---:|"];
const summary = {};
for (const corpus of Object.keys(corpora)) {
  summary[corpus] = {};
  for (const metric of metrics) {
    const cells = {};
    for (const kind of ["og", "master"]) {
      const values = results.filter((r) => r.corpus === corpus && r.kind === kind).map((r) => r.metrics[metric]).filter(Number.isFinite);
      cells[kind] = values.length === RUNS ? stats(values) : null;
    }
    summary[corpus][metric] = cells;
    if (metric.startsWith("rss") && corpus !== "10k") continue;
    const render = (stat, kind) => {
      if (stat) return `${(metric.startsWith("rss") ? stat.median / 1048576 : stat.median).toFixed(1)} [${(metric.startsWith("rss") ? stat.min / 1048576 : stat.min).toFixed(1)}, ${(metric.startsWith("rss") ? stat.max / 1048576 : stat.max).toFixed(1)}]`;
      const completed = results.filter((r) => r.corpus === corpus && r.kind === kind && Number.isFinite(r.metrics[metric])).length;
      return `failed (${completed}/${RUNS} samples)`;
    };
    const delta = cells.og && cells.master ? `${((cells.og.median / cells.master.median - 1) * 100).toFixed(1)}%` : "—";
    table.push(`| ${corpus} | ${metric} | ${render(cells.og, "og")} | ${render(cells.master, "master")} | ${delta} |`);
  }
}
table.push("", "## Main-thread tasks or animation-frame gaps over 100 ms", "",
  "WebKitGTK uses the animation-frame gap fallback on this runner. Values list every recorded gap over 100 ms across the five trials.", "",
  "| Corpus | Journey | og maximum and gaps (ms) | master maximum and gaps (ms) |",
  "|---|---|---|---|");
for (const corpus of Object.keys(corpora)) {
  for (const journeyName of ["open", "search", "openPage", "linkedReferences", "unlinkedReferences", "typing", "save", "rename"]) {
    const cell = (kind) => {
      const values = results.filter((trial) => trial.corpus === corpus && trial.kind === kind)
        .flatMap((trial) => trial.longTasks?.[journeyName] || []);
      return values.length ? `${Math.max(...values).toFixed(1)}; ${values.map((value) => value.toFixed(1)).join(", ")}` : "0; none";
    };
    table.push(`| ${corpus} | ${journeyName} | ${cell("og")} | ${cell("master")} |`);
  }
}
const report = { schemaVersion: 1, sourceRevisions: { og: ogRevision(), master: masterBuild.sourceRevision }, binarySha256, seed: 543, runs: RUNS, probe: "PerformanceObserver longtask when supported; otherwise rAF gap", summary, results };
fs.writeFileSync(path.join(OUT, "summary.json"), JSON.stringify(report, null, 2) + "\n");
fs.writeFileSync(path.join(OUT, "comparison.md"), table.join("\n") + "\n");
if (results.some((r) => r.failure || Object.keys(r.journeyFailures).length || r.budgetViolations?.length)) process.exitCode = 1;
