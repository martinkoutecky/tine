// GH #875: real editing and sync-like external rewrites, with scratch data only.
import "./lib/e2e-auto-account-home.mjs";
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openJournals, openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/fix-beta-conflict-875");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const markdownFile = path.join(graph, "pages/Conflict 875.md");
const orgFile = path.join(graph, "pages/Org Conflict 875.org");
const today = new Date();
const journalFile = path.join(graph, "journals", `${today.getFullYear()}_${String(today.getMonth() + 1).padStart(2, "0")}_${String(today.getDate()).padStart(2, "0")}.md`);
let file = markdownFile, prefix = "-";
fs.writeFileSync(file, "- alpha\n- beta\n");
fs.writeFileSync(orgFile, "* alpha\n* beta\n");
fs.writeFileSync(journalFile, "- alpha\n- beta\n");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const env = { ...process.env, HOME: path.join(out, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4924);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(port, port + 1, "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { base: execFileSync("/usr/bin/git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex") };
let browser, stage = "startup";
const banner = () => browser.execute(() => Boolean(document.querySelector(".conflict-banner")));
const shot = name => {
  try { execFileSync("import", ["-window", "root", path.join(out, name)], { env, timeout: 10000 }); } catch {}
};
const edit = async text => {
  await browser.waitUntil(() => browser.execute(() => {
    const block = document.querySelector(".main-content .ls-block .block-content-wrapper");
    block?.scrollIntoView({ block: "center" });
    return Boolean(block);
  }), { timeout: 10000 });
  await browser.$(".main-content .ls-block .block-content-wrapper").click();
  const input = await browser.$(".main-content textarea.block-editor");
  await input.waitForExist({ timeout: 5000 });
  await browser.keys(["Control", "a"]);
  await browser.keys(text);
};
const makeConflict = async (mine, disk) => {
  await edit(mine);
  fs.writeFileSync(file, `${prefix} ${disk}\n${prefix} beta\n`);
  await waitForFileText(file, text => text === `${prefix} ${disk}\n${prefix} beta\n`);
  await browser.waitUntil(banner, { timeout: 10000, timeoutMsg: "external write did not produce a live conflict" });
  await browser.keys("Escape");
  assert.ok(await browser.execute(mine => document.querySelector(".main-content .page-blocks")?.textContent.includes(mine), mine),
    "differing disk bytes lost the editor's input");
};
const deliverIdentical = async text => {
  const bytes = `${prefix} ${text}\n${prefix} beta\n`;
  fs.writeFileSync(file, bytes);
  await waitForFileText(file, content => content === bytes);
  await browser.waitUntil(async () => !await banner(), { timeout: 15000,
    timeoutMsg: "later identical sync delivery left the banner" });
};
const modifiedTitleClick = async key => {
  await browser.performActions([{ type: "key", id: "keyboard", actions: [{ type: "keyDown", value: key }] }]);
  try { await browser.$('[data-pane-id="main"] h1.page-title').click(); }
  finally { await browser.releaseActions(); }
};
try {
  // Wait for our driver's socket rather than sleeping for startup.
  const deadline = Date.now() + 15000;
  await new Promise((resolve, reject) => {
    const probe = () => {
      const socket = net.connect(port, "127.0.0.1");
      socket.once("connect", () => { socket.destroy(); resolve(); });
      socket.once("error", error => {
        socket.destroy();
        if (Date.now() >= deadline || driver.exitCode !== null) reject(error);
        else setImmediate(probe);
      });
    };
    probe();
  });
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 0, connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(app, "beta-conflict-875") });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Conflict 875");

  stage = "later sync delivery equals editor bytes";
  await makeConflict("alpha MINE", "alpha DISK");
  shot("differing.png");
  await deliverIdentical("alpha MINE");

  stage = "editor returns to disk bytes without another watcher event";
  await makeConflict("alpha LOCAL", "alpha REMOTE");
  await edit("alpha REMOTE");
  await browser.keys("Escape");
  // Re-entering Review reads both current versions rather than retaining the
  // comparison opened before the draft was edited.
  await openJournals(browser);
  await openPageByName(browser, "Conflict 875");
  await browser.waitUntil(async () => !await banner(), { timeout: 15000,
    timeoutMsg: "identical editor and disk left the red banner (GH #875)" });
  await waitForFileText(file, text => text === "- alpha REMOTE\n- beta\n");
  assert.equal(await browser.execute(() => Boolean(document.querySelector(".main-content .page-conflict"))), false);

  stage = "Org page uses the same exact serialized-byte retirement";
  file = orgFile; prefix = "*";
  await openPageByName(browser, "Org Conflict 875");
  await makeConflict("alpha ORG MINE", "alpha ORG DISK");
  await deliverIdentical("alpha ORG MINE");

  stage = "journal page in the feed retires its live conflict";
  file = journalFile; prefix = "-";
  await openJournals(browser);
  await makeConflict("alpha JOURNAL MINE", "alpha JOURNAL DISK");
  await deliverIdentical("alpha JOURNAL MINE");

  stage = "one page in main, split pane and sidebar retires all reviews";
  file = markdownFile;
  await openPageByName(browser, "Conflict 875");
  await makeConflict("alpha PANES MINE", "alpha PANES DISK");
  await modifiedTitleClick("\uE008"); // Shift: sidebar.
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".right-sidebar .rs-item-body")?.textContent.includes("alpha PANES MINE")),
    { timeout: 10000, timeoutMsg: "sidebar did not retain the unsaved page text" });
  await modifiedTitleClick("\uE00A"); // Alt: other pane.
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll('[data-pane-id] h1.page-title').length >= 2),
    { timeout: 10000, timeoutMsg: "the same page did not open in a split pane" });
  await deliverIdentical("alpha PANES MINE");
  await browser.waitUntil(() => browser.execute(() => !document.querySelector('.page-conflict[data-source="live-save"]')),
    { timeout: 10000, timeoutMsg: "a pane retained its live conflict review" });
  assert.ok(await browser.execute(() => document.querySelector(".right-sidebar .rs-item-body")?.textContent.includes("alpha PANES MINE")),
    "retirement lost the sidebar's editor text");
  shot("retired.png");
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify({ ...proof, outcome: "PASS" }, null, 2));
  console.log(`PASS BETA-CONFLICT-875: ${out}`);
} catch (error) {
  shot("failure.png");
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "Identical live versions retire the banner; differing versions preserve the draft",
    observed: String(error.stack), classification: "ambiguous; inspect stage and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
