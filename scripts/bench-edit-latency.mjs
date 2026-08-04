#!/usr/bin/env node

// Native, semantic latency probe for GH #248. It drives the real Tauri/WebKit
// editor against synthetic graphs, schedules ordinary input work at a fixed
// cadence, and requires both the inserted bytes and the exact original bytes
// after deletion to reach disk. It intentionally measures event-loop gaps and
// persistence rather than pixels or DOM-settle time.

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = path.resolve(process.env.TINE_APP || path.join(ROOT, "target/release/tine"));
const TAURI_DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const WEBKIT_DRIVER = process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver";
const PULSES = positiveInt("TINE_ISSUE248_PULSES", 120);
const PERIOD_MS = positiveInt("TINE_ISSUE248_PERIOD_MS", 25);
const PERIODIC_PULSES = positiveInt("TINE_ISSUE248_PERIODIC_PULSES", 8);
const PERIODIC_PERIOD_MS = positiveInt("TINE_ISSUE248_PERIODIC_PERIOD_MS", 550);
const REPEATS = positiveInt("TINE_ISSUE248_REPEATS", 1);
const SMALL_GRAPH_PAGES = positiveInt("TINE_ISSUE248_SMALL_GRAPH_PAGES", 50);
const LARGE_GRAPH_PAGES = positiveInt("TINE_ISSUE248_LARGE_GRAPH_PAGES", 5_000);
const SMALL_PAGE_BLOCKS = positiveInt("TINE_ISSUE248_SMALL_PAGE_BLOCKS", 20);
const LARGE_PAGE_BLOCKS = positiveInt("TINE_ISSUE248_LARGE_PAGE_BLOCKS", 2_000);
const OUTPUT = process.env.TINE_ISSUE248_OUTPUT
  ? path.resolve(process.env.TINE_ISSUE248_OUTPUT)
  : path.join(os.tmpdir(), `tine-issue248-${new Date().toISOString().replace(/[:.]/g, "-")}.json`);

function positiveInt(name, fallback) {
  const value = process.env[name];
  if (value === undefined) return fallback;
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed <= 0) throw new Error(`${name} must be a positive integer, got ${JSON.stringify(value)}`);
  return parsed;
}

function currentJournalStem() {
  const now = new Date();
  return `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
}

function seedGraph(root, graphPages, pageBlocks) {
  const graph = path.join(root, "graph");
  const pages = path.join(graph, "pages");
  const journals = path.join(graph, "journals");
  fs.mkdirSync(pages, { recursive: true });
  fs.mkdirSync(journals, { recursive: true });
  fs.mkdirSync(path.join(graph, "logseq"), { recursive: true });
  fs.writeFileSync(path.join(graph, "logseq", "config.edn"), "{}\n");
  const target = path.join(pages, "Edit target.md");
  const targetLines = [
    "- active",
    ...Array.from({ length: pageBlocks - 1 }, (_, index) => `- filler-${String(index + 1).padStart(5, "0")} ${Array(8).fill("content").join(" ")}`),
    "",
  ];
  fs.writeFileSync(target, targetLines.join("\n"));
  for (let index = 1; index < graphPages; index += 1) {
    fs.writeFileSync(path.join(pages, `Unrelated-${String(index).padStart(5, "0")}.md`), `- unrelated graph page ${index}\n`);
  }
  fs.writeFileSync(path.join(journals, `${currentJournalStem()}.md`), "- [[Edit target]]\n");
  return { graph, target };
}

function startDisplay() {
  if (process.env.DISPLAY) return { display: process.env.DISPLAY, child: null };
  for (const display of [":98", ":99", ":100", ":101", ":102"]) {
    const child = spawn("Xvfb", [display, "-screen", "0", "1400x1000x24"], { stdio: "ignore" });
    if (child.pid) return { display, child };
  }
  throw new Error("could not start Xvfb for the native latency benchmark");
}

function stopProcessGroup(child) {
  if (!child?.pid) return;
  try { process.kill(-child.pid, "SIGKILL"); } catch {}
}

function quantile(values, q) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * q))];
}

function summarize(values) {
  if (!values.length) return { count: 0, min: null, p50: null, p95: null, max: null, samples: [] };
  return {
    count: values.length,
    min: Math.min(...values),
    p50: quantile(values, 0.5),
    p95: quantile(values, 0.95),
    max: Math.max(...values),
    samples: values,
  };
}

async function waitFor(predicate, label, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await sleep(80);
  }
  throw new Error(`timed out waiting for ${label}`);
}

async function openTarget(browser) {
  await browser.$(".page-title, .ls-block").waitForExist({ timeout: 45_000 });
  const routed = await browser.execute(() => {
    const link = [...document.querySelectorAll(".page-ref")]
      .find((node) => node.textContent?.includes("Edit target"));
    if (!link) return false;
    for (const type of ["mousedown", "mouseup", "click"]) {
      link.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0 }));
    }
    return true;
  });
  if (!routed) {
    const diagnostic = await browser.execute(() => ({
      title: document.querySelector(".page-title")?.textContent,
      pageRefs: [...document.querySelectorAll(".page-ref")].map((node) => node.textContent?.trim()),
      blocks: [...document.querySelectorAll(".ls-block")].slice(0, 8).map((node) => node.textContent?.trim()),
    }));
    throw new Error(`the synthetic journal did not expose the routed Edit target page reference: ${JSON.stringify(diagnostic)}`);
  }
  await browser.waitUntil(async () => (await browser.$(".page-title").getText()).trim() === "Edit target", {
    timeout: 30_000,
    timeoutMsg: "routed page title did not become Edit target",
  });
  const startedEditing = await browser.execute(() => {
    const content = document.querySelector(".ls-block .block-content");
    if (!content) return false;
    for (const type of ["mousedown", "mouseup", "click"]) {
      content.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0 }));
    }
    return true;
  });
  if (!startedEditing) throw new Error("the routed page did not expose a block content target for editing");
  await browser.$("textarea.block-editor").waitForExist({ timeout: 15_000 });
}

// Loading a routed 2,000-block page can still be finishing its initial mount
// when the editor first becomes present. This is pre-measurement warm-up, not a
// measured DOM-settle oracle: the user journey and every metric begin only after
// the same quiet interval in every matrix cell.
async function settleRoutedEditor() {
  await sleep(750);
}

async function installCollector(browser) {
  await browser.execute(async () => {
    const phases = Object.create(null);
    let phase = "setup";
    let running = true;
    let lastFrame = performance.now();
    const phaseData = (name) => phases[name] ??= Object.create(null);
    const record = (name, value) => {
      if (!Number.isFinite(value)) return;
      (phaseData(phase)[name] ??= []).push(value);
    };
    const frame = (now) => {
      record("animationFrameGapMs", now - lastFrame);
      lastFrame = now;
      if (running) requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
    let unlisten = () => {};
    let nativeEventBridge = "not-attempted";
    const bench = {
      begin(name) { phase = name; phaseData(name); },
      record,
      snapshot() { return { phases, nativeEventBridge }; },
      stop() { running = false; unlisten(); },
    };
    window.__tineIssue248Bench = bench;
    try {
      // WebDriver executes this function directly in the WebKit page rather
      // than through Vite's module graph, so use the same Tauri event bridge
      // that @tauri-apps/api/event's bundled `listen` uses.
      const internals = window.__TAURI_INTERNALS__;
      const eventMetrics = {
        "issue-248-backend-save-ms": "backend.persistenceMs",
        "issue-248-legacy-save-page-ms": "backend.legacySavePageMs",
      };
      const handler = internals.transformCallback((event) => {
        const metric = eventMetrics[event.event];
        if (metric) record(metric, Number(event.payload));
      });
      const listeners = await Promise.all(Object.keys(eventMetrics).map(async (event) => ({
        event,
        eventId: await internals.invoke("plugin:event|listen", {
          event,
          target: { kind: "Any" },
          handler,
        }),
      })));
      unlisten = () => {
        for (const { event, eventId } of listeners) {
          window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener(event, eventId);
          void internals.invoke("plugin:event|unlisten", { event, eventId });
        }
      };
      nativeEventBridge = "listening";
    } catch (error) {
      nativeEventBridge = `unavailable: ${error instanceof Error ? error.message : String(error)}`;
      // A browser-only run can still measure frontend phases without native events.
    }
  });
}

async function runInput(browser, phase, kind) {
  return browser.execute(async (label, operation, pulses, periodMs) => {
    const bench = window.__tineIssue248Bench;
    if (!bench) throw new Error("GH #248 benchmark collector is absent");
    bench.begin(label);
    const editor = document.querySelector("textarea.block-editor");
    if (!(editor instanceof HTMLTextAreaElement)) throw new Error("routed target editor is absent");
    const original = editor.value;
    editor.focus();
    editor.setSelectionRange(editor.value.length, editor.value.length);
    const scheduleDelayMs = [];
    const inputHandlerMs = [];
    let due = performance.now() + periodMs;
    for (let index = 0; index < pulses; index += 1) {
      const now = performance.now();
      const wait = Math.max(0, due - now);
      if (wait) await new Promise((resolve) => setTimeout(resolve, wait));
      const started = performance.now();
      scheduleDelayMs.push(Math.max(0, started - due));
      due += periodMs;
      if (operation === "insert") {
        editor.value += "1";
        editor.setSelectionRange(editor.value.length, editor.value.length);
      } else {
        if (!editor.value.endsWith("1")) throw new Error("deletion phase did not begin with the inserted suffix");
        editor.value = editor.value.slice(0, -1);
        editor.setSelectionRange(editor.value.length, editor.value.length);
      }
      const event = new InputEvent("input", {
        bubbles: true,
        inputType: operation === "insert" ? "insertText" : "deleteContentBackward",
        data: operation === "insert" ? "1" : null,
      });
      editor.dispatchEvent(event);
      inputHandlerMs.push(performance.now() - started);
    }
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    for (const value of scheduleDelayMs) bench.record("scheduledInputDelayMs", value);
    for (const value of inputHandlerMs) bench.record("inputHandlerMs", value);
    return { original, final: editor.value, scheduleDelayMs, inputHandlerMs };
  }, phase, kind, PULSES, PERIOD_MS);
}

async function phase(browser, name) {
  await browser.execute((label) => window.__tineIssue248Bench?.begin(label), name);
}

async function oneRun({ graphPages, pageBlocks, repeat, portOffset, display }) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "tine-issue248-"));
  const { graph, target } = seedGraph(root, graphPages, pageBlocks);
  const originalBytes = fs.readFileSync(target);
  const xdg = path.join(root, "xdg");
  for (const directory of ["data", "config", "cache"]) fs.mkdirSync(path.join(xdg, directory), { recursive: true });
  const driverPort = 4720 + portOffset * 2;
  const nativePort = driverPort + 1;
  const logPath = path.join(root, "tauri-driver.log");
  const log = fs.openSync(logPath, "w");
  const env = {
    ...process.env,
    DISPLAY: display,
    TINE_GRAPH: graph,
    XDG_DATA_HOME: path.join(xdg, "data"),
    XDG_CONFIG_HOME: path.join(xdg, "config"),
    XDG_CACHE_HOME: path.join(xdg, "cache"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1",
    WEBKIT_DISABLE_COMPOSITING_MODE: "1",
    LIBGL_ALWAYS_SOFTWARE: "1",
    GDK_BACKEND: "x11",
    TINE_ISSUE248_BENCH: "1",
  };
  const driver = spawn(TAURI_DRIVER, [
    "--port", String(driverPort),
    "--native-port", String(nativePort),
    "--native-driver", WEBKIT_DRIVER,
  ], { env, stdio: ["ignore", log, log], detached: true });
  let browser;
  try {
    await sleep(2_000);
    browser = await remote({
      hostname: "127.0.0.1",
      port: driverPort,
      path: "/",
      logLevel: "error",
      connectionRetryCount: 1,
      connectionRetryTimeout: 60_000,
      capabilities: {
        browserName: "wry",
        "wdio:enforceWebDriverClassic": true,
        "tauri:options": { application: APP },
      },
    });
    await openTarget(browser);
    await settleRoutedEditor();
    await installCollector(browser);
    const insert = await runInput(browser, "insert-input", "insert");
    if (insert.final !== `${insert.original}${"1".repeat(PULSES)}`) throw new Error("insert input did not leave the expected editor bytes");
    await phase(browser, "insert-persist-and-datarev");
    await waitFor(() => fs.readFileSync(target, "utf8").includes(`active${"1".repeat(PULSES)}`), "inserted bytes on disk");
    await sleep(1_200); // save debounce + dataRev debounce, without becoming a timing oracle
    const deleteResult = await runInput(browser, "delete-input", "delete");
    if (deleteResult.final !== insert.original) throw new Error("deletion input did not restore the editor bytes");
    await phase(browser, "delete-persist-and-datarev");
    try {
      await waitFor(() => fs.readFileSync(target).equals(originalBytes), "exact original bytes after deletion");
    } catch (error) {
      const actual = fs.readFileSync(target);
      throw new Error(`${error.message}; graph=${graph}; initial=${JSON.stringify(originalBytes.toString("utf8").slice(0, 1_000))}; actual=${JSON.stringify(actual.toString("utf8").slice(0, 1_000))}`);
    }
    await sleep(1_200); // allows the debounced data-revision consumers to run
    // This cadence deliberately crosses the ordinary 400 ms save debounce, so
    // the next input can be observed while the preceding editor save is in
    // flight. It separates a costly asynchronous save from an actual keystroke
    // or event-loop stall without forcing a save or changing its policy.
    const periodicInsert = await browser.execute(async (pulses, periodMs) => {
      const bench = window.__tineIssue248Bench;
      bench?.begin("periodic-insert");
      const editor = document.querySelector("textarea.block-editor");
      if (!(editor instanceof HTMLTextAreaElement)) throw new Error("routed target editor is absent for periodic insert");
      const original = editor.value;
      editor.focus();
      editor.setSelectionRange(editor.value.length, editor.value.length);
      const scheduleDelayMs = [];
      const inputHandlerMs = [];
      let due = performance.now() + periodMs;
      for (let index = 0; index < pulses; index += 1) {
        const wait = Math.max(0, due - performance.now());
        if (wait) await new Promise((resolve) => setTimeout(resolve, wait));
        const started = performance.now();
        scheduleDelayMs.push(Math.max(0, started - due));
        due += periodMs;
        editor.value += "1";
        editor.setSelectionRange(editor.value.length, editor.value.length);
        editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "1" }));
        inputHandlerMs.push(performance.now() - started);
      }
      for (const value of scheduleDelayMs) bench?.record("scheduledInputDelayMs", value);
      for (const value of inputHandlerMs) bench?.record("inputHandlerMs", value);
      return { original, final: editor.value };
    }, PERIODIC_PULSES, PERIODIC_PERIOD_MS);
    if (periodicInsert.final !== `${periodicInsert.original}${"1".repeat(PERIODIC_PULSES)}`) throw new Error("periodic insertion did not leave the expected editor bytes");
    await waitFor(() => fs.readFileSync(target, "utf8").includes(`active${"1".repeat(PERIODIC_PULSES)}`), "periodically inserted bytes on disk", 30_000);
    await sleep(1_200);
    const periodicDelete = await browser.execute(async (pulses, periodMs) => {
      const bench = window.__tineIssue248Bench;
      bench?.begin("periodic-delete");
      const editor = document.querySelector("textarea.block-editor");
      if (!(editor instanceof HTMLTextAreaElement)) throw new Error("routed target editor is absent for periodic delete");
      const original = editor.value;
      editor.focus();
      editor.setSelectionRange(editor.value.length, editor.value.length);
      const scheduleDelayMs = [];
      const inputHandlerMs = [];
      let due = performance.now() + periodMs;
      for (let index = 0; index < pulses; index += 1) {
        const wait = Math.max(0, due - performance.now());
        if (wait) await new Promise((resolve) => setTimeout(resolve, wait));
        const started = performance.now();
        scheduleDelayMs.push(Math.max(0, started - due));
        due += periodMs;
        if (!editor.value.endsWith("1")) throw new Error("periodic deletion did not begin with the inserted suffix");
        editor.value = editor.value.slice(0, -1);
        editor.setSelectionRange(editor.value.length, editor.value.length);
        editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "deleteContentBackward", data: null }));
        inputHandlerMs.push(performance.now() - started);
      }
      for (const value of scheduleDelayMs) bench?.record("scheduledInputDelayMs", value);
      for (const value of inputHandlerMs) bench?.record("inputHandlerMs", value);
      return { original, final: editor.value };
    }, PERIODIC_PULSES, PERIODIC_PERIOD_MS);
    if (periodicDelete.final !== periodicInsert.original) throw new Error("periodic deletion did not restore the editor bytes");
    await phase(browser, "periodic-delete-persist-and-datarev");
    try {
      await waitFor(() => fs.readFileSync(target).equals(originalBytes), "exact original bytes after periodic deletion", 30_000);
    } catch (error) {
      const actual = fs.readFileSync(target);
      throw new Error(`${error.message}; graph=${graph}; initial=${JSON.stringify(originalBytes.toString("utf8").slice(0, 1_000))}; actual=${JSON.stringify(actual.toString("utf8").slice(0, 1_000))}`);
    }
    await sleep(1_200);
    const collector = await browser.execute(() => {
      const bench = window.__tineIssue248Bench;
      bench?.stop();
      return bench?.snapshot() ?? { phases: {}, nativeEventBridge: "collector-absent" };
    });
    return {
      graphPages,
      pageBlocks,
      repeat,
      pulses: PULSES,
      periodMs: PERIOD_MS,
      exactPersistence: fs.readFileSync(target).equals(originalBytes),
      phases: collector.phases,
      nativeEventBridge: collector.nativeEventBridge,
      summaries: Object.fromEntries(Object.entries(collector.phases).map(([name, metrics]) => [
        name,
        Object.fromEntries(Object.entries(metrics).map(([metric, values]) => [metric, summarize(values)])),
      ])),
      artifacts: { root, target, tauriDriverLog: logPath },
    };
  } finally {
    try { await browser?.deleteSession(); } catch {}
    stopProcessGroup(driver);
    fs.closeSync(log);
  }
}

function gitRevision() {
  const result = spawnSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" });
  return result.status === 0 ? result.stdout.trim() : null;
}

const display = startDisplay();
const matrix = [
  ["small-graph/small-page", SMALL_GRAPH_PAGES, SMALL_PAGE_BLOCKS],
  ["small-graph/large-page", SMALL_GRAPH_PAGES, LARGE_PAGE_BLOCKS],
  ["large-graph/small-page", LARGE_GRAPH_PAGES, SMALL_PAGE_BLOCKS],
  ["large-graph/large-page", LARGE_GRAPH_PAGES, LARGE_PAGE_BLOCKS],
];
const results = [];
try {
  if (!fs.existsSync(APP)) throw new Error(`native app binary is missing: ${APP}`);
  for (let repeat = 1; repeat <= REPEATS; repeat += 1) {
    for (let matrixIndex = 0; matrixIndex < matrix.length; matrixIndex += 1) {
      const [cell, graphPages, pageBlocks] = matrix[matrixIndex];
      console.log(`GH #248 ${cell}, run ${repeat}/${REPEATS}: graph=${graphPages} pages, current=${pageBlocks} blocks`);
      const result = await oneRun({ graphPages, pageBlocks, repeat, portOffset: (repeat - 1) * matrix.length + matrixIndex, display: display.display });
      results.push({ cell, ...result });
      const insert = result.summaries["insert-input"]?.inputHandlerMs;
      const deleteMetric = result.summaries["delete-input"]?.inputHandlerMs;
      console.log(JSON.stringify({ cell, repeat, insertInputP95Ms: insert?.p95, deleteInputP95Ms: deleteMetric?.p95, exactPersistence: result.exactPersistence }));
    }
  }
} finally {
  stopProcessGroup(display.child);
}

const receipt = {
  schemaVersion: 1,
  issue: 248,
  sourceRevision: gitRevision(),
  app: APP,
  recordedAt: new Date().toISOString(),
  method: {
    eventLoop: "fixed-cadence in-page InputEvent dispatch plus requestAnimationFrame gaps; no pixel or DOM-settle timing",
    semanticOutcome: "routed page insert then delete through the normal editor, with inserted bytes observed on disk and exact initial bytes restored",
    limits: "synthetic InputEvent is programmatic but uses the production textarea input handler; native keyboard/device repeat cadence is deliberately not asserted",
  },
  configuration: {
    pulses: PULSES,
    periodMs: PERIOD_MS,
    periodicPulses: PERIODIC_PULSES,
    periodicPeriodMs: PERIODIC_PERIOD_MS,
    repeats: REPEATS,
    smallGraphPages: SMALL_GRAPH_PAGES,
    largeGraphPages: LARGE_GRAPH_PAGES,
    smallPageBlocks: SMALL_PAGE_BLOCKS,
    largePageBlocks: LARGE_PAGE_BLOCKS,
  },
  results,
};
fs.mkdirSync(path.dirname(OUTPUT), { recursive: true });
fs.writeFileSync(OUTPUT, `${JSON.stringify(receipt, null, 2)}\n`);
console.log(`GH #248 raw receipt → ${OUTPUT}`);
