// PDF viewer perf harness: app/session plumbing (evidence branch
// batch/og-pdf-perf-review; never integrate). Launches the real Tine binary
// through tauri-driver (as scripts/e2e-pdf-logseq.mjs does), serves the stock
// pdf.js viewer + corpus over a local HTTP server, samples /proc RSS, and
// drives xdotool input.
import { execFileSync, spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { tauriCapabilities, webdriverServerArgs } from "../e2e-capabilities.mjs";
import { ensureMainWindow } from "../lib/e2e-main-window.mjs";
import { APP_ID } from "../lib/app-identity.mjs";

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || "tauri-driver";
export const COLLECTOR = fs.readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), "collector.js"), "utf8");

// ---------------------------------------------------------------- static server
// Serves /web/* and /build/* from the pdf.js dist and /pdfs/* from the corpus,
// with HTTP Range support (the stock viewer uses range requests when offered).
export function startStaticServer(pdfjsDist, corpus) {
  const types = { ".html": "text/html", ".mjs": "text/javascript", ".js": "text/javascript", ".css": "text/css",
    ".pdf": "application/pdf", ".svg": "image/svg+xml", ".png": "image/png", ".json": "application/json",
    ".ftl": "text/plain", ".bcmap": "application/octet-stream", ".pfb": "application/octet-stream", ".ttf": "font/ttf",
    ".map": "application/json", ".wasm": "application/wasm" };
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://x");
    const p = decodeURIComponent(url.pathname);
    let file;
    if (p.startsWith("/pdfs/")) file = path.join(corpus, path.basename(p));
    else file = path.join(pdfjsDist, path.normalize(p).replace(/^(\.\.[/\\])+/, ""));
    if (!file.startsWith(pdfjsDist) && !file.startsWith(corpus)) { res.writeHead(403); return res.end(); }
    fs.stat(file, (err, st) => {
      if (err || !st.isFile()) { res.writeHead(404); return res.end(); }
      const type = types[path.extname(file)] || "application/octet-stream";
      const range = req.headers.range && /bytes=(\d*)-(\d*)/.exec(req.headers.range);
      if (range) {
        const start = range[1] ? Number(range[1]) : st.size - Number(range[2]);
        const end = range[1] && range[2] ? Math.min(Number(range[2]), st.size - 1) : st.size - 1;
        res.writeHead(206, { "Content-Type": type, "Accept-Ranges": "bytes", "Content-Range": `bytes ${start}-${end}/${st.size}`,
          "Content-Length": end - start + 1, "Cache-Control": "no-store" });
        fs.createReadStream(file, { start, end }).pipe(res);
      } else {
        res.writeHead(200, { "Content-Type": type, "Accept-Ranges": "bytes", "Content-Length": st.size, "Cache-Control": "no-store" });
        fs.createReadStream(file).pipe(res);
      }
    });
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve({ server, port: server.address().port })));
}

// ---------------------------------------------------------------- /proc RSS
function procRows() {
  const rows = [];
  for (const entry of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(entry)) continue;
    try {
      const status = fs.readFileSync(`/proc/${entry}/status`, "utf8");
      rows.push({
        pid: Number(entry),
        ppid: Number(status.match(/^PPid:\s+(\d+)$/m)?.[1] || 0),
        rss: Number(status.match(/^VmRSS:\s+(\d+)\s+kB$/m)?.[1] || 0),
        name: status.match(/^Name:\s+(.*)$/m)?.[1] || "",
      });
    } catch {}
  }
  return rows;
}
/** RSS (KiB) of the app's main process and of its WebKit web-content / network
 * processes (descendants of the app). */
export function appRss(appPid) {
  const rows = procRows();
  const byParent = new Map();
  for (const r of rows) { if (!byParent.has(r.ppid)) byParent.set(r.ppid, []); byParent.get(r.ppid).push(r); }
  const out = { main: 0, web: 0, network: 0, other: 0 };
  const main = rows.find((r) => r.pid === appPid);
  if (!main) return null;
  out.main = main.rss;
  const stack = [...(byParent.get(appPid) || [])];
  while (stack.length) {
    const r = stack.pop();
    if (/WebKitWebProces|WebKitWebProcess/.test(r.name)) out.web += r.rss;
    else if (/WebKitNetwork/.test(r.name)) out.network += r.rss;
    else out.other += r.rss;
    stack.push(...(byParent.get(r.pid) || []));
  }
  return out;
}
export function findAppPid() {
  for (const entry of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(entry)) continue;
    try { if (fs.readlinkSync(`/proc/${entry}/exe`) === APP) return Number(entry); } catch {}
  }
  return null;
}
/** Sample RSS every `ms` until stopped. */
export function rssSampler(appPid, ms = 100) {
  const samples = [];
  const timer = setInterval(() => { const r = appRss(appPid); if (r) samples.push([Date.now(), r.main, r.web, r.network]); }, ms);
  return { samples, stop: () => clearInterval(timer) };
}

export const loadavg = () => os.loadavg().map((x) => Math.round(x * 100) / 100);

// ---------------------------------------------------------------- xdotool
export const xdo = (...args) => execFileSync("xdotool", args, { encoding: "utf8", timeout: 120_000 }).trim();

// ---------------------------------------------------------------- app session
/** Launch Tine on a fresh graph containing `pdfs` (basenames in corpus) and
 * return { browser, appPid, stop }. */
export async function launchApp({ corpus, pdfs, dpr, tmp, driverPort, nativePort, cpus }) {
  fs.rmSync(tmp, { recursive: true, force: true });
  const graph = path.join(tmp, "graph");
  for (const d of ["pages", "journals", "logseq", "assets"]) fs.mkdirSync(path.join(graph, d), { recursive: true });
  for (const d of ["data", "config", "cache"]) fs.mkdirSync(path.join(tmp, "xdg", d), { recursive: true });
  fs.mkdirSync(path.join(tmp, "xdg", "data", APP_ID), { recursive: true });
  for (const name of pdfs) fs.copyFileSync(path.join(corpus, name), path.join(graph, "assets", name));
  const now = new Date();
  const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
  fs.writeFileSync(path.join(graph, "journals", `${journal}.md`),
    pdfs.map((name) => `- ![${name}](../assets/${name})`).join("\n") + "\n");
  const env = {
    ...process.env,
    TINE_GRAPH: graph,
    XDG_DATA_HOME: path.join(tmp, "xdg", "data"),
    XDG_CONFIG_HOME: path.join(tmp, "xdg", "config"),
    XDG_CACHE_HOME: path.join(tmp, "xdg", "cache"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1",
    WEBKIT_DISABLE_COMPOSITING_MODE: "1",
    LIBGL_ALWAYS_SOFTWARE: "1",
    GDK_BACKEND: "x11",
    ...(dpr === 2 ? { GDK_SCALE: "2" } : {}),
  };
  const log = fs.openSync(path.join(tmp, "tauri-driver.log"), "w");
  const args = webdriverServerArgs(driverPort, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver");
  const cmd = cpus ? "taskset" : TD;
  const cmdArgs = cpus ? ["-c", cpus, TD, ...args] : args;
  const td = spawn(cmd, cmdArgs, { env, stdio: ["ignore", log, log], detached: true });
  await sleep(2500);
  const browser = await remote({
    hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, "default", process.platform),
  });
  await ensureMainWindow(browser);
  await browser.setTimeout({ script: 300_000 });
  let appPid = null;
  for (let i = 0; i < 50 && !appPid; i++) { appPid = findAppPid(); if (!appPid) await sleep(100); }
  const stop = async () => {
    try { await browser.deleteSession(); } catch {}
    try { process.kill(-td.pid, "SIGKILL"); } catch {}
    await sleep(500);
  };
  return { browser, appPid, stop, graph, env };
}

/** Find the app's X window and size it to w x h CSS px (device px = css*dpr). */
export async function sizeWindow(browser, w, h) {
  try { await browser.setWindowRect(0, 0, w, h); } catch {}
  return browser.execute(() => ({ iw: innerWidth, ih: innerHeight, dpr: devicePixelRatio, sx: screenX, sy: screenY }));
}

/** Screen (X11) coordinates of a CSS client point: calibrated by moving the
 * real pointer and reading back clientX in the page. */
export async function calibratePointer(browser) {
  const probe = async (sx, sy) => {
    xdo("mousemove", String(sx), String(sy));
    await sleep(150);
    return browser.execute(() => window.__perf?.pointer ?? null);
  };
  const a = await probe(400, 300);
  const b = await probe(800, 600);
  if (!a || !b) return null;
  const scaleX = (800 - 400) / (b[0] - a[0]);
  const scaleY = (600 - 300) / (b[1] - a[1]);
  return { offX: 400 - a[0] * scaleX, offY: 300 - a[1] * scaleY, scaleX, scaleY,
    toScreen: (cx, cy) => [Math.round(400 - a[0] * scaleX + cx * scaleX), Math.round(300 - a[1] * scaleY + cy * scaleY)] };
}
