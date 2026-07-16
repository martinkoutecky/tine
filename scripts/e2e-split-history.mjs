// Linux real-app proof for GH #170. A global Back/Forward toolbar click must
// preserve the focused split pane and navigate that pane's active-tab history.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4522);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4523);
const TMP = "/tmp/tine-split-history-e2e";
const GRAPH = `${TMP}/graph`;

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Page 1]]\n");
fs.writeFileSync(`${GRAPH}/pages/Page 1.md`, "- left source [[Page 2]]\n");
fs.writeFileSync(`${GRAPH}/pages/Page 2.md`, "- right source [[B]]\n");
fs.writeFileSync(`${GRAPH}/pages/B.md`, "- right destination\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });

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
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });

  const openPageViaSwitcher = async (paneId, name) => {
    // Make the pane choice an explicit user observation, not an assumption
    // about which async split/layout event happened last. This also exercises
    // the production capture-phase pane tracker before opening the overlay.
    await browser.execute((id) => {
      const pane = document.querySelector(`[data-pane-id="${id}"]`);
      pane?.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button: 0 }));
    }, paneId);
    await browser.waitUntil(() => browser.execute((id) =>
      document.querySelector(`.pane-leaf[data-pane-id="${id}"]`)?.classList.contains("pane-focused")
      ?? (id === "main" && !document.querySelector(".pane-leaf")), paneId), {
      timeout: 5_000,
      timeoutMsg: `${paneId} did not become the focused switcher origin`,
    });
    await browser.keys(["Control", "k"]);
    const input = await browser.$(".switcher-input");
    await input.waitForExist({ timeout: 5_000 });
    await input.setValue(name);
    await browser.waitUntil(async () => browser.execute((label) => {
      const active = document.querySelector(".switcher-row.active");
      return active?.querySelector(".switcher-kind")?.textContent?.trim() === "page"
        && active.querySelector(".switcher-name")?.textContent?.trim() === label;
    }, name), { timeout: 10_000, timeoutMsg: `${name} did not become the active page result in the switcher` });
    await browser.keys("Enter");
    await browser.waitUntil(async () => browser.execute((id, expected) =>
      document.querySelector(`[data-pane-id="${id}"] .page-title`)?.textContent?.trim() === expected,
    paneId, name), { timeout: 10_000, timeoutMsg: `${paneId} did not open ${name}` });
  };

  await openPageViaSwitcher("main", "Page 1");
  await browser.execute(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", {
      key: "\\", code: "Backslash", ctrlKey: true, altKey: true, bubbles: true, cancelable: true,
    }));
  });
  await browser.waitUntil(async () => browser.execute(() =>
    document.querySelectorAll(".pane-leaf[data-pane-id]").length === 2,
  ), { timeout: 10_000, timeoutMsg: "split pane was not created" });

  const rightPane = await browser.execute(() =>
    [...document.querySelectorAll(".pane-leaf[data-pane-id]")]
      .map((node) => node.getAttribute("data-pane-id"))
      .find((id) => id && id !== "main") ?? null,
  );
  if (!rightPane) throw new Error("new split pane id was not found");

  await openPageViaSwitcher(rightPane, "Page 2");
  await openPageViaSwitcher(rightPane, "B");
  const leftBefore = await browser.execute(() =>
    document.querySelector('[data-pane-id="main"] .page-title')?.textContent?.trim() ?? null,
  );

  const back = await browser.$('button[title="Go back"]');
  if (!(await back.isEnabled())) throw new Error("Back was disabled while the focused split pane had history");
  await back.click();
  await browser.waitUntil(async () => browser.execute((id) =>
    document.querySelector(`[data-pane-id="${id}"] .page-title`)?.textContent?.trim() === "Page 2",
  rightPane), { timeout: 5_000, timeoutMsg: "toolbar Back did not return the focused split pane to Page 2" });

  const forward = await browser.$('button[title="Go forward"]');
  if (!(await forward.isEnabled())) throw new Error("Forward was disabled after split-pane Back");
  await forward.click();
  await browser.waitUntil(async () => browser.execute((id) =>
    document.querySelector(`[data-pane-id="${id}"] .page-title`)?.textContent?.trim() === "B",
  rightPane), { timeout: 5_000, timeoutMsg: "toolbar Forward did not return the focused split pane to B" });

  const leftAfter = await browser.execute(() =>
    document.querySelector('[data-pane-id="main"] .page-title')?.textContent?.trim() ?? null,
  );
  if (leftAfter !== leftBefore) throw new Error(`toolbar history mutated the main pane: ${leftBefore} -> ${leftAfter}`);

  console.log("PASS: global Back/Forward preserved the focused split pane and navigated only its active-tab history");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
