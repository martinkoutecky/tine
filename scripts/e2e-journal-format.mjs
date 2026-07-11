// Verifies a graph with a CUSTOM journal date format loads in the real app:
// seeds journals in `dd-MM-yyyy` filenames + a `yyyy-MM-dd` title format, launches
// the app, and checks the journal feed renders them (titled in the user's format).
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";

const G = "/tmp/tgraph-jf";
const APP = process.env.TINE_APP || `${process.env.HOME}/research/tine`;
const TD =
  process.env.TAURI_DRIVER ||
  (process.env.CARGO_HOME ? `${process.env.CARGO_HOME}/bin/tauri-driver` : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4444);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4445);

fs.rmSync(G, { recursive: true, force: true });
fs.mkdirSync(`${G}/pages`, { recursive: true });
fs.mkdirSync(`${G}/journals`, { recursive: true });
fs.mkdirSync(`${G}/logseq`, { recursive: true });
fs.writeFileSync(
  `${G}/logseq/config.edn`,
  '{:journal/file-name-format "dd-MM-yyyy" :journal/page-title-format "yyyy-MM-dd"}\n'
);
// Real journal files in the user's dd-MM-yyyy filename format.
fs.writeFileSync(`${G}/journals/24-06-2026.md`, "- entry from the 24th [[Pokus]]\n");
fs.writeFileSync(`${G}/journals/23-06-2026.md`, "- entry from the 23rd\n");
fs.writeFileSync(`${G}/pages/Pokus.md`, "- a page\n");

fs.rmSync("/tmp/txdg-jf", { recursive: true, force: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(`/tmp/txdg-jf/${d}`, { recursive: true });
const env = {
  ...process.env,
  TINE_GRAPH: G,
  XDG_DATA_HOME: "/tmp/txdg-jf/data",
  XDG_CONFIG_HOME: "/tmp/txdg-jf/config",
  XDG_CACHE_HOME: "/tmp/txdg-jf/cache",
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  GDK_BACKEND: "x11",
};

const tdLog = fs.openSync("/tmp/td.log", "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env,
  stdio: ["ignore", tdLog, tdLog],
  detached: true,
});
await sleep(3000);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1",
    port: DRIVER_PORT,
    path: "/",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
    logLevel: "error",
    connectionRetryCount: 1,
    connectionRetryTimeout: 60000,
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20000 });
} catch (e) {
  console.log("LAUNCH ERROR:", String(e).split("\n").slice(0, 3).join(" | "));
  process.exitCode = 1;
}
try {
  await sleep(2500);
  const text = await browser.execute(() => document.body.innerText);
  const has = (s) => text.includes(s);
  const checks = {
    "feed shows 2026-06-24 (custom title)": has("2026-06-24"),
    "feed shows 2026-06-23 (custom title)": has("2026-06-23"),
    "does not show raw filename 24-06-2026": !has("24-06-2026"),
    "entry content present": has("entry from the 24th"),
  };
  for (const [name, ok] of Object.entries(checks)) console.log(`${ok ? "PASS" : "FAIL"}: ${name}`);
  if (Object.values(checks).some((ok) => !ok)) throw new Error("custom journal format invariants failed");
} catch (e) {
  console.log("CHECK ERROR:", String(e).split("\n").slice(0, 3).join(" | "));
  process.exitCode = 1;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
}
