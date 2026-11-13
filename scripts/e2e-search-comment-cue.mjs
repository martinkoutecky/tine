// COMMENT-OUT-V2: native palette labels and comment-target navigation.
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { openPageByName, currentPageTitle } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || "target/artifacts/search-comment-cue-1113/native");
const testedCommit = process.env.TINE_SOURCE_REVISION || execFileSync("git", ["-C", process.cwd(), "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
fs.mkdirSync(artifacts, { recursive: true });
const scratch = fs.mkdtempSync(path.join(artifacts, "scratch-"));
const graph = path.join(scratch, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(scratch, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(graph, "pages/Start.md"), "- Start here\n");
const source = "- Host passage\n  - CueWitness root\n    tine.quote:: Host passage\n    - CueWitness reply\n- CueWitness body\n";
fs.writeFileSync(path.join(graph, "pages/Annotations.md"), source);
const env = { ...process.env, TINE_GRAPH: graph, HOME: path.join(scratch, "home"),
  XDG_DATA_HOME: path.join(scratch, "data"), XDG_CONFIG_HOME: path.join(scratch, "config"),
  XDG_CACHE_HOME: path.join(scratch, "cache"), XDG_STATE_HOME: path.join(scratch, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const fixture = accountHomeFixture(scratch, artifacts, env);
const port = Number(process.env.E2E_DRIVER_PORT || 4912);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4913);
const log = fs.openSync(path.join(artifacts, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver"),
  webdriverServerArgs(port, nativePort, process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"),
  { env: fixture.env, stdio: ["ignore", log, log], detached: true });
let browser;
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 3, connectionRetryTimeout: 20_000,
    capabilities: tauriCapabilities(path.resolve(process.env.TINE_APP || "target/release/tine"), "search-comment-cue") });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
  await assertAppLease(fixture);
  await browser.setWindowSize(1800, 1000);
  await openPageByName(browser, "Start");
  await browser.keys(["Control", "k"]);
  await browser.$(".switcher-input").waitForExist();
  for (const key of "CueWitness") await browser.keys([key]);
  let rows;
  await browser.waitUntil(async () => {
    rows = await browser.execute(() => [...document.querySelectorAll(".switcher-row.block-result")].map(row => ({
      text: row.querySelector(".search-result-excerpt")?.textContent,
      kind: row.querySelector(".switcher-kind")?.textContent,
      context: row.querySelector(".search-result-context")?.textContent,
    })));
    return rows.length === 3;
  }, { timeout: 15_000, timeoutMsg: "Palette did not show root, reply and body hits" });
  for (const row of rows) {
    const expected = row.text.includes("body") ? "block" : "comment";
    if (row.kind !== expected) throw new Error(`Expected ${expected}: ${JSON.stringify(row)}`);
    if (expected === "comment" && !row.context.includes("Host passage")) throw new Error("Missing host breadcrumb");
  }
  await browser.saveScreenshot(path.join(artifacts, "palette.png"));
  let reply;
  for (const row of await browser.$$(".switcher-row.block-result")) {
    if ((await row.getText()).includes("CueWitness reply")) { reply = row; break; }
  }
  if (!reply) throw new Error("Reply result disappeared before activation");
  await reply.click();
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Annotations", { timeout: 15_000 });
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".comment-card, .zoomed-block")]
    .some(el => el.textContent.includes("CueWitness reply") && el.getBoundingClientRect().height > 0)),
  { timeout: 10_000, timeoutMsg: "Comment target did not reveal its thread" });
  await browser.saveScreenshot(path.join(artifacts, "opened.png"));
  if (fs.readFileSync(path.join(graph, "pages/Annotations.md"), "utf8") !== source) throw new Error("Navigation changed graph bytes");
  fs.writeFileSync(path.join(artifacts, "receipt.json"), JSON.stringify({ testedCommit, rows, clickThrough: "reply visible", unchanged: true }, null, 2));
  console.log("e2e-search-comment-cue: PASS");
} catch (error) {
  await browser?.saveScreenshot(path.join(artifacts, "failure.png")).catch(() => {});
  fs.writeFileSync(path.join(artifacts, "failure.json"), JSON.stringify({ testedCommit, journey: "palette comment hit and reply navigation", expected: "comment cues and visible reply", observed: String(error), screenshot: "failure.png", classification: "ambiguous" }));
  throw error;
} finally {
  fixture.sample();
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
  fixture.close();
  fixture.verify();
}
