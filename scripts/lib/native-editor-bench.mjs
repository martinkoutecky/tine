// Shared native WebDriver instrumentation. No application/test-only editor API.
import fs from "node:fs";
import path from "node:path";
const identity = JSON.parse(fs.readFileSync(new URL("../../src-tauri/app-identity.json", import.meta.url), "utf8"));

export const editorKind = process.env.BENCH_EDITOR || "textarea";
if (!["textarea", "cm6"].includes(editorKind)) throw new Error("BENCH_EDITOR must be textarea or cm6");
export const editorSelector = editorKind === "cm6" ? '.cm-content[contenteditable="true"]' : "textarea.block-editor";
export const summary = xs => {
  const sorted = [...xs].sort((a, b) => a - b);
  return { median: sorted[Math.floor(sorted.length / 2)] ?? null, p95: sorted[Math.ceil(sorted.length * .95) - 1] ?? null, samples: xs };
};

export function scratchProfile(out) {
  const env = {};
  for (const [key, dir] of Object.entries({ HOME: "home", XDG_DATA_HOME: "data", XDG_CONFIG_HOME: "config", XDG_CACHE_HOME: "cache", XDG_STATE_HOME: "state", XDG_RUNTIME_DIR: "runtime" })) {
    env[key] = path.join(out, dir);
    fs.mkdirSync(env[key], { recursive: true, mode: 0o700 });
  }
  // Seed only the scratch device preference, before application startup.
  const config = path.join(env.XDG_DATA_HOME, identity.identities[identity.ship].identifier);
  fs.mkdirSync(config, { recursive: true });
  fs.writeFileSync(path.join(config, "tine-settings.json"), JSON.stringify({ experimental_cm6_editor: editorKind === "cm6" }));
  return { ...env, LANG: "C.UTF-8" };
}

export async function startTyping(browser) {
  await browser.execute(selector => {
    const editor = document.querySelector(selector);
    if (!editor || document.activeElement !== editor) throw new Error("Requested editor is not focused");
    const p = window.__nativeTyping = { selector, start: performance.now(), previous: 0, frames: [], keys: [], inputs: [], paint: [], active: true };
    p.key = event => {
      if (!event.target.matches(selector) || !["x", "Backspace"].includes(event.key)) return;
      const at = performance.now(); p.keys.push(at);
      requestAnimationFrame(() => setTimeout(() => p.paint.push(performance.now() - at), 0));
    };
    p.input = event => {
      if (!event.target.matches(selector)) return;
      p.inputs.push(performance.now());
    };
    document.addEventListener("keydown", p.key, true);
    document.addEventListener("input", p.input, true);
    const tick = t => { if (!p.active) return; if (p.previous) p.frames.push(t - p.previous); p.previous = t; requestAnimationFrame(tick); };
    requestAnimationFrame(tick);
  }, editorSelector);
}

export async function stopTyping(browser) {
  // Let the final paint callback finish before collecting samples.
  await browser.executeAsync(done => requestAnimationFrame(() => setTimeout(done, 0)));
  const samples = await browser.execute(() => {
    const p = window.__nativeTyping; p.active = false;
    document.removeEventListener("keydown", p.key, true);
    document.removeEventListener("input", p.input, true);
    return { elapsed: performance.now() - p.start, frames: p.frames, keys: p.keys, inputs: p.inputs, paint: p.paint,
      focused: document.activeElement?.matches(p.selector) };
  });
  return { ...samples, hostAfter: process.platform === "linux" ? { load: fs.readFileSync("/proc/loadavg", "utf8").trim(), cpuPressure: fs.readFileSync("/proc/pressure/cpu", "utf8").trim() } : undefined };
}

export async function editorText(browser, scope = "") {
  return browser.execute(selector => {
    const editor = document.querySelector(selector);
    return editor instanceof HTMLTextAreaElement ? editor.value : [...editor.querySelectorAll(".cm-line")].map(line => line.textContent).join("\n");
  }, `${scope} ${editorSelector}`.trim());
}

export async function nativeCalibration(browser) {
  const samples = await browser.execute(() => {
    const samples = [];
    for (let run = 0; run < 3; run++) {
      let x = 0x9e3779b9 >>> 0, acc = 0;
      const start = performance.now();
      for (let i = 0; i < 20_000_000; i++) {
        x ^= x << 13; x >>>= 0; x ^= x >>> 17; x >>>= 0; x ^= x << 5; x >>>= 0;
        acc = (acc + x) >>> 0;
      }
      window.__nativeCalibrationSink = acc;
      samples.push(performance.now() - start);
    }
    return samples;
  });
  return { workMs: summary(samples), load: fs.readFileSync("/proc/loadavg", "utf8").trim(), cpuPressure: fs.readFileSync("/proc/pressure/cpu", "utf8").trim() };
}

export async function startNativeProfile(browser) {
  await browser.execute(() => {
    const p = window.__nativeProfile = { calls: {}, slow: [], transactions: [], restore: [] };
    // Optional diagnostic only: this is CM's DOM-to-view link, never used to
    // drive an edit. All measured typing still arrives via native WebDriver.
    const view = document.querySelector(".cm-content")?.cmTile?.root?.view;
    if (view) {
      const original = view.update;
      view.update = function (transactions) {
        const at = performance.now();
        const result = original.call(this, transactions);
        p.transactions.push({ docChanged: transactions.some(tr => tr.docChanged), reconfigured: transactions.some(tr => tr.reconfigured),
          effects: transactions.reduce((n, tr) => n + tr.effects.length, 0), ms: performance.now() - at, length: this.state.doc.length });
        return result;
      };
      p.restore.push(() => { view.update = original; });
    }
    for (const [object, name] of [[Element.prototype, "getBoundingClientRect"], [Range.prototype, "getBoundingClientRect"], [Range.prototype, "getClientRects"], [window, "getComputedStyle"]]) {
      const original = object[name];
      object[name] = function (...args) {
        const at = performance.now();
        const result = original.apply(this, args);
        const elapsed = performance.now() - at;
        const key = `${object === window ? "window" : object === Element.prototype ? "Element" : "Range"}.${name}`;
        const entry = p.calls[key] ??= { count: 0, ms: 0 };
        entry.count++; entry.ms += elapsed;
        if (elapsed >= 1) p.slow.push({ key, ms: elapsed, stack: new Error().stack });
        return result;
      };
      p.restore.push(() => { object[name] = original; });
    }
    window.__tineBench = true;
    window.__tineParseStats = { calls: 0, hits: 0, misses: 0 };
  });
}

export async function stopNativeProfile(browser) {
  return browser.execute(() => {
    const p = window.__nativeProfile;
    p.restore.forEach(restore => restore());
    window.__tineBench = false;
    return { calls: p.calls, slow: p.slow, transactions: p.transactions, parse: window.__tineParseStats };
  });
}

export async function identifyWebProcess(browser, driverPid) {
  const candidates = webProcessMemory(driverPid).processes;
  const ticks = pid => fs.readFileSync(`/proc/${pid}/stat`, "utf8").split(") ")[1].split(" ").slice(11, 13).reduce((n, value) => n + Number(value), 0);
  const before = candidates.map(p => ({ pid: p.pid, ticks: ticks(p.pid) }));
  // Attribute the measured browsing context to its process by CPU activity;
  // the hidden Quick Capture has a separate, otherwise idle web process.
  await browser.execute(() => { const until = performance.now() + 250; while (performance.now() < until) {} });
  const activity = before.map(p => ({ pid: p.pid, ticks: ticks(p.pid) - p.ticks })).sort((a, b) => b.ticks - a.ticks);
  if (!activity[0]?.ticks || activity[0].ticks === activity[1]?.ticks) throw new Error("Cannot identify the main WebKit process");
  return { pid: activity[0].pid, activity };
}

export function webProcessMemory(driverPid, mainPid) {
  const processes = fs.readdirSync("/proc").filter(name => /^\d+$/.test(name)).flatMap(name => {
    try {
      const status = fs.readFileSync(`/proc/${name}/status`, "utf8");
      return [{ pid: Number(name), parent: Number(status.match(/^PPid:\s+(\d+)/m)[1]), command: fs.readFileSync(`/proc/${name}/cmdline`, "utf8").replaceAll("\0", " ") }];
    } catch { return []; }
  });
  const descendants = new Set([driverPid]);
  for (let changed = true; changed;) {
    changed = false;
    for (const p of processes) if (descendants.has(p.parent) && !descendants.has(p.pid)) { descendants.add(p.pid); changed = true; }
  }
  const web = processes.filter(p => descendants.has(p.pid) && /WebKitWebProcess/.test(p.command)).map(p => {
    const raw = fs.readFileSync(`/proc/${p.pid}/smaps_rollup`, "utf8");
    return { ...p, rssKiB: Number(raw.match(/^Rss:\s+(\d+)/m)[1]), pssKiB: Number(raw.match(/^Pss:\s+(\d+)/m)[1]), raw };
  });
  if (!web.length) throw new Error("No descendant WebKit web process found");
  const main = web.find(p => p.pid === mainPid);
  return { processes: web, main: main && { pid: main.pid, rssKiB: main.rssKiB, pssKiB: main.pssKiB },
    rssKiB: web.reduce((n, p) => n + p.rssKiB, 0), pssKiB: web.reduce((n, p) => n + p.pssKiB, 0) };
}
