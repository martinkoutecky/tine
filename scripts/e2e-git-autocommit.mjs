// Git autocommit (ADR 0078 slice 1) on a disposable graph that is a git repository:
//  1. Settings → Graph shows the branch and turns on "Commit automatically".
//  2. An edit made by another program is pending; renaming a page through its title
//     commits that edit first ("Tine: autosave"), then the rename as its own commit,
//     and leaves the work tree clean.
// Usage: node scripts/e2e-git-autocommit.mjs (needs git on PATH).
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { startWebdriverApplication, stopWebdriverApplication, tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle, openPageByName } from "./lib/e2e-navigation.mjs";

const run = path.resolve(`artifacts/git-autocommit/native/run-${Date.now()}`);
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, dir), { recursive: true });
fs.writeFileSync(path.join(graph, "pages/Alpha.md"), "- alpha body\n");
fs.writeFileSync(path.join(graph, "pages/Other.md"), "- before\n");
const git = (...args) => execFileSync("git", ["-C", graph, ...args], { encoding: "utf8" }).trim();
git("init", "-q", "-b", "main");
git("config", "user.name", "E2E");
git("config", "user.email", "e2e@example.invalid");
git("add", "-A");
git("commit", "-q", "-m", "initial");

const app = process.env.TINE_APP || path.resolve("target/debug/tine");
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4598);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4599);
const target = await startWebdriverApplication(app, env, nativePort);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"), { env: target.env, stdio: ["ignore", log, log], detached: true });
const subjects = () => git("log", "--format=%s").split("\n");
let browser;
try {
  await sleep(2500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: tauriCapabilities(app, "default", process.platform, target.debuggerAddress) });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1280, 900);
  await browser.$(".page-title, .ls-block").waitForExist({ timeout: 30_000 });

  await browser.$('button.icon-btn[title^="Settings"]').click();
  await browser.waitUntil(() => browser.execute(() => {
    const item = [...document.querySelectorAll(".settings-nav-item")].find((node) => node.textContent?.trim() === "Graph");
    item?.click();
    return !!document.querySelector("[data-git-policy]");
  }), { timeout: 15_000, timeoutMsg: "Settings → Graph never offered the git policy control" });
  const shown = await browser.execute(() => document.querySelector('[data-setting-label="Commit changes to git"]')?.textContent ?? "");
  if (!shown.includes("Branch: main")) throw new Error(`Settings did not name the branch: ${JSON.stringify(shown)}`);
  await browser.execute(() => {
    const select = document.querySelector("[data-git-policy]");
    select.value = "commit";
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await browser.$("[data-git-interval]").waitForExist({ timeout: 10_000, timeoutMsg: "the interval never appeared after choosing Commit automatically" });
  console.log("PASS: Settings → Graph names the branch and turns on Commit automatically");
  await browser.keys("Escape");

  fs.writeFileSync(path.join(graph, "pages/Other.md"), "- edited by another program\n");
  await openPageByName(browser, "Alpha");
  await browser.execute(() => document.querySelector("h1.page-title")?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, view: window })));
  await browser.$(".page-title-input").waitForExist({ timeout: 5000, timeoutMsg: "rename input never appeared" });
  await browser.execute(() => {
    const input = document.querySelector(".page-title-input");
    input.focus(); input.value = "Gamma";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Gamma", { timeout: 15_000, timeoutMsg: "rename never landed on Gamma" });
  await browser.waitUntil(() => subjects()[0] === 'Rename page "Alpha" to "Gamma"', { timeout: 15_000, interval: 200,
    timeoutMsg: `no rename commit; log is ${JSON.stringify(subjects())}` });
  const history = subjects();
  if (JSON.stringify(history) !== JSON.stringify(['Rename page "Alpha" to "Gamma"', "Tine: autosave", "initial"])) throw new Error(`unexpected history ${JSON.stringify(history)}`);
  if (git("show", "HEAD~1:pages/Other.md") !== "- edited by another program") throw new Error("the pre-rename autosave did not hold the other program's edit");
  if (git("ls-tree", "--name-only", "HEAD", "pages/").split("\n").sort().join(",") !== "pages/Gamma.md,pages/Other.md") throw new Error(`rename commit tree: ${git("ls-tree", "--name-only", "HEAD", "pages/")}`);
  const status = git("status", "--porcelain");
  if (status) throw new Error(`work tree not clean after the rename commit: ${status}`);
  console.log("PASS: rename commits the pending external edit, then the rename, and leaves the tree clean");
} catch (error) {
  await browser?.saveScreenshot(path.join(run, "failure.png")).catch(() => {});
  fs.writeFileSync(path.join(run, "failure.json"), JSON.stringify({ journey: "git autocommit", expected: "Settings enables autocommit; a rename is bracketed by commits", observed: String(error), classification: "ambiguous", commit: process.env.TESTED_COMMIT || "working tree" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  stopWebdriverApplication(target); fs.closeSync(log);
}
