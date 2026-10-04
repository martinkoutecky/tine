// QC1: real page menu after an external namespace-file deletion, with the watcher running.
// Run under xvfb-run; TINE_APP must name the candidate binary.
import { spawn } from "node:child_process";
import fs from "node:fs";
import { remote } from "webdriverio";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";
import { answerNativeDialog } from "./lib/e2e-native-dialog.mjs";

const root = fs.mkdtempSync("/tmp/tine-qc1-identity-");
const graph = `${root}/graph`;
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${graph}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${root}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${graph}/logseq/config.edn`, "{:file/name-format :triple-lowbar}\n");
fs.writeFileSync(`${graph}/pages/Example___Namespace.md`, "- namespace body\n");
fs.writeFileSync(`${graph}/pages/Clean___Namespace.md`, "- clean body\n");
fs.writeFileSync(`${graph}/pages/Untouched.md`, "- sibling survives\n");
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: `${root}/xdg/data`,
  XDG_CONFIG_HOME: `${root}/xdg/config`, XDG_CACHE_HOME: `${root}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", LIBGL_ALWAYS_SOFTWARE: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4490);
const log = fs.openSync(`${root}/driver.log`, "w");
const driver = spawn(process.env.TAURI_DRIVER || `${process.env.CARGO_HOME}/bin/tauri-driver`,
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver",
    process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
try {
  // Driver readiness is observed, not assumed from a delay.
  const deadline = Date.now() + 10_000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (Date.now() > deadline) throw new Error("driver did not become ready");
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP } }, connectionRetryCount: 0 });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Example/Namespace");
  await browser.execute(() => [...document.querySelectorAll(".nav-section-header")]
    .find((e) => e.textContent.includes("NAMESPACES"))?.click());
  await browser.waitUntil(async () => browser.execute(() =>
    [...document.querySelectorAll(".ns-node-label")].some((e) => e.textContent.trim() === "Example")),
    { timeout: 8_000, timeoutMsg: "seed namespace was not visible" });
  // Original report: a clean external deletion retires the abandoned namespace.
  await openPageByName(browser, "Clean/Namespace");
  fs.unlinkSync(`${graph}/pages/Clean___Namespace.md`);
  await browser.waitUntil(async () => (await currentPageTitle(browser)) !== "Clean/Namespace"
    && await browser.execute(() => ![...document.querySelectorAll(".ns-node-label")]
      .some((e) => e.textContent.trim() === "Clean")),
    { timeout: 8_000, timeoutMsg: "clean external deletion left an abandoned namespace" });
  await openPageByName(browser, "Example/Namespace");
  // The menu snapshots a path; external removal can retire its route during confirmation.
  await browser.execute(() => document.querySelector(".page-title")
    .dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true,
      clientX: 300, clientY: 200 })));
  await browser.$('[data-page-action-id="delete-page"]').waitForExist();
  await browser.execute(() => document.querySelector('[data-page-action-id="delete-page"]').click());
  fs.unlinkSync(`${graph}/pages/Example___Namespace.md`);
  await browser.waitUntil(async () => browser.execute(() =>
    ![...document.querySelectorAll(".ns-node-label")].some((e) => e.textContent.trim() === "Example")),
    { timeout: 8_000, timeoutMsg: "external deletion left a ghost namespace in the sidebar" });
  console.log("PASS external deletion: namespace inventory retires while confirmation is open");
  await answerNativeDialog("yes", { env });
  await browser.waitUntil(async () => browser.execute(() =>
    [...document.querySelectorAll(".toast")].some((e) => e.textContent.includes("Deleted"))),
    { timeout: 8_000, timeoutMsg: "GH #620: page-menu deletion of the externally deleted namespace failed" });
  if ((await currentPageTitle(browser)) === "Example/Namespace") throw new Error("deleted page remains routed");
  if (fs.existsSync(`${graph}/pages/Example___Namespace.md`)) throw new Error("deleted namespace was recreated");
  if (fs.readFileSync(`${graph}/pages/Untouched.md`, "utf8") !== "- sibling survives\n") throw new Error("sibling changed");
  console.log("PASS GH #620: watcher deletion followed by page-menu deletion retires the page without writing siblings");
} catch (error) {
  console.error(String(error));
  if (browser) {
    console.error(await browser.execute(() => ({ titles: [...document.querySelectorAll(".page-title")].map((e) => e.textContent),
      messages: [...document.querySelectorAll(".toast, .conflict-bar")].map((e) => e.textContent) })));
  }
  process.exitCode = 1;
} finally {
  await Promise.race([browser?.deleteSession().catch(() => {}), new Promise((resolve) => setTimeout(resolve, 3000))]);
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
  console.log(`QC1 artifacts: ${root}`);
}
