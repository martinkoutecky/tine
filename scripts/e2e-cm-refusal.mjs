// D-20261116-P3H: routed title rename -> refusal -> explicit ordinary save
// and one retry -> exact page files. No real graph or account-home state.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/cm/refusal");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
const seeded = {
  "pages/Old.md": "- source body\n",
  "pages/referrer.md": "- tags:: [[Old]]\n",
  "pages/unrelated.md": "- untouched\n",
  "logseq/config.edn": "{:tine/guide-announced? true}\n",
};
for (const [file, text] of Object.entries(seeded)) fs.writeFileSync(path.join(graph, file), text);
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
fs.mkdirSync(path.join(run, "data", APP_ID), { recursive: true });
fs.writeFileSync(path.join(run, "data", APP_ID, "tine-settings.json"), JSON.stringify({ known_graphs: [{ name: "graph", path: graph }], last_graph_path: graph }));
const fixture = accountHomeFixture(run, run);
const env = { ...fixture.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4936);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const commit = execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
let driver, browser;
let stage = "startup";
try {
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(process.env.TINE_APP || path.join(root, "target/debug/tine"), "cm-refusal") });
  await ensureMainWindow(browser);
  const appUrl = new URL(await browser.getUrl());
  if (appUrl.protocol !== "tauri:" && appUrl.hostname !== "tauri.localhost") throw new Error(`Expected embedded app assets, got ${appUrl.origin}`);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });
  await assertAppLease(fixture);
  stage = "rename Old through its routed page title";
  await openPageByName(browser, "Old");
  await browser.execute(() => document.querySelector("h1.page-title")?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, view: window })));
  const input = await browser.$(".page-title-input");
  await input.waitForExist({ timeout: 4000 });
  // Use the existing rename journey's interior-input path: WebKit's native
  // clear-and-send sequence can blur and retire this commit-on-blur input.
  await browser.execute(() => {
    const input = document.querySelector(".page-title-input");
    input.focus();
    input.value = "New";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });
  stage = "refusal explains the file, waits for the user, and writes nothing";
  const action = await browser.$(".toast-error .toast-action:not(.toast-copy)");
  await action.waitForExist({ timeout: 15_000 });
  const message = await browser.$(".toast-error .toast-msg").getText();
  assert.match(message, /Tine would change the layout of pages\/referrer.md/);
  assert.match(message, /first time it saves it/);
  assert.equal(await action.getText(), "Save it and rename");
  for (const [file, text] of Object.entries(seeded)) assert.equal(fs.readFileSync(path.join(graph, file), "utf8"), text);
  assert.equal(fs.existsSync(path.join(graph, "pages/New.md")), false);
  stage = "explicit save then same rename produces the ordinary-save header and rewritten reference";
  await action.click();
  await waitForFileText(path.join(graph, "pages/referrer.md"), text => text === "tags:: [[New]]\n", { timeoutMs: 15_000 });
  await waitForFileText(path.join(graph, "pages/New.md"), text => text === seeded["pages/Old.md"], { timeoutMs: 15_000 });
  assert.equal(fs.existsSync(path.join(graph, "pages/Old.md")), false);
  assert.equal(fs.readFileSync(path.join(graph, "pages/unrelated.md"), "utf8"), seeded["pages/unrelated.md"]);
  assert.equal(fs.readFileSync(path.join(graph, "logseq/config.edn"), "utf8"), seeded["logseq/config.edn"]);
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "New", { timeout: 15_000 });
  fixture.verify();
  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, workingDiff: true, outcome: "PASS", graph, message, appUrl: appUrl.href,
    journey: "Refusal writes nothing; one user click saves the named referrer normally and repeats the rename; exact final files and routed title verified." }, null, 2));
  console.log(`PASS CM refusal recovery: ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${commit} + worktree diff\nJourney: ${stage}\nExpected: explicit save then rename, exact referrer and source bytes\nObserved: ${error.stack}\nClassification: ambiguous until logs inspected\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  fixture.close?.();
}
