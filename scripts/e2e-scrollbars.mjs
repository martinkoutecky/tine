// Linux real-WebKit regression for GH #103: the three primary pane scrollers
// use the shared theme-aware contract while retaining independent scrolling.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4492);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4493);
const TMP = "/tmp/tine-scrollbars-e2e";
const GRAPH = `${TMP}/graph`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || `${TMP}/artifacts`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(ARTIFACTS, { recursive: true });
const favorites = Array.from({ length: 70 }, (_, i) => `Page ${String(i).padStart(3, "0")}`);
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, `{:favorites [${favorites.map((name) => JSON.stringify(name)).join(" ")}]}\n`);
fs.writeFileSync(`${GRAPH}/pages/Overflow.md`, Array.from({ length: 140 }, (_, i) => `- Long block ${i + 1} with enough content to occupy the main page and sidebar`).join("\n") + "\n");
for (let i = 0; i < 130; i++) fs.writeFileSync(`${GRAPH}/pages/Page ${String(i).padStart(3, "0")}.md`, `- page ${i}\n`);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- Open [[Overflow]]\n");

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
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  await browser.keys(["Control", "k"]);
  const input = await browser.$(".switcher-input");
  await input.setValue("Overflow");
  await browser.$(".switcher-row").click();
  await browser.waitUntil(async () => (await browser.$("h1.page-title").getText()).trim() === "Overflow", { timeout: 10_000 });
  await browser.execute(() => {
    const title = document.querySelector("h1.page-title");
    title?.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0, shiftKey: true }));
  });
  await browser.$(".right-sidebar .rs-item-body").waitForExist({ timeout: 10_000 });

  const proof = await browser.execute(() => {
    const selectors = [".left-sidebar-scroll", ".main-content", ".right-sidebar"];
    const amounts = [120, 240, 360];
    return selectors.map((selector, index) => {
      const element = document.querySelector(selector);
      if (!(element instanceof HTMLElement)) return { selector, missing: true };
      element.scrollTop = amounts[index];
      const style = getComputedStyle(element);
      return {
        selector,
        clientHeight: element.clientHeight,
        scrollHeight: element.scrollHeight,
        scrollTop: element.scrollTop,
        scrollbarColor: style.getPropertyValue("scrollbar-color"),
        thumb: style.getPropertyValue("--scrollbar-thumb").trim(),
      };
    });
  });
  if (proof.some((item) => item.missing || item.scrollHeight <= item.clientHeight || item.scrollTop <= 0 || !item.thumb)) {
    throw new Error(`pane scrollbar contract/overflow failed: ${JSON.stringify(proof)}`);
  }
  const positions = proof.map((item) => item.scrollTop);
  if (new Set(positions).size !== 3) throw new Error(`pane scroll positions are not independent: ${JSON.stringify(proof)}`);
  await browser.saveScreenshot(path.join(ARTIFACTS, "three-pane-scrollbars.png"));
  console.log(`PASS: three primary panes overflow independently with shared scrollbar token (${JSON.stringify(proof)})`);
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
