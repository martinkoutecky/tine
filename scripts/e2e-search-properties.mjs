// GH #870: literal Ctrl+K property search, highlighted evidence, and navigation.
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { openPageByName, currentPageTitle } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const ARTIFACTS = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/og-search-props-870/native"));
fs.mkdirSync(ARTIFACTS, { recursive: true });
const TMP = fs.mkdtempSync(path.join(ARTIFACTS, "scratch-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(TMP, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(GRAPH, "pages/Starting Page.md"), "- Start here\n");
const sources = {
  "Property Host.md": "category:: MdPageWitness870\n\n" + "- Filler before the property owner\n".repeat(60) + "- Countercheck\n  reviewer:: MdValueWitness870\n  id:: 87087087-0870-4870-8870-870870870870\n",
  "Org Host.org": "#+category: OrgPageWitness870\n\n" + "* Filler before the property owner\n".repeat(60) + "* Org countercheck\n:PROPERTIES:\n:reviewer: OrgValueWitness870\n:END:\n",
};
for (const [name, source] of Object.entries(sources)) fs.writeFileSync(path.join(GRAPH, "pages", name), source);
const env = { ...process.env, HOME: path.join(TMP, "home"), TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"),
  XDG_CACHE_HOME: path.join(TMP, "cache"), XDG_STATE_HOME: path.join(TMP, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const fixture = accountHomeFixture(TMP, ARTIFACTS, env);
const port = Number(process.env.E2E_DRIVER_PORT || 4870);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4871);
const td = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const log = fs.openSync(path.join(ARTIFACTS, "driver.log"), "w");
const driver = spawn(td, webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: fixture.env, stdio: ["ignore", log, log], detached: true });
const receipt = { testedCommit: process.env.TINE_SOURCE_REVISION || execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), journey: "Ctrl+K finds user properties", observations: [] };
let browser;
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 3, connectionRetryTimeout: 20_000,
    capabilities: tauriCapabilities(APP, "search-properties") });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
  await assertAppLease(fixture);
  for (const [query, page, body, header] of [
    ["MdValueWitness870", "Property Host", "Countercheck", false],
    ["MdPageWitness870", "Property Host", "Countercheck", true],
    ["OrgValueWitness870", "Org Host", "Org countercheck", false],
    ["OrgPageWitness870", "Org Host", "Org countercheck", true],
  ]) {
    await openPageByName(browser, "Starting Page");
    await browser.keys(["Control", "k"]);
    await browser.$(".switcher-input").waitForExist({ timeout: 5000 });
    // Keystrokes, rather than injected input events, exercise the reported path.
    for (const key of query) await browser.keys([key]);
    let observation;
    await browser.waitUntil(async () => {
      observation = await browser.execute((wanted) => {
        if (document.querySelector('#switcher-results [role="status"]')) return null;
        const row = [...document.querySelectorAll(".switcher-row.block-result")]
          .find(row => row.querySelector(".search-result-excerpt")?.textContent?.includes(wanted));
        return row ? { excerpt: row.querySelector(".search-result-excerpt")?.textContent,
          marks: [...row.querySelectorAll("mark")].map(mark => mark.textContent) } : null;
      }, query);
      return observation?.marks?.some(mark => mark.includes(query));
    }, { timeout: 15_000, interval: 100, timeoutMsg: `No highlighted property result for ${query}` });
    // Select by semantic result identity, allowing ordinary page/Create rows
    // before it. Every activation remains literal keyboard input.
    const isSelected = () => browser.execute((wanted) => {
      const row = document.querySelector(".switcher-row.active");
      return row?.classList.contains("block-result") && row.textContent.includes(wanted);
    }, query);
    const rows = await browser.$$(".switcher-row[role='option']");
    for (let step = 0; step < rows.length && !(await isSelected()); step++) await browser.keys(["ArrowDown"]);
    if (!(await isSelected())) throw new Error("Keyboard navigation did not select the property result");
    await browser.keys(["Enter"]);
    await browser.waitUntil(async () => (await currentPageTitle(browser)) === page,
      { timeout: 15_000, timeoutMsg: `Property result did not open ${page}` });
    await browser.waitUntil(() => browser.execute((body, header, query) => {
      if (header) return document.querySelector(".main-content")?.textContent.includes(query);
      const blocks = [...document.querySelectorAll(".page-blocks .ls-block")];
      return blocks.some(block => {
        if (!block.textContent.includes(body)) return false;
        const rect = block.getBoundingClientRect();
        return rect.bottom > 0 && rect.top < window.innerHeight;
      });
    }, body, header, query), { timeout: 10_000, timeoutMsg: "The owning page/block was not revealed" });
    receipt.observations.push({ query, page, header, ...observation });
  }
  for (const [name, source] of Object.entries(sources)) {
    if (fs.readFileSync(path.join(GRAPH, "pages", name), "utf8") !== source) throw new Error(`Search modified ${name}`);
  }
  await browser.saveScreenshot(path.join(ARTIFACTS, "pass.png"));
  fs.writeFileSync(path.join(ARTIFACTS, "receipt.json"), JSON.stringify({ ...receipt, pageBytesUnchanged: true }, null, 2) + "\n");
  console.log("e2e-search-properties: PASS (Markdown/Org block and page properties; unchanged files)");
} catch (error) {
  const state = await browser?.execute(() => ({ title: document.querySelector("h1.page-title")?.textContent,
    switcher: document.querySelector(".switcher")?.textContent, body: document.querySelector(".page-blocks")?.textContent })).catch(() => null);
  await browser?.saveScreenshot(path.join(ARTIFACTS, "failure.png")).catch(() => {});
  fs.writeFileSync(path.join(ARTIFACTS, "failure.json"), JSON.stringify({ ...receipt,
    expected: "Highlighted property evidence opens its owning page/block", observed: String(error), state,
    classification: "ambiguous" }, null, 2) + "\n");
  throw error;
} finally {
  fixture.sample();
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  fixture.close();
  fixture.verify();
}
