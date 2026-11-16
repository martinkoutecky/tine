// One isolated app session for scripts/bench-storage-ab.mjs: a private graph copy,
// private HOME/XDG dirs (never ~/.local/share/page.tine.* or ~/.config/page.tine.*),
// its own tauri-driver + WebKitWebDriver on free ports, the publish-signal adapter,
// and the UI helpers every scenario shares.
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import net from "node:net";
import { spawn, spawnSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { Adapter } from "./bench-ab-adapters.mjs";
import { openPageByName } from "./e2e-navigation.mjs";
import { describe, typedPrefixIn } from "./bench-ab-stats.mjs";

const ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789";

async function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const value = server.address().port;
      server.close(() => resolve(value));
    });
  });
}

async function waitForListening(port, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const open = await new Promise((resolve) => {
      const socket = net.connect(port, "127.0.0.1");
      socket.once("connect", () => { socket.destroy(); resolve(true); });
      socket.once("error", () => resolve(false));
    });
    if (open) return;
    if (Date.now() > deadline) throw new Error(`tauri-driver did not listen on ${port} within ${timeoutMs} ms`);
    await sleep(50);
  }
}

/** The tauri-driver binary: $TAURI_DRIVER, $CARGO_HOME/bin, a sibling .toolchain, else PATH. */
export function findTauriDriver(root) {
  if (process.env.TAURI_DRIVER) return process.env.TAURI_DRIVER;
  if (process.env.CARGO_HOME && fs.existsSync(path.join(process.env.CARGO_HOME, "bin/tauri-driver"))) {
    return path.join(process.env.CARGO_HOME, "bin/tauri-driver");
  }
  for (let dir = root, i = 0; i < 4; i++, dir = path.dirname(dir)) {
    const candidate = path.join(dir, ".toolchain/cargo/bin/tauri-driver");
    if (fs.existsSync(candidate)) return candidate;
  }
  return "tauri-driver";
}

export const mib = (bytes) => (bytes == null ? null : bytes / 1048576);

/** Main-process pid of the app launched on `graph` from `binary`, or null. */
export function appPid(binary, graph) {
  const target = fs.realpathSync(binary);
  for (const pid of fs.readdirSync("/proc").filter((item) => /^\d+$/.test(item))) {
    try {
      if (fs.realpathSync(`/proc/${pid}/exe`) !== target) continue;
      if (!fs.readFileSync(`/proc/${pid}/environ`, "utf8").includes(`TINE_GRAPH=${graph}\0`)) continue;
      return Number(pid);
    } catch { /* process exited */ }
  }
  return null;
}

function rssKiB(pid) {
  try {
    return Number(fs.readFileSync(`/proc/${pid}/status`, "utf8").match(/^VmRSS:\s+(\d+)\s+kB/m)?.[1]);
  } catch { return NaN; }
}

/** RSS in bytes of the app's main process and of its whole process tree (the
 *  webview's helper processes are children of the main process). */
export function rssOf(pid) {
  if (!pid) return { main: null, tree: null };
  const children = new Map();
  for (const entry of fs.readdirSync("/proc").filter((item) => /^\d+$/.test(item))) {
    try {
      const stat = fs.readFileSync(`/proc/${entry}/stat`, "utf8");
      const ppid = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
      if (!children.has(ppid)) children.set(ppid, []);
      children.get(ppid).push(Number(entry));
    } catch { /* gone */ }
  }
  let tree = 0;
  const walk = (p) => { const k = rssKiB(p); if (Number.isFinite(k)) tree += k * 1024; for (const c of children.get(p) ?? []) walk(c); };
  walk(pid);
  const main = rssKiB(pid) * 1024;
  return { main: Number.isFinite(main) ? main : null, tree: tree || null };
}

export class Session {
  constructor({ arm, group, run, corpusDir, outDir, repoRoot, tauriDriver, webDriver, timeoutMs, extraEnv = {} }) {
    Object.assign(this, { arm, group, run, corpusDir, repoRoot, tauriDriver, webDriver, timeoutMs, extraEnv });
    this.dir = path.join(outDir, "trials", `${group}-${arm.id}-${String(run).padStart(2, "0")}`);
    this.graph = path.join(this.dir, "graph");
    this.xdg = path.join(this.dir, "xdg");
    this.home = path.join(this.dir, "home");
    this.eventsFile = path.join(this.dir, "events.ndjson");
    this.metrics = {};
    this.series = {};
    this.notes = {};
    this.failures = {};
    this.adapter = new Adapter(arm.adapter, this.eventsFile);
    this.browser = null;
    this.td = null;
    this.timedOut = false;
  }

  // -- bookkeeping ---------------------------------------------------------
  m(name, value) { if (Number.isFinite(value)) this.metrics[name] = value; }
  addSeries(name, values) { const v = values.filter(Number.isFinite); if (v.length) this.series[name] = (this.series[name] ?? []).concat(v); }
  note(key, value) { this.notes[key] = value; }
  /** Run one named step; a failure is recorded, never thrown, so later steps still run. */
  async step(name, fn) {
    try { await fn(); } catch (error) {
      this.failures[name] = String(error?.stack ?? error).slice(0, 600);
      this.shot(`failure-${name}`);
      return false;
    }
    return true;
  }
  shot(name) {
    try { spawnSync("import", ["-window", "root", path.join(this.dir, `${name.replace(/[^\w.-]/g, "_")}.png`)], { env: this.env, timeout: 5000 }); } catch { /* best effort */ }
  }

  // -- lifecycle -----------------------------------------------------------
  /** Fresh trial directory with a private copy of the corpus (never the original). */
  prepare({ keepGraph = false } = {}) {
    if (!keepGraph) {
      fs.rmSync(this.dir, { recursive: true, force: true });
      fs.mkdirSync(this.dir, { recursive: true });
      fs.cpSync(this.corpusDir, this.graph, { recursive: true });
      fs.rmSync(path.join(this.graph, ".ab-bench-ready"), { force: true });
      for (const name of ["data", "config", "cache", "state"]) fs.mkdirSync(path.join(this.xdg, name), { recursive: true });
      fs.mkdirSync(this.home, { recursive: true });
    }
    this.env = {
      ...process.env, ...this.adapter.env(), ...this.extraEnv,
      HOME: this.home, TINE_GRAPH: this.graph,
      XDG_DATA_HOME: path.join(this.xdg, "data"), XDG_CONFIG_HOME: path.join(this.xdg, "config"),
      XDG_CACHE_HOME: path.join(this.xdg, "cache"), XDG_STATE_HOME: path.join(this.xdg, "state"),
      WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
    };
  }

  /** Launch the app; resolves with ms from the launch request to the first painted page. */
  async launch({ tag = "launch" } = {}) {
    // The driver's first session request is sometimes dropped (UND_ERR_SOCKET) while the
    // machine is loaded; nothing has run yet then, so a fresh driver is a clean retry and the
    // timing starts at the attempt that connected. Anything else is a real failure.
    let started;
    for (let attempt = 1; ; attempt++) {
      this.driverPort = await freePort();
      this.nativePort = await freePort();
      this.logFd = fs.openSync(path.join(this.dir, `driver-${tag}${attempt > 1 ? `-try${attempt}` : ""}.log`), "w");
      this.td = spawn(process.env.DBUS_RUN_SESSION || "dbus-run-session",
        ["--", this.tauriDriver, "--port", String(this.driverPort), "--native-port", String(this.nativePort), "--native-driver", this.webDriver],
        { env: this.env, stdio: ["ignore", this.logFd, this.logFd], detached: true });
      await waitForListening(this.driverPort);
      started = performance.now();
      this.launchPerf0 = started;
      this.launchStartedEpoch = Date.now();
      try {
        this.browser = await remote({ hostname: "127.0.0.1", port: this.driverPort, path: "/", logLevel: "silent",
          connectionRetryCount: 1, connectionRetryTimeout: 120000,
          capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: this.arm.binary } } });
        break;
      } catch (error) {
        if (attempt >= 3 || !/UND_ERR_SOCKET/.test(String(error))) throw error;
        this.launchRetries = (this.launchRetries ?? 0) + 1;
        this.kill();
        await sleep(1000);
      }
    }
    if (this.launchRetries) this.note("launchRetries", String(this.launchRetries));
    this.probeMode = await this.adapter.install(this.browser);
    await this.waitForFirstPage();
    await this.paint();
    return performance.now() - started;
  }

  async waitForFirstPage() {
    const deadline = Date.now() + this.timeoutMs - 10000;
    let startupError = "";
    while (Date.now() < deadline) {
      const state = await this.browser.execute(() => ({
        ready: !!document.querySelector(".ls-block, .page-title"),
        pageError: document.querySelector(".page-load-error")?.textContent?.trim() || "",
      }));
      if (state.ready) return;
      if (state.pageError) startupError = state.pageError;
      await sleep(50);
    }
    throw new Error(startupError ? `startup page error: ${startupError}` : "first page did not render");
  }

  async paint() {
    // A timer queued inside rAF runs after that rendering opportunity.
    await this.browser.executeAsync((done) => requestAnimationFrame(() => setTimeout(done, 0)));
  }

  /** Hard-kill the app (a crash): no orderly close, no flush. */
  kill() {
    try { if (this.td) process.kill(-this.td.pid, "SIGKILL"); } catch { /* already gone */ }
    try { if (this.logFd != null) fs.closeSync(this.logFd); } catch { /* closed */ }
    this.logFd = null;
    this.td = null;
    this.browser = null;
  }

  async stop() {
    if (this.browser && !this.timedOut) { try { await Promise.race([this.browser.deleteSession(), sleep(5000)]); } catch { /* best effort */ } }
    this.kill();
    await sleep(250);
  }

  pid() { return appPid(this.arm.binary, this.graph); }
  rss() { return rssOf(this.pid()); }

  // -- shared UI helpers -----------------------------------------------------
  async journey(name) { await this.adapter.journey(this.browser, name); }
  async records() { return this.adapter.collect(this.browser); }
  async openPage(name) { await openPageByName(this.browser, name, { timeout: 30000 }); await this.paint(); }

  /** Click the page's first block into edit mode with the caret at its end. */
  async enterFirstBlock() {
    const b = this.browser;
    await b.$(".ls-block .block-content").click();
    const editor = await b.$("textarea.block-editor");
    await editor.waitForExist({ timeout: 10000 });
    await b.keys(["End"]);
  }

  async leaveEditor() { await this.browser.keys(["Escape"]); await sleep(50); }

  /** Type token[from..to) with an absolute schedule (every `intervalMs`), so the
   *  rate does not depend on WebDriver latency. Returns the epoch ms of the first key. */
  async typeRange(token, from, to, intervalMs) {
    const t0 = performance.now();
    const first = Date.now();
    for (let i = from; i < to; i++) {
      await this.browser.keys([token[i]]);
      const wait = t0 + (i - from + 1) * intervalMs - performance.now();
      if (wait > 0) await sleep(wait);
    }
    return first;
  }

  /** Wait until every tracked save has settled and nothing new has happened for `quietMs`. */
  async settle(quietMs = 800, timeoutMs = 20000) {
    const deadline = Date.now() + timeoutMs;
    let lastCount = -1;
    let lastChange = Date.now();
    while (Date.now() < deadline) {
      const r = await this.records();
      const count = r.probe.calls.length + r.publishes.length + r.drafts.length;
      if (count !== lastCount || r.probe.inflight > 0) { lastCount = count; lastChange = Date.now(); }
      else if (Date.now() - lastChange >= quietMs) return;
      await sleep(100);
    }
  }

  /** Wait for a publish carrying the whole typed token, after `sinceMs`. */
  async waitPublished(token, sinceMs, timeoutMs = 30000) {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const r = await this.records();
      const hit = Adapter.findPublish(r, token, sinceMs);
      if (hit) return { hit, records: r };
      if (Date.now() > deadline) throw new Error(`no publish carrying ${token.length} typed characters within ${timeoutMs} ms`);
      await sleep(50);
    }
  }

  async waitDraft(token, sinceMs, timeoutMs = 30000) {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const r = await this.records();
      const hit = Adapter.findDraft(r, token, sinceMs);
      if (hit) return { hit, records: r };
      if (Date.now() > deadline) throw new Error(`no draft carrying ${token.length} typed characters within ${timeoutMs} ms`);
      await sleep(50);
    }
  }

  /** Input events (keys) recorded at or after `sinceMs`, in order. */
  keysSince(records, sinceMs) { return records.probe.keys.filter((k) => k.t >= sinceMs); }

  /** Typing latency p50/p95 and long frame gaps for `keys`, by journey. */
  typingStats(records, keys, prefix) {
    const lat = keys.map((k) => k.lat).filter(Number.isFinite);
    const d = describe(lat);
    if (d) { this.m(`${prefix}.typingP50Ms`, d.median); this.m(`${prefix}.typingP95Ms`, d.p95); this.addSeries(`${prefix}.typingMs`, lat); }
    const spans = records.saveSpans;
    const during = keys.filter((k) => spans.some((s) => k.t >= s.t0 && k.t <= s.t1)).map((k) => k.lat).filter(Number.isFinite);
    const dd = describe(during);
    this.m(`${prefix}.typingDuringSaveN`, during.length);
    if (dd) { this.m(`${prefix}.typingDuringSaveP50Ms`, dd.median); this.m(`${prefix}.typingDuringSaveP95Ms`, dd.p95); this.addSeries(`${prefix}.typingDuringSaveMs`, during); }
    // Both arms save in tens of milliseconds on these pages, so few keys land INSIDE a save. The
    // UI work a save causes (change events, refreshes) follows its end, so keys in the save span
    // plus a grace period after it are the ones a save can slow.
    const near = keys.filter((k) => spans.some((sp) => k.t >= sp.t0 && k.t <= sp.t1 + NEAR_SAVE_GRACE_MS)).map((k) => k.lat).filter(Number.isFinite);
    const dn = describe(near);
    this.m(`${prefix}.typingNearSaveN`, near.length);
    if (dn) { this.m(`${prefix}.typingNearSaveP50Ms`, dn.median); this.m(`${prefix}.typingNearSaveP95Ms`, dn.p95); this.addSeries(`${prefix}.typingNearSaveMs`, near); }
  }

  longTasks(records, journey, prefix) {
    const events = records.probe.longs.filter((e) => e.journey === journey).map((e) => e.ms);
    this.m(`${prefix}.longTaskCount`, events.length);
    this.m(`${prefix}.longTaskMaxMs`, events.length ? Math.max(...events) : 0);
  }
}

/** Keys typed up to this long after a save ends count as near it. */
export const NEAR_SAVE_GRACE_MS = 500;

export function makeToken(code, length) {
  let s = `qzx${code}`;
  for (let i = 0; s.length < length; i++) s += ALPHABET[i % ALPHABET.length];
  return s;
}

export { typedPrefixIn, os };
