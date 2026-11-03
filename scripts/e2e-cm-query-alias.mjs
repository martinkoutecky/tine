// Native journey for CM step 2 batch 9 (conformance S10, D-20261019-Q8): a
// reference query matches the page's whole alias group, live. On a page the
// user types `{{query [[ML]]}}` (pasted through the native clipboard, as a
// user would after typing it elsewhere); the result lists Notes' block
// `also [[Machine Learning]]`, because ML is an alias of Machine Learning.
// Editing Machine Learning's alias in the app (`alias:: ML` -> `alias:: AI`)
// drops that block from the mounted query; restoring `alias:: ML` brings it
// back. Written by the cm-query-leaves lane.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { APP_ID } from "./lib/app-identity.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/cm-query-alias");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
const mlFile = path.join(graph, "pages/Machine Learning.md");
const queriesFile = path.join(graph, "pages/Queries.md");
// The alias lives in the page's first block (Logseq's page-properties block), which the app
// renders and edits as a block.
fs.writeFileSync(mlFile, "- alias:: ML\n- intro\n");
fs.writeFileSync(path.join(graph, "pages/Notes.md"), "- also [[Machine Learning]]\n- unrelated line\n");
fs.writeFileSync(queriesFile, "- placeholder\n");
fs.writeFileSync(path.join(graph, "journals", "2026_11_03.md"), "- journal\n");
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir));
fs.mkdirSync(path.join(run, "data", APP_ID), { recursive: true });
fs.writeFileSync(path.join(run, "data", APP_ID, "tine-settings.json"), JSON.stringify({ known_graphs: [{ name: "graph", path: graph }], last_graph_path: graph }));
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4912);
const log = fs.openSync(path.join(run, "driver.log"), "w");
let driver;
const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
let browser;
let stage = "startup";

/** The first lines of the blocks the main pane's query result lists. */
const queryRows = () => browser.execute(() => [...document.querySelectorAll(".main-content .query-block")]
  .map((el) => el.textContent?.trim() ?? ""));
const queryShows = async (text) => (await queryRows()).some((row) => row.includes(text));

async function paste(text) {
  const pasted = await browser.executeAsync((text, done) => window.__TAURI_INTERNALS__.invoke("plugin:clipboard-manager|write_text", { text })
    .then(() => done(true), () => done(false)), text);
  assert.ok(pasted, "native clipboard preparation");
  await browser.keys(["Control", "a"]);
  await browser.keys(["Control", "v"]);
}

/** Edit the first block of the scope's page to `text`, as a user would. */
async function replaceFirstBlock(scope, text) {
  await browser.execute((scope) => {
    const el = document.querySelector(`${scope} .block-content-wrapper`);
    el?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  }, scope);
  await browser.$(`${scope} textarea.block-editor`).waitForExist({ timeout: 10_000, timeoutMsg: `no editor in ${scope}` });
  await paste(text);
  await browser.execute((scope) => document.querySelector(`${scope} textarea.block-editor`)?.blur(), scope);
}

try {
  driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env, stdio: ["ignore", log, log], detached: true });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 5,
    connectionRetryTimeout: 120_000, capabilities: tauriCapabilities(app, "cm-query-alias") });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1600, 1100);
  await browser.$(".graph-switch-btn").waitForExist({ timeout: 30_000 });

  stage = "type {{query [[ML]]}} on a page";
  await openPageByName(browser, "Queries");
  await replaceFirstBlock(".main-content", "{{query [[ML]]}}");
  await waitForFileText(queriesFile, (text) => text === "- {{query [[ML]]}}\n", { timeoutMs: 15_000 });

  stage = "the query lists the block that links [[Machine Learning]] (ML is its alias)";
  await browser.waitUntil(() => queryShows("also"), { timeout: 30_000,
    timeoutMsg: "the [[ML]] query never listed `also [[Machine Learning]]`" });
  assert.ok(!(await queryShows("unrelated line")), "a block that names no page of the group is not listed");

  stage = "open Machine Learning beside the query";
  assert.ok(await browser.execute(() => {
    // The link's text keeps its brackets (`[[Machine Learning]]`); its data-ref
    // is the page identity key.
    const link = [...document.querySelectorAll(".main-content .query-block a.page-ref[data-ref]")]
      .find((el) => el.getAttribute("data-ref") === "machine learning");
    if (!(link instanceof HTMLElement)) return false;
    link.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true, button: 0 }));
    return true;
  }), "the result's [[Machine Learning]] link is clickable");
  await browser.$(".right-sidebar .block-content-wrapper").waitForExist({ timeout: 15_000 });

  stage = "edit the alias away: the mounted query drops the block";
  await replaceFirstBlock(".right-sidebar", "alias:: AI");
  await waitForFileText(mlFile, (text) => /^(- )?alias:: AI\n/.test(text), { timeoutMs: 15_000 });
  await browser.waitUntil(async () => !(await queryShows("also")), { timeout: 30_000,
    timeoutMsg: "the [[ML]] query still lists `also [[Machine Learning]]` after ML stopped being an alias" });

  stage = "restore the alias: the mounted query lists the block again";
  await replaceFirstBlock(".right-sidebar", "alias:: ML");
  await waitForFileText(mlFile, (text) => /^(- )?alias:: ML\n/.test(text), { timeoutMs: 15_000 });
  await browser.waitUntil(() => queryShows("also"), { timeout: 30_000,
    timeoutMsg: "the [[ML]] query did not list `also [[Machine Learning]]` again after the alias came back" });

  fs.writeFileSync(path.join(run, "receipt.json"), JSON.stringify({ commit, app, outcome: "PASS",
    journey: "typed {{query [[ML]]}} lists the block linking [[Machine Learning]] (alias ML); removing the alias in the app drops it, restoring it brings it back", graph }, null, 2));
  console.log(`PASS CM query alias. ${run}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  let rows = "(unavailable)";
  try { rows = JSON.stringify(await queryRows()); } catch {}
  const files = [mlFile, queriesFile].map((file) => `${path.relative(graph, file)}: ${fs.existsSync(file) ? JSON.stringify(fs.readFileSync(file, "utf8")) : "(absent)"}`).join("\n");
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${commit} + worktree diff, app ${app}\nJourney: ${stage}\nExpected: the [[ML]] query follows Machine Learning's alias group\nObserved: ${error.stack}\nQuery rows: ${rows}\n${files}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
