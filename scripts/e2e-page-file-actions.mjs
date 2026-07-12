// Linux real-app proof for GH #84. A fake xdg-open records the direct argv so
// the test verifies exact nested source identity without launching a host app.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4472);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4473);
const TMP = "/tmp/tine-page-file-e2e";
const GRAPH = `${TMP}/graph`;
const SOURCE = `${GRAPH}/pages/nested/Exact.org`;
const CALLS = `${TMP}/opener-calls.txt`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages/nested", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(`${TMP}/bin`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{:preferred-format \"Org\"}\n");
fs.writeFileSync(SOURCE, "* exact nested page\n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.org`, "* open [[Exact]]\n");
fs.writeFileSync(`${TMP}/bin/dbus-send`, "#!/bin/sh\nexit 1\n", { mode: 0o755 });
fs.writeFileSync(`${TMP}/bin/xdg-open`, `#!/bin/sh\nprintf '%s\\n' "$@" >> ${JSON.stringify(CALLS)}\n`, { mode: 0o755 });

const env = {
  ...process.env,
  PATH: `${TMP}/bin:${process.env.PATH}`,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

let browser;
async function runMenu(label) {
  await browser.execute(() => {
    const title = document.querySelector("h1.page-title");
    title.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 100, clientY: 100 }));
  });
  await browser.waitUntil(() => browser.execute((wanted) =>
    [...document.querySelectorAll(".ctx-item")].some((item) => item.textContent?.trim() === wanted), label),
  { timeout: 5000, timeoutMsg: `${label} menu item did not appear` });
  await browser.execute((wanted) => {
    const item = [...document.querySelectorAll(".ctx-item")].find((node) => node.textContent?.trim() === wanted);
    item.click();
  }, label);
}

try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  for (const selector of ["a.page-ref=Exact", "span.page-ref=Exact", "*=Exact"]) {
    const link = await browser.$(selector);
    if (await link.isExisting()) { await link.click(); break; }
  }
  await browser.$("h1.page-title").waitForExist({ timeout: 10_000 });
  await runMenu("Show in folder");
  await browser.waitUntil(() => fs.existsSync(CALLS), { timeout: 5000 });
  await runMenu("Open with default app");
  await browser.waitUntil(() => fs.readFileSync(CALLS, "utf8").trim().split("\n").length >= 2, { timeout: 5000 });
  const calls = fs.readFileSync(CALLS, "utf8").trim().split("\n");
  if (calls[0] !== path.dirname(SOURCE) || calls[1] !== SOURCE) throw new Error(`wrong opener argv: ${JSON.stringify(calls)}`);
  console.log(`PASS: reveal used ${calls[0]} and open used exact source ${calls[1]}`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
