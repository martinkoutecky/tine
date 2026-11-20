// Native scroll journey and frame/operation measurements for GH #1001.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { remote } from "webdriverio";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/gh1001");
const app = process.env.TINE_APP || path.join(root, process.platform === "win32" ? "target/release/tine.exe" : "target/release/tine");
const driverPort = +(process.env.E2E_DRIVER_PORT || 4710);
const nativePort = +(process.env.E2E_NATIVE_PORT || 4711);
const corpus = process.env.GH1001_CORPUS;
const page = process.env.GH1001_PAGE || "Query scroll";
const control = process.env.GH1001_CONTROL || "Plain scroll";
const customCss = corpus && process.env.GH1001_CSS !== "off";
const counts = process.env.GH1001_COUNTS !== "off";
const run = fs.mkdtempSync(path.join(os.tmpdir(), "tine-gh1001-"));
fs.mkdirSync(out, { recursive: true });
const graph = path.join(run, "graph");
if (corpus) {
  fs.cpSync(corpus, graph, { recursive: true,
    filter: file => customCss || path.relative(corpus, file) !== path.join("logseq", "custom.css") });
  // The corpus is distributed read-only. The scratch graph is a real graph.
  const makeWritable = dir => {
    fs.chmodSync(dir, 0o700);
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.isDirectory()) makeWritable(path.join(dir, entry.name));
      else if (entry.isFile()) fs.chmodSync(path.join(dir, entry.name), 0o600);
    }
  };
  makeWritable(graph);
}
for (const dir of ["logseq", "pages", "journals"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
const fixtureRows = Array.from({ length: 28 }, (_, i) =>
  `- TODO Result ${i + 1}\n${Array.from({ length: 12 }, (_, j) => `\t- Child ${i + 1}/${j + 1}\n`).join("")}`);
if (!corpus) {
  fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
  fixtureRows.forEach((rows, i) => fs.writeFileSync(path.join(graph, `pages/Source ${i + 1}.md`), rows));
  fs.writeFileSync(path.join(graph, "pages/Query scroll.md"), [
    "- Outline\n\t- Section\n\t\t- Query\n",
    ...Array.from({ length: 3 }, () => "\t\t\t- {{query (task TODO)}}\n\t\t\t  collapsed:: true\n"),
  ].join(""));
}
if (!corpus || !process.env.GH1001_CONTROL) {
  const outline = corpus ? fixtureRows : fixtureRows.map(row => row.replace("- TODO ", "- "));
  fs.writeFileSync(path.join(graph, "pages/Plain scroll.md"), outline.join(""));
}
const fixture = accountHomeFixture(run, out, {
  ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_CACHE_HOME: path.join(run, "cache"),
  APPDATA: path.join(run, "appdata"), LOCALAPPDATA: path.join(run, "localappdata"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", GDK_BACKEND: "x11",
});
const launcher = await startWebdriverApplication(app, fixture.env, nativePort, "gh1001");
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(driverPort, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: launcher.env, stdio: ["ignore", log, log], detached: process.platform !== "win32" });

function startCounters(profile) {
  const work = {};
  const count = (key, n = 1) => { work[key] = (work[key] ?? 0) + n; };
  for (const name of ["IntersectionObserver", "ResizeObserver"]) {
    window[name] = new Proxy(window[name], {
      construct(Target, [callback, ...options]) {
        return new Target((entries, observer) => {
          count(name);
          count(`${name}Entries`, entries.length);
          if (name === "IntersectionObserver") for (const { target } of entries) {
            if (target.matches(".query-group")) count("queryGroupEntries");
            if (target.matches(".block-main")) count("blockMainEntries");
          }
          callback(entries, observer);
        }, ...options);
      },
    });
  }
  const wrap = (object, name) => {
    const original = object[name];
    object[name] = function (...args) {
      count(name);
      if (!profile) return original.apply(this, args);
      const start = performance.now();
      try { return original.apply(this, args); }
      finally { count(`${name}Ms`, performance.now() - start); }
    };
  };
  wrap(Element.prototype, "getBoundingClientRect");
  wrap(window, "getComputedStyle");
  const accessors = {};
  for (const [prototype, name] of [[Element.prototype, "scrollTop"], [HTMLElement.prototype, "offsetHeight"]]) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    accessors[name] = descriptor;
    Object.defineProperty(prototype, name, { ...descriptor, get() { count(name); return descriptor.get.call(this); } });
  }
  const fetchOriginal = window.fetch;
  window.fetch = function (input, ...args) {
    const command = typeof input === "string" && input.match(/^(?:ipc:\/\/localhost|https?:\/\/ipc\.localhost)\/([^?]+)/)?.[1];
    if (command) count(`ipc:${command}`);
    return fetchOriginal.call(this, input, ...args);
  };
  const message = window.ipc?.postMessage;
  if (message) {
    window.ipc.postMessage = function (payload) {
      const command = typeof payload === "string" && JSON.parse(payload).cmd;
      if (command) count(`ipc:${command}`);
      return message.call(this, payload);
    };
  }
  window.gh1001Probe = { work, count, accessors };
}

async function scroll(browser, query) {
  return browser.executeAsync((selectQuery, done) => {
    const scroller = document.querySelector(".main-content");
    const disclosure = scroller.querySelector(".query-collapse:not(.collapsed)");
    const section = selectQuery ? disclosure.closest(".query-block") : scroller;
    const probe = window.gh1001Probe;
    const scrollTop = probe?.accessors.scrollTop ?? Object.getOwnPropertyDescriptor(Element.prototype, "scrollTop");
    const rect = Element.prototype.getBoundingClientRect;
    const y = () => scrollTop.get.call(scroller);
    const start = selectQuery ? Math.max(0, y() + rect.call(section).top - rect.call(scroller).top) : 0;
    const initial = { top: y(), height: rect.call(section).height, viewport: scroller.clientHeight,
      rows: section.querySelectorAll(".ls-block").length, work: probe && { ...probe.work } };
    scrollTop.set.call(scroller, start);
    const next = () => new Promise(requestAnimationFrame);
    const observer = probe && new MutationObserver(records => {
      for (const record of records) for (const [key, nodes] of [["rowsAdded", record.addedNodes], ["rowsRemoved", record.removedNodes]]) {
        for (const node of nodes) if (node instanceof Element) {
          const rows = Number(node.matches(".ls-block")) + node.querySelectorAll(".ls-block").length;
          if (rows) probe.count(key, rows);
        }
      }
    });
    observer?.observe(scroller, { childList: true, subtree: true });
    const onScroll = () => probe?.count("scrollEvents");
    if (probe) scroller.addEventListener("scroll", onScroll, { passive: true });
    const cleanup = () => {
      observer?.disconnect();
      scroller.removeEventListener("scroll", onScroll);
    };
    const snapshot = () => ({ ...probe?.work });
    const delta = before => Object.fromEntries(Object.entries(snapshot())
      .map(([key, value]) => [key, value - (before[key] || 0)]).filter(([, n]) => n));
    const sweep = async direction => {
      const before = snapshot();
      const frames = [];
      const peaks = {};
      const slowFrames = [];
      let moved = 0;
      let last = await next();
      for (let step = 0; step < 144; step++) {
        const previous = snapshot();
        const old = y();
        const target = Math.max(start, old + direction * 40);
        scrollTop.set.call(scroller, target);
        const actual = y();
        moved += actual - old;
        const frame = await next();
        if (actual !== old) {
          const time = frame - last;
          frames.push(time);
          const work = delta(previous);
          for (const [key, n] of Object.entries(work)) peaks[key] = Math.max(peaks[key] ?? 0, n);
          if (time > 50) slowFrames.push({ step, time, work });
        }
        last = frame;
      }
      const work = delta(before);
      frames.sort((a, b) => a - b);
      return { moved, samples: frames.length, p50: frames[Math.floor(frames.length * .5)],
        p95: frames[Math.floor(frames.length * .95)], max: frames.at(-1),
        over25: frames.filter(ms => ms > 25).length, over50: slowFrames.length, work, peaks, slowFrames };
    };
    (async () => {
      const sweeps = [await sweep(1), await sweep(-1), await sweep(1), await sweep(-1)];
      cleanup();
      done({ initial, sweeps, final: { height: rect.call(section).height, scrollHeight: scroller.scrollHeight,
        rows: section.querySelectorAll(".ls-block").length }, css: !!document.querySelector("#tine-custom-css")?.textContent,
        ua: navigator.userAgent, origin: location.origin, nativeAnchor: CSS.supports("overflow-anchor", "auto") });
    })().catch(error => { cleanup(); done({ error: String(error) }); });
  }, query);
}

let browser;
let phase = "driver";
try {
  const deadline = Date.now() + 20_000;
  while ((await fetch(`http://127.0.0.1:${driverPort}/status`).catch(() => null))?.ok !== true) {
    if (driver.exitCode !== null || Date.now() >= deadline) throw new Error("WebDriver did not start");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/",
    logLevel: "error", connectionRetryCount: 0, capabilities: tauriCapabilities(app, "gh1001", process.platform, launcher.debuggerAddress) });
  await ensureMainWindow(browser);
  await browser.setTimeout({ script: 45_000 });
  try { await browser.setWindowRect(0, 0, 1280, 800); } catch (error) { console.log(error.message); }
  await assertAppLease(fixture);
  if (counts) await browser.execute(startCounters, process.env.GH1001_PROFILE === "1");
  phase = "expanded query";
  await openPageByName(browser, page);
  await browser.waitUntil(() => browser.execute((useCorpus) => {
    const blocks = [...document.querySelectorAll(".main-content .query-block")];
    const target = useCorpus ? blocks.find(block => block.querySelector(".query-count")?.textContent === "28") : blocks[0];
    if (!target) return false;
    for (const button of document.querySelectorAll(".main-content .query-collapse")) {
      if (button.classList.contains("collapsed") === (button.closest(".query-block") === target)) button.click();
    }
    return true;
  }, !!corpus), { timeout: 25_000, timeoutMsg: "The reported query did not appear on the routed page" });
  await browser.waitUntil(() => browser.execute(() => {
    const open = document.querySelector(".main-content .query-collapse:not(.collapsed)")?.closest(".query-block");
    return open?.querySelector(".query-count")?.textContent === "28"
      && !!open.querySelector(".query-group .ls-block:not(.ref-block) .ls-block");
  }), { timeout: 20_000, timeoutMsg: "The expanded query did not show 28 results with live children" });
  const query = await scroll(browser, true);
  phase = "plain outline";
  await openPageByName(browser, control);
  await browser.waitUntil(() => browser.execute(() => {
    const scroller = document.querySelector(".main-content");
    return !!scroller?.querySelector(".ls-block") && scroller.scrollHeight > scroller.clientHeight * 2;
  }), { timeout: 20_000, timeoutMsg: "The plain outline did not become scrollable" });
  const plain = await scroll(browser, false);
  const result = { query, plain };
  fs.writeFileSync(path.join(out, "scroll.json"), JSON.stringify(result, null, 2) + "\n");
  if ([query, plain].some(surface => surface.error || !surface.sweeps.every(sweep => Math.abs(sweep.moved) >= surface.initial.viewport))) {
    throw new Error("The page did not scroll a viewport in both directions. See scroll.json.");
  }
  if (corpus && query.css !== Boolean(customCss)) throw new Error("The selected custom CSS condition did not take effect");
  console.log("PASS: the expanded query shows nested results and both routed pages scroll down and back up");
} catch (error) {
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({
    commit: process.env.TINE_SOURCE_REVISION || "working tree", journey: "GH #1001 query scroll",
    phase, expected: "Expanded query shows nested results and both routed pages scroll",
    observed: String(error), classification: phase === "driver" ? "infrastructure" : "ambiguous", log: "driver.log",
  }, null, 2) + "\n");
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  driver.kill();
  stopWebdriverApplication(launcher);
  fixture.close?.();
  fixture.verify();
  fs.closeSync(log);
}
