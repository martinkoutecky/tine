// GH #886: disposable property-heavy backlink graphs, native release binaries.
// Run under xvfb-run; TINE_APP, MEASURE_GROUPS (200/400), E2E_ARTIFACT_DIR,
// E2E_DRIVER_PORT and E2E_NATIVE_PORT select an isolated measurement.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { createHash } from "node:crypto";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";

const APP = process.env.TINE_APP;
if (!APP) throw new Error("TINE_APP must name the release binary being measured");
if (process.platform !== "linux") throw new Error("This measurement uses native Linux WebKitGTK");
const N = Number(process.env.MEASURE_GROUPS || 200);
const OUT = path.resolve(process.env.E2E_ARTIFACT_DIR || "target/artifacts/measure-886/run");
const DRIVER = Number(process.env.E2E_DRIVER_PORT || 4930);
const NATIVE = Number(process.env.E2E_NATIVE_PORT || 4931);
const QUIET = 200;
const WALKS = Number(process.env.MEASURE_WALKS || 3);
const CAP = 20_000;
const TMP = path.join(OUT, `session-${process.pid}`);
const GRAPH = path.join(TMP, "graph");
fs.mkdirSync(OUT, { recursive: true });
for (const dir of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(TMP, dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
fs.writeFileSync(path.join(GRAPH, "pages/Hub.md"), "- Hub intro\n");
for (let i = 0; i < N; i++) {
  const properties = Array.from({ length: 20 }, (_, p) => {
    const value = p === 0 ? "[[Hub]]" : [
      `Text value ${i}-${p}`, `[[Category ${p % 3}]]`, `[[Nov ${String(p + 1).padStart(2, "0")}, 2026]]`,
      String(i * 20 + p), `#tag${p % 3}`,
    ][p % 5];
    return `property-${String(p).padStart(2, "0")}:: ${value}`;
  });
  fs.writeFileSync(path.join(GRAPH, `pages/Source ${String(i).padStart(3, "0")}.md`), `${properties.join("\n")}\n\n- Short reference to [[Hub]].\n`);
}
const build = fs.existsSync(`${APP}.build.json`) ? JSON.parse(fs.readFileSync(`${APP}.build.json`, "utf8")) : {
  sourceRevision: process.env.MEASURE_REVISION || "working-tree",
  appSha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), buildInputsDirty: true,
};
if (build.appSha256 !== createHash("sha256").update(fs.readFileSync(APP)).digest("hex")) throw new Error("Binary does not match its build receipt");
const account = accountHomeFixture(TMP, OUT, { ...process.env, TINE_SOURCE_REVISION: build.sourceRevision });
const env = { ...account.env, HOME: path.join(TMP, "home"), TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"),
  XDG_CACHE_HOME: path.join(TMP, "cache"), XDG_STATE_HOME: path.join(TMP, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11", GTK_OVERLAY_SCROLLING: "0" };
const target = await startWebdriverApplication(APP, env, NATIVE);
const log = fs.openSync(path.join(OUT, "driver.log"), "w");
const wm = spawn(process.env.E2E_WINDOW_MANAGER || "openbox", ["--sm-disable"], { env, stdio: ["ignore", log, log], detached: true });
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(DRIVER, NATIVE, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: target.env, stdio: ["ignore", log, log], detached: true });
const result = { app: APP, build, groups: N, walks: WALKS, quietMs: QUIET, capMs: CAP, startLoad: os.loadavg(), steps: [], checkpoints: [] };
let browser;
try {
  await sleep(2500);
  browser = await remote({ hostname: "127.0.0.1", port: DRIVER, path: "/", logLevel: "error", connectionRetryCount: 1,
    connectionRetryTimeout: 90_000, capabilities: tauriCapabilities(APP, "default", process.platform, target.debuggerAddress) });
  await browser.setTimeout({ script: CAP + 30_000 });
  await ensureMainWindow(browser);
  await assertAppLease(account);
  await browser.setWindowRect(0, 0, 1280, 1000);
  await openPageByName(browser, "Hub", { timeout: 60_000 });
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".linked-references .references-header")), { timeout: 60_000 });
  await browser.execute(() => {
    if (!document.querySelector(".linked-references .reference-blocks")) document.querySelector(".linked-references .references-header").click();
  });
  await browser.waitUntil(() => browser.execute(n => document.querySelectorAll(".linked-references .reference-blocks").length === n, N), { timeout: 60_000 });
  // Observe the literal routed scroller. DOM writes per owner are a proxy for
  // visible group redraws, not a claim about Solid component function calls.
  result.environment = await browser.execute(() => {
    const scroller = document.querySelector(".main-content");
    const rec = window.__measure886 = { current: null, lastMutation: performance.now(), frame: performance.now(), running: true, visible: new Set() };
    rec.begin = (kind, fraction) => {
      const start = performance.now();
      rec.lastMutation = start;
      return rec.current = { kind, fraction, start, from: scroller.scrollTop, mutations: 0, groupMutations: {},
        sourceReads: 0, addedNodes: 0, removedNodes: 0, longTasks: [], frameGaps: [], lastMutationMs: 0,
        mutationSamples: [], mutationTargets: {}, commands: {}, postCommands: {}, groupContentMutations: {} };
    };
    const visibleObserver = new IntersectionObserver(entries => {
      for (const entry of entries) {
        if (entry.isIntersecting) rec.visible.add(entry.target);
        else rec.visible.delete(entry.target);
      }
    }, { root: scroller });
    document.querySelectorAll(".reference-group").forEach(group => visibleObserver.observe(group));
    const ownerOf = node => (node instanceof Element ? node : node.parentElement)?.closest(".reference-group");
    new MutationObserver(records => {
      const now = performance.now();
      rec.lastMutation = now;
      const step = rec.current;
      if (!step) return;
      step.lastMutationMs = now - step.start;
      step.mutations += records.length;
      for (const mutation of records) {
        const owner = ownerOf(mutation.target);
        const name = owner?.querySelector(".reference-page")?.textContent;
        const targetKind = `${mutation.type}:${String(mutation.target.className || mutation.target.nodeName)}:${mutation.attributeName || ""}`;
        step.mutationTargets[targetKind] = (step.mutationTargets[targetKind] || 0) + 1;
        if (name) step.groupMutations[name] = (step.groupMutations[name] || 0) + 1;
        if (name && mutation.type !== "attributes") step.groupContentMutations[name] = (step.groupContentMutations[name] || 0) + 1;
        if (step.mutationSamples.length < 24) step.mutationSamples.push({ ms: now - step.start, owner: name,
          type: mutation.type, target: String(mutation.target.className || mutation.target.nodeName),
          attribute: mutation.attributeName, added: mutation.addedNodes.length, removed: mutation.removedNodes.length });
        if (mutation.type === "childList") {
          step.addedNodes += mutation.addedNodes.length;
          step.removedNodes += mutation.removedNodes.length;
        }
      }
    }).observe(scroller, { subtree: true, childList: true, characterData: true, attributes: true });
    const longTasksSupported = PerformanceObserver.supportedEntryTypes?.includes("longtask") ?? false;
    if (longTasksSupported) new PerformanceObserver(list => {
      if (rec.current) for (const entry of list.getEntries()) rec.current.longTasks.push({ startMs: entry.startTime - rec.current.start, durationMs: entry.duration });
    }).observe({ entryTypes: ["longtask"] });
    const tick = now => {
      const gap = now - rec.frame;
      if (rec.current && gap > 50) rec.current.frameGaps.push(gap);
      rec.frame = now;
      if (rec.running) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    // Count actual native source reads; there are no writes in this journey.
    // Tauri freezes invoke/ipc; the engine fetch transport remains writable.
    // Count postMessage fallbacks separately (a failed fetch may use both).
    const nativeFetch = window.fetch;
    rec.pending = 0;
    window.fetch = async function(input, init) {
      const command = String(input).split("/").pop();
      const sourceRead = command === "get_page" || command === "get_page_by_path";
      const step = rec.current;
      if (sourceRead) { rec.pending++; if (step) step.sourceReads++; }
      if (step) step.commands[command] = (step.commands[command] || 0) + 1;
      try { return await nativeFetch.call(this, input, init); }
      finally { if (sourceRead) rec.pending--; }
    };
    const proto = Object.getPrototypeOf(window.webkit.messageHandlers.ipc);
    const nativePost = proto.postMessage;
    proto.postMessage = function(message) {
      const step = rec.current;
      if (step) {
        const command = JSON.parse(message).cmd;
        step.postCommands[command] = (step.postCommands[command] || 0) + 1;
      }
      return nativePost.call(this, message);
    };
    return { ua: navigator.userAgent, timeOrigin: performance.timeOrigin, longTasksSupported, viewport: scroller.clientHeight,
      expandedGroups: document.querySelectorAll('.reference-group-disclosure[aria-expanded="true"]').length,
      propertyRows: document.querySelectorAll(".page-property-reference-row").length };
  });
  const measure = async (kind, fraction = null) => {
    result.pendingStep = { kind, fraction, startedAt: new Date().toISOString() };
    fs.writeFileSync(path.join(OUT, "result.json"), JSON.stringify(result, null, 2));
    let watchdog;
    const unresponsive = new Promise((_, reject) => { watchdog = setTimeout(() => {
      try { process.kill(-driver.pid, "SIGKILL"); } catch {}
      reject(new Error(`Native page stopped answering during ${kind} (${CAP + 10_000} ms external deadline)`));
    }, CAP + 10_000); });
    let step;
    try { step = await Promise.race([browser.executeAsync((kind, fraction, quiet, cap, done) => {
      const rec = window.__measure886, s = document.querySelector(".main-content");
      const step = rec.current ?? rec.begin(kind, fraction);
      const start = step.start;
      if (kind === "viewport") s.scrollTop += s.clientHeight * 0.9;
      else if (fraction !== null) s.scrollTop = fraction * (s.scrollHeight - s.clientHeight);
      const check = () => {
        const now = performance.now(), elapsed = now - start;
        if (step.visibleReadyMs === undefined && rec.visible.size && [...rec.visible].every(group =>
          group.querySelector(".live-ref-group .ls-block"))) step.visibleReadyMs = elapsed;
        // IO and RAF run during the quiet interval; pending native reads must
        // drain too. Report both quiet confirmation and the last DOM write.
        if (elapsed >= cap || (elapsed >= quiet && now - rec.lastMutation >= quiet && rec.pending === 0)) {
          step.timeToQuietMs = elapsed;
          step.timedOut = elapsed >= cap;
          step.to = s.scrollTop;
          step.height = s.scrollHeight;
          step.atEnd = s.scrollTop + s.clientHeight >= s.scrollHeight - 2;
          step.groupsMutated = Object.keys(step.groupMutations).length;
          step.groupsContentMutated = Object.keys(step.groupContentMutations).length;
          step.maxFrameGapMs = Math.max(0, ...step.frameGaps);
          rec.current = null;
          // Snapshot after stopping the measured clock; no full-list scan on
          // each frame or mutation callback.
          step.mountedGroups = [...s.querySelectorAll(".live-ref-group")].filter(el => el.querySelector(".ls-block")).length;
          done(step);
        } else setTimeout(check, 25);
      };
      setTimeout(check, 25);
    }, kind, fraction, QUIET, CAP), unresponsive]); }
    finally { clearTimeout(watchdog); }
    delete result.pendingStep;
    result.steps.push(step);
    fs.writeFileSync(path.join(OUT, "result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify({ step: result.steps.length, kind, quiet: Math.round(step.timeToQuietMs), lastWrite: Math.round(step.lastMutationMs), mutations: step.mutations, groups: step.groupsMutated, reads: step.sourceReads, atEnd: step.atEnd }));
    if (step.timedOut) throw new Error(`step ${result.steps.length} did not settle within ${CAP} ms`);
    return step;
  };
  await measure("initial", 0);
  result.baselineAt = new Date().toISOString();
  await sleep(1500); // allow the independent 1 Hz PSS sampler a membership baseline
  for (const fraction of [0.9, 0.3, 0.7]) await measure("cold-jump", fraction);
  await measure("cold-return", 0);
  for (let walk = 1; walk <= WALKS; walk++) {
    if (walk > 1) await measure("walk-start", 0);
    for (let i = 0; i < 3000; i++) if ((await measure("viewport")).atEnd) break;
    result.checkpoints.push({ walk, settledAt: new Date().toISOString(), step: result.steps.length });
    fs.writeFileSync(path.join(OUT, "result.json"), JSON.stringify(result, null, 2));
    await sleep(1500); // sample settled memory without forcing collection
  }
  for (const fraction of [0.1, 0.85, 0.35, 1, 0]) await measure("large-jump", fraction);
  // Native input journeys accompany the repeatable scroll-position samples.
  // xdotool drives the real GTK scrollbar, wheel and PageDown rather than DOM
  // synthetic wheel events (which have no browser default action).
  const { execFileSync } = await import("node:child_process");
  const rect = await browser.execute(() => {
    const r = document.querySelector(".main-content").getBoundingClientRect();
    return { x: Math.round(r.right - 5), y: Math.round(r.top + 15), bottom: Math.round(r.bottom - 20), middle: Math.round(r.left + r.width / 2) };
  });
  await browser.execute(() => {
    document.querySelector(".reference-page")?.focus({ preventScroll: true });
    window.__measure886.begin("native-wheel-keyboard", null);
  });
  execFileSync("xdotool", ["mousemove", String(rect.middle), String(rect.y + 50), "click", "5", "key", "Next"]);
  const wheelStep = await measure("native-wheel-keyboard");
  if (wheelStep.from === wheelStep.to) throw new Error("Native wheel/PageDown did not move the routed scroller");
  await measure("drag-start", 0);
  result.nativeGeometry = rect;
  await browser.performActions([{ type: "pointer", id: "scrollbar", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, x: rect.x, y: rect.y }, { type: "pause", duration: 350 },
  ] }]);
  await browser.execute(() => window.__measure886.begin("native-scrollbar-drag", null));
  await browser.performActions([{ type: "pointer", id: "scrollbar", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerDown", button: 0 }, { type: "pointerMove", duration: 500, x: rect.x, y: rect.bottom }, { type: "pointerUp", button: 0 },
  ] }]);
  await browser.releaseActions();
  const dragStep = await measure("native-scrollbar-drag");
  if (dragStep.from === dragStep.to) throw new Error("Dragging the real scrollbar did not move the routed scroller");
  await measure("roundtrip-start", 0.35);
  const anchor = await browser.execute(() => {
    const scroller = document.querySelector(".main-content"), r = scroller.getBoundingClientRect();
    const group = [...window.__measure886.visible][0];
    return { name: group.querySelector(".reference-page").textContent, offset: group.getBoundingClientRect().top - r.top,
      fraction: scroller.scrollTop / (scroller.scrollHeight - scroller.clientHeight) };
  });
  await measure("roundtrip-away", 1);
  await measure("roundtrip-return", anchor.fraction);
  result.anchorRoundtrip = await browser.execute(anchor => {
    const group = [...document.querySelectorAll(".reference-group")].find(group => group.querySelector(".reference-page").textContent === anchor.name);
    const offset = group.getBoundingClientRect().top - document.querySelector(".main-content").getBoundingClientRect().top;
    return { ...anchor, returnedOffset: offset, delta: offset - anchor.offset };
  }, anchor);
  if (Math.abs(result.anchorRoundtrip.delta) > 1) throw new Error(`Unchanged round trip displaced the visible reference: ${JSON.stringify(result.anchorRoundtrip)}`);
  result.final = await browser.execute(() => {
    const rec = window.__measure886; rec.running = false;
    return { groups: document.querySelectorAll(".reference-group").length,
      propertyRows: document.querySelectorAll(".page-property-reference-row").length,
      propertyText: document.querySelector(".reference-blocks")?.textContent.slice(0, 1500) };
  });
  // WebKitGTK's WebDriver screenshot can time out after a long scroll even
  // while execute/paint remain responsive. Capture the actual X11 display.
  execFileSync("import", ["-window", "root", path.join(OUT, "final.png")]);
} catch (error) {
  result.failure = String(error);
  await Promise.race([browser?.saveScreenshot(path.join(OUT, "failure.png")).catch(() => {}), sleep(2000)]);
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  process.exitCode = 1;
} finally {
  try { account.verify(); } catch (error) { result.failure = `${result.failure || ""} ${error}`; process.exitCode = 1; }
  account.close();
  result.endLoad = os.loadavg();
  fs.writeFileSync(path.join(OUT, "result.json"), JSON.stringify(result, null, 2));
  if (!result.failure) await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  stopWebdriverApplication(target);
  try { process.kill(-wm.pid, "SIGKILL"); } catch {}
}
if (result.failure) {
  fs.writeFileSync(path.join(OUT, "failure-capsule.json"), JSON.stringify({ testedCommit: build.sourceRevision,
    build, journey: { pending: result.pendingStep, completedSteps: result.steps.length,
      lastKind: result.steps.at(-1)?.kind, lastTimeMs: result.steps.at(-1)?.timeToQuietMs },
    expected: "The routed property-heavy Hub settles after each viewport or large scroll jump",
    observed: result.failure, classification: /owner|lease/i.test(result.failure) ? "harness" : "ambiguous",
    evidence: ["result.json", "driver.log", "failure.png (if the WebView still answered)"] }, null, 2));
  console.error(result.failure);
}
