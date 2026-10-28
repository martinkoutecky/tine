// Git sync on demand (ADR 0078 slice 2) against a local bare repository over file://:
//  1. Settings → Graph names the remote and turns on "Commit and sync on demand".
//  2. An edit in the app, then Sync now: the bare repository has the autosave commit with the edit.
//  3. A second clone pushes a change to an open page; Sync now updates that page in the open app.
//  4. Both sides move: Sync now stops at "diverged", changes nothing in the graph or the remote, says so in
//     Settings, and the notice outside Settings appears with the git command to merge.
//  5. An HTTPS remote offers Sign in; the token lands in a private app-data file, never in the settings file.
// Usage: node scripts/e2e-git-sync.mjs (needs git on PATH; a custom-protocol build at TINE_APP or target/debug/tine).
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const run = path.resolve(`artifacts/git-sync/native/run-${Date.now()}`);
const shots = process.env.E2E_SCREENSHOT_DIR ? path.resolve(process.env.E2E_SCREENSHOT_DIR) : run;
const graph = path.join(run, "graph");
const bare = path.join(run, "remote.git");
const peer = path.join(run, "peer");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "home"]) fs.mkdirSync(path.join(run, dir), { recursive: true });
fs.mkdirSync(shots, { recursive: true });
fs.writeFileSync(path.join(graph, "pages/Alpha.md"), "- alpha body\n");
fs.writeFileSync(path.join(graph, "pages/Other.md"), "- before\n");
const gitIn = (dir) => (...args) => execFileSync("git", ["-C", dir, ...args], { encoding: "utf8" }).trim();
const git = gitIn(graph);
const configure = (g) => { g("config", "user.name", "E2E"); g("config", "user.email", "e2e@example.invalid"); g("config", "core.autocrlf", "false"); };
execFileSync("git", ["init", "-q", "--bare", "-b", "main", bare]);
const url = pathToFileURL(bare).href;
git("init", "-q", "-b", "main");
configure(git);
git("add", "-A");
git("commit", "-q", "-m", "initial");
git("remote", "add", "origin", url);
git("push", "-q", "-u", "origin", "main");
execFileSync("git", ["clone", "-q", url, peer]);
const laptop = gitIn(peer);
configure(laptop);
const remoteGit = (...args) => execFileSync("git", ["--git-dir", bare, ...args], { encoding: "utf8" }).trim();
const laptopPushes = (text) => {
  laptop("pull", "-q", "--ff-only");
  fs.writeFileSync(path.join(peer, "pages/Other.md"), text);
  laptop("commit", "-q", "-am", "from the laptop");
  laptop("push", "-q");
  return laptop("rev-parse", "HEAD");
};

const app = process.env.TINE_APP || path.resolve("target/debug/tine");
const env = { ...process.env, TINE_GRAPH: graph, HOME: path.join(run, "home"), XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4636);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4637);
const target = await startWebdriverApplication(app, env, nativePort);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env: target.env, stdio: ["ignore", log, log], detached: true });
let browser;

const gitSetting = () => browser.execute(() => document.querySelector('[data-setting-label="Commit changes to git"]')?.textContent ?? "");
async function openGitSettings() {
  await browser.$('button.icon-btn[title^="Settings"]').click();
  await browser.waitUntil(() => browser.execute(() => {
    const item = [...document.querySelectorAll(".settings-nav-item")].find((node) => node.textContent?.trim() === "Graph");
    item?.click();
    return !!document.querySelector("[data-git-policy]");
  }), { timeout: 15_000, timeoutMsg: "Settings → Graph never offered the git policy control" });
  await browser.execute(() => document.querySelector('[data-setting-label="Commit changes to git"]')?.scrollIntoView({ block: "center" }));
}
async function closeSettings() {
  await browser.keys("Escape");
  await browser.waitUntil(() => browser.execute(() => !document.querySelector("[data-git-policy]")), { timeout: 5000, timeoutMsg: "Settings did not close" });
}
/** A screenshot of the X display: WebKitWebDriver's screenshot stalls on the main window here once a page has
 *  been edited (the stall family in the E2E notes), so the journey does not depend on it. */
const shot = (name) => {
  const file = path.join(shots, `${name}.png`);
  if (process.env.DISPLAY) return execFileSync("import", ["-window", "root", file]);
  return browser.saveScreenshot(file);
};
/** Press Sync now in Settings and wait for the cycle's result row. */
async function syncNow(expected) {
  await browser.$("[data-git-sync-now]").waitForEnabled({ timeout: 10_000, timeoutMsg: "Sync now stayed disabled" });
  await browser.execute(() => document.querySelector("[data-git-sync-now]").click());
  await browser.waitUntil(() => browser.execute((code) => !!document.querySelector(`[data-git-sync-result="${code}"]`), expected), {
    timeout: 30_000, interval: 250, timeoutMsg: `Sync now never ended "${expected}": ${JSON.stringify(gitSetting())}` });
}
async function editFirstBlock(text) {
  await browser.waitUntil(() => browser.execute(() => {
    const content = document.querySelector(".page .ls-block .block-content-wrapper");
    if (!(content instanceof HTMLElement)) return false;
    content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, button: 0 }));
    content.click();
    return document.activeElement instanceof HTMLTextAreaElement;
  }), { timeout: 10_000, interval: 300, timeoutMsg: "the first block's editor did not open" });
  await browser.keys(["End"]);
  for (const key of text) await browser.keys([key]);
  await browser.keys("Escape");
}
const pageText = () => browser.execute(() => document.querySelector(".page")?.textContent ?? "");

try {
  await sleep(2500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(app, "default", process.platform, target.debuggerAddress) });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1280, 900);
  await browser.$(".page-title, .ls-block").waitForExist({ timeout: 30_000 });

  // 1. The remote, and the sync option.
  await openGitSettings();
  const shown = await gitSetting();
  if (!shown.includes(`Remote: origin (${url})`)) throw new Error(`Settings did not name the remote: ${JSON.stringify(shown)}`);
  await shot("settings-off");
  await browser.execute(() => {
    const select = document.querySelector("[data-git-policy]");
    select.value = "sync-on-demand";
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await browser.$("[data-git-sync-now]").waitForExist({ timeout: 10_000, timeoutMsg: "Sync now never appeared after choosing Commit and sync on demand" });
  await shot("settings-sync-on-demand");
  await closeSettings();
  console.log("PASS: Settings → Graph names the remote and turns on Commit and sync on demand");

  // 2. Edit → Sync now → the bare repository has the commit.
  await openPageByName(browser, "Alpha");
  await editFirstBlock(" edited in Tine");
  await waitForFileText(path.join(graph, "pages/Alpha.md"), (text) => text.includes("alpha body edited in Tine"));
  await openGitSettings();
  await syncNow("pushed");
  if (remoteGit("log", "-1", "--format=%s", "main") !== "Tine: autosave") throw new Error(`remote tip: ${remoteGit("log", "-3", "--format=%s", "main")}`);
  if (!remoteGit("show", "main:pages/Alpha.md").includes("alpha body edited in Tine")) throw new Error("the pushed commit lacks the edit");
  if (git("rev-parse", "HEAD") !== remoteGit("rev-parse", "main")) throw new Error("the graph and the remote disagree after a push");
  await shot("settings-pushed");
  await closeSettings();
  console.log("PASS: an edit, then Sync now, pushes Tine's commit to the bare repository");

  // 3. A second clone pushes; Sync now updates the open page.
  await openPageByName(browser, "Other");
  const theirs = laptopPushes("- from the laptop\n");
  await openGitSettings();
  await syncNow("pulled");
  await closeSettings();
  await browser.waitUntil(async () => (await pageText()).includes("from the laptop"), { timeout: 15_000, interval: 200 })
    .catch(async () => { throw new Error(`the open page never showed the pulled change: ${JSON.stringify(await pageText())}`); });
  if (git("rev-parse", "HEAD") !== theirs) throw new Error("HEAD did not fast-forward to the laptop's commit");
  if (fs.readFileSync(path.join(graph, "pages/Other.md"), "utf8") !== "- from the laptop\n") throw new Error("the pulled page is not on disk");
  console.log("PASS: a commit from another clone reaches the open page through Sync now");

  // 4. Both sides move: diverged, nothing changes, the notice says what to do.
  const remoteTip = laptopPushes("- laptop again\n");
  await openPageByName(browser, "Alpha");
  await editFirstBlock(" twice");
  await waitForFileText(path.join(graph, "pages/Alpha.md"), (text) => text.includes("edited in Tine twice"));
  await openGitSettings();
  await syncNow("diverged");
  await browser.$("[data-git-sync-problem=diverged]").waitForExist({ timeout: 5000, timeoutMsg: "Settings named no diverged problem" });
  const problem = await gitSetting();
  if (!problem.includes("both have new commits") || !problem.includes(`git -C "${graph}" pull`)) throw new Error(`diverged text: ${JSON.stringify(problem)}`);
  if (remoteGit("rev-parse", "main") !== remoteTip) throw new Error("a diverged sync changed the remote");
  if (fs.readFileSync(path.join(graph, "pages/Other.md"), "utf8") !== "- from the laptop\n") throw new Error("a diverged sync changed the graph");
  if (git("log", "-1", "--format=%s") !== "Tine: autosave" || git("status", "--porcelain")) throw new Error("Sync now did not commit the pending edit before stopping");
  await shot("settings-diverged");
  await closeSettings();
  await browser.waitUntil(() => browser.execute(() => document.querySelector('[data-testid="git-sync-notice"]')?.getAttribute("data-git-sync-code") === "diverged"),
    { timeout: 25_000, interval: 500, timeoutMsg: "the notice outside Settings never showed the divergence" });
  const notice = await browser.execute(() => document.querySelector('[data-testid="git-sync-notice"]')?.textContent ?? "");
  if (!notice.includes("Copy the git command") || !notice.includes("Git settings")) throw new Error(`notice offers no fix: ${JSON.stringify(notice)}`);
  await shot("notice-diverged");
  console.log("PASS: a diverged sync changes nothing, says so in Settings, and the notice offers the git command");

  // 5. An HTTPS remote: Sign in keeps the token in a private app-data file only.
  git("remote", "set-url", "origin", "https://example.invalid/me/notes.git");
  await openGitSettings();
  await browser.$("[data-git-sign-in-open]").waitForExist({ timeout: 20_000, timeoutMsg: "an HTTPS remote offered no Sign in" });
  await browser.execute(() => document.querySelector("[data-git-sign-in-open]").click());
  await browser.$("[data-git-token]").waitForExist({ timeout: 5000 });
  await browser.execute(() => {
    const set = (selector, value) => { const input = document.querySelector(selector); input.value = value; input.dispatchEvent(new Event("input", { bubbles: true })); };
    set("[data-git-username]", "me"); set("[data-git-token]", "e2e-secret-token");
  });
  await shot("settings-sign-in");
  await browser.execute(() => document.querySelector("[data-git-sign-in-form]").requestSubmit());
  await browser.waitUntil(async () => (await gitSetting()).includes("Signed in to https://example.invalid as me."), { timeout: 10_000, timeoutMsg: "sign-in never showed" });
  await shot("settings-signed-in");
  const files = (dir) => fs.readdirSync(dir, { recursive: true }).map((name) => path.join(dir, name));
  const tokenFile = files(path.join(run, "data")).find((file) => path.basename(file) === "git-credentials.json");
  if (!tokenFile || (fs.statSync(tokenFile).mode & 0o777) !== 0o600) throw new Error(`token file: ${tokenFile}`);
  const leaks = [...files(path.join(run, "data")), ...files(path.join(run, "config")), ...files(graph)]
    .filter((file) => file !== tokenFile && fs.statSync(file).isFile() && fs.readFileSync(file).includes("e2e-secret-token"));
  if (leaks.length) throw new Error(`the token leaked into ${leaks}`);
  console.log("PASS: Sign in keeps the token in an owner-only app-data file and nowhere else");
} catch (error) {
  try { if (process.env.DISPLAY) execFileSync("import", ["-window", "root", path.join(run, "failure.png")]); } catch {}
  fs.writeFileSync(path.join(run, "failure.json"), JSON.stringify({ journey: "git sync", expected: "Sync now pushes, pulls into the open page, stops at divergence with a notice; Sign in stores the token privately", observed: String(error), classification: "ambiguous", commit: process.env.TESTED_COMMIT || "working tree" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  stopWebdriverApplication(target); fs.closeSync(log);
}
