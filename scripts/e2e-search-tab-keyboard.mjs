// GH #697: native Search tab typing, roving highlight, and keyboard navigation.
// GH #796: Settings > Editor > Advanced > "Search results open in": New tab (default,
// GH #416) opens the highlighted page beside Search; This tab opens it in the Search tab.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/keys-696-697"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), '{:shortcuts {:go/search-tab "mod+alt+f"}}\n');
for (const suffix of ["Alpha", "Beta", "Gamma"]) fs.writeFileSync(path.join(graph, "pages", `Keyboard ${suffix}.md`), `- Body for ${suffix}\n`);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(out, "data"),
  XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4870);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  workingDiff: true, app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"),
  journey: "Open Search tab through bound shortcut; type; ArrowDown then ArrowUp returns to the search box; ArrowDown twice; Enter opens second displayed page in a new tab; Settings: Search results open in = This tab; Enter on the first result navigates the Search tab itself" };
let browser;
let stage = "startup";
try {
  await sleep(1500);
  // WebKitGTK permits one automation session. Another lane may own it briefly.
  for (let attempt = 0; ; attempt++) {
    try {
      browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
        connectionRetryTimeout: 30000, capabilities: tauriCapabilities(app, "search-tab-keyboard") });
      break;
    } catch (error) {
      if (!String(error).includes("Maximum number of active sessions") || attempt >= 11) throw error;
      console.log("Another native journey owns WebKit; waiting 10 seconds before retry.");
      await sleep(10000);
    }
  }
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await browser.$(".page-title").waitForDisplayed({ timeout: 30000 });
  stage = "open Search tab";
  await browser.keys(["Control", "Alt", "f"]);
  const input = await browser.$(".query-workspace-source");
  await input.waitForDisplayed({ timeout: 10000 });
  await input.click();
  for (const key of "Keyboard") await browser.keys([key]);
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".query-workspace button[data-query-result]").length === 3),
    { timeout: 10000, timeoutMsg: "typed query did not return the three fixture pages" });
  const rows = await browser.execute(() => [...document.querySelectorAll(".query-workspace button[data-query-result]")]
    .map(row => row.querySelector(".search-result-excerpt")?.textContent));
  stage = "ArrowUp on the first row returns to the search box";
  await browser.keys(["ArrowDown"]);
  await browser.keys(["ArrowUp"]);
  const back = await browser.execute(() => ({ input: document.activeElement?.classList.contains("query-workspace-source"),
    active: document.querySelectorAll(".query-workspace button[data-query-result].active").length }));
  if (!back.input || back.active !== 0) throw Error(`ArrowUp on the first row did not return to the search box: ${JSON.stringify(back)}`);
  stage = "ArrowDown twice";
  await browser.keys(["ArrowDown"]);
  await browser.keys(["ArrowDown"]);
  const selected = await browser.execute(() => ({ text: document.activeElement?.querySelector(".search-result-excerpt")?.textContent,
    active: document.activeElement?.classList.contains("active"), tabIndex: document.activeElement?.tabIndex }));
  if (selected.text !== rows[1] || !selected.active || selected.tabIndex !== 0) throw Error(`Expected second accessible highlighted row: ${JSON.stringify({ rows, selected })}`);
  try { execFileSync("import", ["-window", "root", path.join(out, "highlight.png")], { env }); } catch {}
  const tabState = () => browser.execute(() => ({ count: document.querySelectorAll(".tab-strip-scroll .tab[data-tab-id]").length,
    active: document.querySelector(".tab-strip-scroll .tab.active[data-tab-id]")?.getAttribute("data-tab-id") ?? null }));
  const tabsBefore = await tabState();
  stage = "Enter opens highlighted page";
  await browser.keys(["Enter"]);
  await browser.waitUntil(async () => (await browser.$(".page-title").getText()).trim() === rows[1],
    { timeout: 10000, timeoutMsg: "Enter did not open the highlighted page" });
  // Default (GH #416): the page opened in a NEW foreground tab and Search stays beside it.
  const tabsAfterDefault = await tabState();
  if (tabsAfterDefault.count !== tabsBefore.count + 1 || tabsAfterDefault.active === tabsBefore.active)
    throw Error(`Default Enter should open a new tab: ${JSON.stringify({ tabsBefore, tabsAfterDefault })}`);
  try { execFileSync("import", ["-window", "root", path.join(out, "opened.png")], { env }); } catch {}
  stage = "GH #796: choose This tab in Settings";
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForExist({ timeout: 5000 });
  await browser.$("//button[contains(@class,'settings-nav-item') and normalize-space(.)='Editor']").click();
  const advanced = await browser.$(".settings-advanced-toggle");
  if ((await advanced.getAttribute("aria-expanded")) !== "true") await advanced.click();
  const choice = await browser.$('[data-setting-label="Search results open in"] select');
  await choice.waitForExist({ timeout: 5000 });
  await choice.scrollIntoView();
  if ((await choice.getValue()) !== "tab") throw Error("Search results open in should default to New tab");
  await choice.selectByAttribute("value", "here");
  await browser.waitUntil(async () => (await choice.getValue()) === "here", { timeout: 5000 });
  await browser.$("//div[contains(@class,'settings-modal')]//button[normalize-space(.)='✕']").click();
  await browser.$(".settings-modal").waitForExist({ timeout: 5000, reverse: true });
  stage = "GH #796: return to the Search tab";
  await browser.execute((id) => document.querySelector(`.tab-strip-scroll .tab[data-tab-id="${id}"]`)?.click(), tabsBefore.active);
  const searchInput = await browser.$(".query-workspace-source");
  await searchInput.waitForDisplayed({ timeout: 10000 });
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".query-workspace button[data-query-result]").length === 3),
    { timeout: 10000, timeoutMsg: "Search tab lost its results" });
  const tabsInSearch = await tabState();
  await searchInput.click();
  await browser.keys(["ArrowDown"]);
  // The first row differs from the page already open in the second tab, so tab reuse cannot explain a switch.
  stage = "GH #796: Enter opens the first result in this tab";
  await browser.keys(["Enter"]);
  await browser.waitUntil(async () => (await browser.$(".page-title").getText()).trim() === rows[0],
    { timeout: 10000, timeoutMsg: "Enter did not open the first result" });
  const tabsInPlace = await tabState();
  if (tabsInPlace.count !== tabsInSearch.count || tabsInPlace.active !== tabsBefore.active)
    throw Error(`This tab should navigate the Search tab itself: ${JSON.stringify({ tabsInSearch, tabsInPlace })}`);
  if (await browser.$(".query-workspace").isExisting()) throw Error("Search workspace still shown after opening in this tab");
  try { execFileSync("import", ["-window", "root", path.join(out, "opened-here.png")], { env }); } catch {}
  proof.outcome = "PASS"; proof.rows = rows; proof.opened = rows[1]; proof.openedHere = rows[0];
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS GH #697: ${out}`);
} catch (error) {
  try { execFileSync("import", ["-window", "root", path.join(out, "failure.png")], { env }); } catch {}
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "Second displayed page opens from Search tab after typing, ArrowDown twice and Enter", observed: String(error.stack),
    classification: "ambiguous; inspect driver log and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
