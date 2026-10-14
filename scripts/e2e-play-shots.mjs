// Google Play phone screenshots from the REAL app (real engine + a curated graph).
// The window cannot go below 640 px, so interface zoom 1.8 gives a ~355 CSS px viewport
// (the phone layout); GDK_SCALE=2 renders it crisply. Output is resized to 1080x1920.
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const APP = process.env.TINE_APP;
const GRAPH_SRC = process.env.STORE_GRAPH;
const OUT = process.env.OUT;
const DRIVER = 4612, NATIVE = 4613;
const TMP = path.join(os.tmpdir(), `tine-play-shots-${process.pid}`);
const GRAPH = path.join(TMP, process.env.GRAPH_NAME ?? "Notes");
fs.rmSync(TMP, { recursive: true, force: true });
fs.cpSync(GRAPH_SRC, GRAPH, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, "xdg", dir), { recursive: true });
fs.mkdirSync(OUT, { recursive: true });
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "xdg", "data"), XDG_CONFIG_HOME: path.join(TMP, "xdg", "config"), XDG_CACHE_HOME: path.join(TMP, "xdg", "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
  GDK_SCALE: process.env.GDK_SCALE ?? "2",
};
const target = await startWebdriverApplication(APP, env, NATIVE);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn("tauri-driver", webdriverServerArgs(DRIVER, NATIVE, "/usr/bin/WebKitWebDriver"), { env: target.env, stdio: ["ignore", log, log], detached: true });
let browser;
const shot = async (name) => {
  await sleep(1200);
  // X-level grab: WebKitWebDriver's screenshot call can stall on some pages.
  const r = await browser.getWindowRect();
  const k = Number(env.GDK_SCALE);
  const raw = path.join(TMP, "root.png");
  execFileSync("import", ["-window", "root", raw], { env: { ...process.env, DISPLAY: process.env.DISPLAY } });
  execFileSync("convert", [raw, "-crop", `${r.width * k}x${r.height * k}+${r.x * k}+${r.y * k}`, "+repage", path.join(OUT, `${name}.png`)]);
  console.log("OK", name, JSON.stringify(r));
};
const closeToasts = () => browser.execute(() => document.querySelectorAll(".toast-close").forEach((b) => b.click()));
async function search(text, enter) {
  await browser.keys(["Control", "k"]); await sleep(400);
  for (const ch of text) { await browser.keys([ch]); await sleep(60); } await sleep(1200);
  if (enter) { await browser.keys(["Enter"]); await sleep(1200); }
}
try {
  await sleep(2500);
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(APP, "default", process.platform, target.debuggerAddress),
  });
  await ensureMainWindow(browser);
  await browser.setWindowSize(Number(process.env.WIN_W ?? 640), Number(process.env.WIN_H ?? 1138));
  await browser.execute((z) => localStorage.setItem("logseq-claude.zoom", z), process.env.ZOOM ?? "1.8");
  await browser.refresh();
  await browser.waitUntil(async () => (await browser.$$(".page-title")).length > 0, { timeout: 30000 });
  await sleep(2500);
  await closeToasts();
  if (Number(process.env.WIN_W ?? 640) <= 640) await browser.keys(["Escape"]);
  const dims = await browser.execute(() => [innerWidth, innerHeight, devicePixelRatio]);
  console.log("viewport", dims);
  const scene = process.env.SCENE ?? "1-journal";
  if (scene === "2-page") { await search(process.env.PAGE ?? "Garden Notes Workshop", true); await closeToasts(); }
  if (scene === "3-outline") { await search("Field Guide", true); await closeToasts(); }
  if (scene === "4-search") await search("soil", false);
  if (scene === "5-dark") { await browser.execute(() => document.querySelector('[title^="Toggle theme"]')?.click()); await sleep(600); await closeToasts(); }
  await shot(scene);
} finally {
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-driver.pid); } catch {}
  stopWebdriverApplication(target);
}
