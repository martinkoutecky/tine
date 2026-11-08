// GH #881: real engine + shared list rendering in a routed page, sidebar and
// block embed. Synthetic graph only; reading these options must not write it.
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import fs from "node:fs";
import path from "node:path";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { installAccountHome } from "./lib/e2e-account-home.mjs";
import { saveJourneyScreenshot } from "./lib/e2e-comment-structure.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
const ARTIFACTS = path.resolve(process.env.E2E_ARTIFACT_DIR || `${ROOT}/target/artifacts/og-breadcrumb-881/native`);
fs.mkdirSync(ARTIFACTS, { recursive: true });
const TMP = fs.mkdtempSync(`${ARTIFACTS}/run-`);
const GRAPH = `${TMP}/graph`;
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(`${TMP}/${dir}`);
installAccountHome(TMP, ARTIFACTS);
const snapshots = new Map();
function seed(name, text) {
  const file = `${GRAPH}/${name}`;
  fs.writeFileSync(file, text);
  snapshots.set(file, text);
}
seed("logseq/config.edn", "{}\n");
seed("pages/Source.md", "- Shopping parent\n  - TODO buy apples\n  - TODO buy pears\n");
const id = "18810000-0000-4000-8000-000000000001";
const query = '[:find (pull ?b [*]) :where [?b :block/marker "TODO"]]';
const block = (options, extra = "") => `- #+BEGIN_QUERY\n  {:title "Shopping" :query ${query}${options}}\n  #+END_QUERY\n${extra}`;
seed("pages/Hidden.md", `- {{query {:query ${query} :breadcrumb-show? false}}}\n  id:: ${id}\n`);
seed("pages/Shown.md", `- {{query {:query ${query}}}}\n`);
seed("pages/Flat.md", `- {{query {:query ${query} :group-by-page? false :breadcrumb-show? false}}}\n`);
seed("pages/BeginList.md", block(" :table-view? false :breadcrumb-show? false"));
seed("pages/Table.md", block(" :table-view? true :breadcrumb-show? false"));
seed("pages/Embedded.md", `- {{embed ((${id}))}}\n`);

const driverPort = Number(process.env.E2E_DRIVER_PORT || 4874);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4875);
const env = { ...process.env, HOME: `${TMP}/home`, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/data`, XDG_CONFIG_HOME: `${TMP}/config`, XDG_CACHE_HOME: `${TMP}/cache`, XDG_STATE_HOME: `${TMP}/state`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(`${ARTIFACTS}/driver.log`, "w");
const td = spawn(process.env.TAURI_DRIVER || `${process.env.CARGO_HOME}/bin/tauri-driver`, ["--port", String(driverPort),
  "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
{ env, stdio: ["ignore", log, log], detached: true });
let browser;
let step = "startup";
const observations = [];
try {
  browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error",
    connectionRetryCount: 5, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP || `${ROOT}/target/release/tine` } } });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
  async function check(name, scope, { trail = false, heading = true, table = false } = {}) {
    step = name;
    await browser.waitUntil(() => browser.execute((selector, table) => {
      const query = document.querySelector(`${selector} .query-block`);
      return query?.textContent.includes("buy apples") && query.textContent.includes("buy pears")
        && (table || query.querySelectorAll(".ls-block:not(.ref-block)").length >= 2);
    }, scope, table), { timeout: 30_000, interval: 150, timeoutMsg: `${name}: missing hydrated matching blocks` });
    if (trail) await browser.waitUntil(() => browser.execute(selector =>
      !!document.querySelector(`${selector} .query-block .ref-breadcrumb`)?.textContent.trim(), scope),
    { timeout: 15_000, interval: 100, timeoutMsg: `${name}: expected ancestor trail never hydrated` });
    const state = await browser.execute(selector => {
      const query = document.querySelector(`${selector} .query-block`);
      const shown = selector => [...query.querySelectorAll(selector)].some(node => {
        const style = getComputedStyle(node);
        return node.getClientRects().length > 0 && style.visibility !== "hidden" && style.opacity !== "0";
      });
      return { text: query.textContent, trail: shown(".ref-breadcrumb"),
        heading: shown(".query-page, .query-crumb"), table: shown(".query-table") };
    }, scope);
    observations.push({ name, ...state });
    await saveJourneyScreenshot(browser, `${ARTIFACTS}/${name}.png`);
    if (state.trail !== trail || state.heading !== heading || state.table !== table) {
      throw Object.assign(new Error(`${name}: expected ${JSON.stringify({ trail, heading, table })}, observed ${JSON.stringify(state)}`), { classification: "product" });
    }
  }
  await openPageByName(browser, "Shown");
  await check("shown-main", ".main-content", { trail: true });
  await openPageByName(browser, "Hidden");
  await check("hidden-main", ".main-content");
  await openPageByName(browser, "Flat");
  await check("hidden-ungrouped", ".main-content", { heading: false });
  await openPageByName(browser, "BeginList");
  await check("hidden-begin-list", ".main-content");
  await openPageByName(browser, "Table");
  await check("table-unaffected", ".main-content", { heading: false, table: true });
  await openPageByName(browser, "Embedded");
  await check("hidden-embed", ".main-content");
  await openPageByName(browser, "Hidden");
  await browser.performActions([{ type: "key", id: "keyboard", actions: [{ type: "keyDown", value: "\uE008" }] }]);
  try { await browser.$("h1.page-title").click(); } finally { await browser.releaseActions(); }
  await check("hidden-sidebar", ".right-sidebar-body");
  for (const [file, before] of snapshots) {
    if (fs.readFileSync(file, "utf8") !== before) throw Object.assign(new Error(`Reading query changed ${path.basename(file)}`), { classification: "product" });
  }
  fs.writeFileSync(`${ARTIFACTS}/observations.json`, JSON.stringify(observations, null, 2));
  console.log("PASS GH #881: main, ungrouped, table, embed, sidebar; authored files unchanged");
} catch (error) {
  if (browser) await saveJourneyScreenshot(browser, `${ARTIFACTS}/failure.png`).catch(() => {});
  const state = await browser?.execute(() => ({ text: document.body.innerText,
    query: [...document.querySelectorAll(".query-block")].map(node => node.outerHTML) })).catch(() => null);
  fs.writeFileSync(`${ARTIFACTS}/failure.json`, JSON.stringify({ testedCommit,
    journey: "GH #881", step, expected: "advanced query respects authored breadcrumb visibility",
    observed: String(error), state, classification: error.classification || (step === "startup" ? "infrastructure" : "ambiguous"), observations }, null, 2));
  throw error;
} finally {
  await browser?.deleteSession().catch(() => {});
  try { process.kill(-td.pid, "SIGTERM"); } catch { /* Already exited. */ }
  fs.closeSync(log);
}
