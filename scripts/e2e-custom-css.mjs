// Linux real-app proof for GH #610 live reload: `logseq/custom.css` edited on
// disk by an outside actor (an editor's temp+rename save, an in-place rewrite,
// a delete) is re-applied in the open window with no reopen of the graph, over
// the real watcher path (store lane -> graph-custom-css-changed -> readCustomCss).
// The rule sets the public --tine-bullet-color token, so the same run also
// proves a token set from custom.css reaches the rendered bullet.
// "Edit custom.css" hands the file to the system opener and is covered by the
// graph-features and render tests; a native run must not launch an editor.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { APP_ID } from "./lib/app-identity.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4492);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4493);
const TMP = "/tmp/tine-custom-css-e2e";
const GRAPH = `${TMP}/graph`;
const CSS = `${GRAPH}/logseq/custom.css`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- custom css probe\n");
const rule = (color) => `:root { --tine-bullet-color: ${color}; }\n`;
fs.writeFileSync(CSS, rule("rgb(1, 2, 3)"));

for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
const appData = `${TMP}/xdg/data/${APP_ID}`;
fs.mkdirSync(appData, { recursive: true });
fs.writeFileSync(`${appData}/tine-settings.json`, JSON.stringify({ last_graph_path: fs.realpathSync(GRAPH) }, null, 2));

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`,
  XDG_CONFIG_HOME: `${TMP}/xdg/config`,
  XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  GDK_BACKEND: "x11",
};
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

let browser;
const bullet = () => browser.execute(() => {
  const el = document.querySelector(".ls-block .bullet");
  return el ? getComputedStyle(el).backgroundColor : null;
});
const waitForBullet = async (wanted, why) => {
  try {
    await browser.waitUntil(async () => (await bullet()) === wanted, { timeout: 15_000, interval: 150 });
  } catch {
    throw new Error(`${why}: bullet is ${await bullet()}, wanted ${wanted}`);
  }
};
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block .bullet").waitForExist({ timeout: 20_000 });
  await waitForBullet("rgb(1, 2, 3)", "custom.css present at open");
  // A page-lifetime marker: a reopen/reload of the graph window would drop it.
  await browser.execute(() => { window.__customCssE2eMarker = 1; });

  // An editor's save: write a temp file, rename over custom.css.
  fs.writeFileSync(`${GRAPH}/logseq/custom.css.tmp`, rule("rgb(10, 120, 30)"));
  fs.renameSync(`${GRAPH}/logseq/custom.css.tmp`, CSS);
  await waitForBullet("rgb(10, 120, 30)", "temp+rename save was not applied live");

  // An in-place rewrite (truncate + write), as simple editors do.
  fs.writeFileSync(CSS, rule("rgb(200, 40, 40)"));
  await waitForBullet("rgb(200, 40, 40)", "in-place rewrite was not applied live");

  // Deleting the file returns Tine to its own rendering.
  fs.unlinkSync(CSS);
  await browser.waitUntil(async () => {
    const color = await bullet();
    return color !== null && !["rgb(1, 2, 3)", "rgb(10, 120, 30)", "rgb(200, 40, 40)"].includes(color);
  }, { timeout: 15_000, interval: 150, timeoutMsg: "deleting custom.css did not clear its rules" });

  // Creating it again is picked up too.
  fs.writeFileSync(CSS, rule("rgb(9, 9, 200)"));
  await waitForBullet("rgb(9, 9, 200)", "a newly created custom.css was not applied live");

  const marker = await browser.execute(() => window.__customCssE2eMarker);
  if (marker !== 1) throw new Error("the window reloaded: live reload must not reopen the graph");
  console.log("PASS: custom.css edits (rename-save, in-place, delete, create) re-applied live without a reopen");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
