// GH #805/#815: synthetic routed page, actual keyboard Quit and native chrome.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { editorSelector, scratchProfile } from "./lib/native-editor-bench.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/og-window-805-815"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
const secondGraph = path.join(out, "second-graph");
for (const graphRoot of [graph, secondGraph]) {
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graphRoot, dir), { recursive: true });
  fs.writeFileSync(path.join(graphRoot, "logseq/config.edn"), '{:tine/guide-announced? true}\n');
  fs.writeFileSync(path.join(graphRoot, "pages", "Window Demo.md"), "- Pending edit\n");
}
const pageFile = path.join(graph, "pages", "Window Demo.md");
const ownerFixture = installAccountHome(out);
const env = { ...process.env, ...scratchProfile(out), TINE_GRAPH: graph,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4886);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  workingDiff: true, app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  journey: "Sidebar header background shares title-bar gestures; workspace switcher clicks; Ctrl+Q while editing saves and closes every graph window and releases owner locks",
  unverified: ["Physical window movement under Xvfb without a window manager", "Windows", "macOS"] };
let browser;
let stage = "startup";
try {
  await sleep(1500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, "og-window") });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Window Demo");
  await assertAppLease(ownerFixture);
  const firstHandle = await browser.getWindowHandle();

  stage = "GH #815 header background and workspace switcher";
  const header = await browser.$("[data-workspace-switcher-sidebar]");
  await header.waitForDisplayed();
  const regions = await browser.execute(() => {
    const row = document.querySelector("[data-workspace-switcher-sidebar]");
    return { header: row.getAttribute("data-tauri-drag-region"),
      toolbar: document.querySelector("header.topbar").getAttribute("data-tauri-drag-region"),
      controls: [...row.querySelectorAll("button")].map(button => button.hasAttribute("data-tauri-drag-region")) };
  });
  if (regions.header !== "" || regions.toolbar !== "" || !regions.controls.length || regions.controls.some(Boolean))
    throw Error(`Wrong native drag regions: ${JSON.stringify(regions)}`);
  const switcher = await browser.$("[data-workspace-switcher-sidebar] .workspace-switcher-btn");
  await switcher.click();
  await browser.waitUntil(async () => await switcher.getAttribute("aria-expanded") === "true", { timeout: 10000 });
  await browser.$("[data-workspace-switcher-sidebar] [role=menu]").waitForDisplayed();
  try { execFileSync("import", ["-window", "root", path.join(out, "workspace-menu.png")], { env }); } catch {}
  await browser.$("[data-workspace-switcher-sidebar] .workspace-menu-name").click();
  await browser.waitUntil(async () => await switcher.getAttribute("aria-expanded") === "false", { timeout: 10000 });
  proof.regions = regions;

  stage = "open another graph through the single-instance CLI handoff";
  const forwarded = spawn(app, ["open", secondGraph], { env, stdio: ["ignore", log, log] });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      forwarded.kill("SIGTERM");
      reject(Error("Harness: second launch did not forward; run inside dbus-run-session on Linux"));
    }, 20000);
    forwarded.once("error", error => { clearTimeout(timer); reject(error); });
    forwarded.once("exit", () => { clearTimeout(timer); resolve(); });
  });
  await browser.waitUntil(async () => (await browser.getWindowHandles()).length > 1, { timeout: 30000 });
  await browser.switchToWindow(firstHandle);
  await assertAppLease(ownerFixture);
  await browser.waitUntil(() => ownerFixture.sample().length === 2, { timeout: 20000 });
  proof.ownersBefore = ownerFixture.sample();

  stage = "GH #805 type a pending edit, then Ctrl+Q without blur";
  await browser.$(".page .block-content-wrapper").click();
  const editor = await browser.$(`.page ${editorSelector}`);
  await editor.waitForDisplayed({ timeout: 10000 });
  await browser.keys(["End"]);
  const suffix = " quit805";
  await editor.addValue(suffix);
  proof.beforeQuit = fs.readFileSync(pageFile, "utf8");
  if (proof.beforeQuit.includes(suffix)) throw Error("Harness: edit was already saved before Ctrl+Q; pending-save ordering was not exercised");
  // No blur, navigation, save wait or screenshot between the edit and Quit.
  try { await browser.keys(["Control", "q", "NULL"]); }
  catch (error) { proof.quitDriverReply = String(error); } // Closing the last webview can terminate the reply.
  const deadline = Date.now() + 20000;
  const alive = pid => {
    try { return !/\) Z /.test(fs.readFileSync(`/proc/${pid}/stat`, "utf8")); }
    catch { return false; }
  };
  while (ownerFixture.sample().length || proof.ownersBefore.some(owner => alive(owner.pid))) {
    if (Date.now() >= deadline) throw Error("Ctrl+Q did not close all graph windows and retire the owner process");
    await sleep(100);
  }
  proof.saved = fs.readFileSync(pageFile, "utf8");
  if (!proof.saved.includes(suffix)) throw Error(`Pending text was lost on quit: ${JSON.stringify(proof.saved)}`);
  proof.ownersAfter = ownerFixture.sample();
  proof.outcome = "PASS";
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS GH #805/#815: ${out}`);
} catch (error) {
  try { execFileSync("import", ["-window", "root", path.join(out, "failure.png")], { env }); } catch {}
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "Clickable workspace menu on a native drag-region header; pending edit saved and all graph windows closed through Ctrl+Q, owner locks released",
    observed: String(error.stack), classification: String(error).includes("Harness:") ? "harness" : "ambiguous; inspect driver log and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
