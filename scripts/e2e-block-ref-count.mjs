// Real-app regression for GH #154. A graph-wide reference-count map is loaded
// before a new block reference exists; saving the reference must refresh the
// source block's badge without reopening the graph.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4530);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4531);
const TMP = "/tmp/tine-block-ref-count-e2e";
const GRAPH = `${TMP}/graph`;
const TARGET = "77777777-7777-4777-8777-777777777777";

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(`${GRAPH}/pages/Source Target.md`, `- Source target\n  id:: ${TARGET}\n`);
fs.writeFileSync(`${GRAPH}/pages/Tester.md`, "- \n");
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[Source Target]]\n- Open [[Tester]]\n");

const env = {
  ...process.env,
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
async function openPage(label) {
  for (const selector of [`a.page-ref=${label}`, `span.page-ref=${label}`, `*=${label}`]) {
    const link = await browser.$(selector);
    if (await link.isExisting()) {
      await link.click();
      await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === label, {
        timeout: 10_000, timeoutMsg: `${label} did not open`,
      });
      return;
    }
  }
  throw new Error(`no page link for ${label}`);
}

try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error",
    connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });

  // Force the initial empty count map to load while the source block is mounted.
  await openPage("Source Target");
  await sleep(1500);
  if (await browser.$(".block-refs-count").isExisting()) throw new Error("source badge existed before any reference");

  const back = await browser.$('button[title="Go back"]');
  await back.click();
  await browser.waitUntil(async () => (await browser.$$(".page-ref")).length >= 2, {
    timeout: 10_000, timeoutMsg: "journal links did not return after Back",
  });
  await openPage("Tester");

  await browser.$(".page-blocks .block-content-wrapper").click();
  const editor = await browser.$(".page-blocks textarea.block-editor");
  await editor.waitForExist({ timeout: 5000 });
  await editor.setValue(`((${TARGET}))`);
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => fs.readFileSync(`${GRAPH}/pages/Tester.md`, "utf8").includes(`((${TARGET}))`), {
    timeout: 10_000, timeoutMsg: "new block reference was not saved",
  });

  const ref = await browser.$(".page-blocks .block-ref");
  await ref.waitForExist({ timeout: 10_000 });
  await ref.click();
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Source Target", {
    timeout: 10_000, timeoutMsg: "saved block reference did not open its source",
  });
  const badge = await browser.$(".block-refs-count");
  await badge.waitForExist({ timeout: 10_000 });
  if ((await badge.getText()).trim() !== "1") throw new Error(`expected reference count 1, got ${JSON.stringify(await badge.getText())}`);
  console.log("PASS: saved block reference refreshed the source badge to 1");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
