// GH #481: browsing mentions creates no journal files; the first body edit does.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { openJournals, openPageByLink } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME, "bin/tauri-driver");
const ARTIFACT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/sw-upcoming2/native"));
fs.mkdirSync(ARTIFACT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(ARTIFACT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
const config = "{:ref/linked-references-collapsed-threshold 0 :tine/guide-announced? true}\n";
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), config);
fs.writeFileSync(path.join(GRAPH, "pages/Plan.md"), "- Remember [[2000-01-03]] and [[2099-01-03]]\n");
const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4871);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim(), binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), journey: "fileless journal mentions and opt-in Upcoming", expected: "no Upcoming by default; adding its sections macro shows future references; config bytes unchanged; browsing creates no journals", steps: [] };
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openJournals(browser);
  await browser.waitUntil(() => browser.execute(() => {
    const day = [...document.querySelectorAll(".page-section")].find(el => el.textContent.includes("Jan 3rd, 2000"));
    day?.scrollIntoView({ block: "center" });
    return day?.querySelector(".references-count")?.textContent === "1";
  }), { timeout: 30_000, interval: 150, timeoutMsg: "fileless past day did not show its counted references" });
  if (await browser.$(".journal-upcoming").isExisting()) throw new Error("Upcoming appeared without a sections macro");
  proof.steps.push("Upcoming absent by default with a referenced future day");
  await browser.$(".today-sections-customize").scrollIntoView({ block: "center" });
  await browser.$(".today-sections-customize").click();
  await browser.$(".journal-sections-header").waitForExist({ timeout: 20_000 });
  await browser.saveScreenshot(path.join(TMP, "sections-header.png"));
  await browser.$(".journal-sections-header button").click();
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".right-sidebar")?.textContent.includes("Journals, tasks, and scheduling")), { timeout: 20_000, timeoutMsg: "Guide did not open in right sidebar" });
  await browser.$(".main-content .page-trailing-block-target").scrollIntoView({ block: "center" });
  await browser.$(".main-content .page-trailing-block-target").click();
  await browser.$(".main-content textarea.block-editor").waitForExist({ timeout: 10_000 });
  // Send literal text through the editor's input path; key-by-key brace
  // pairing can consume a closing brace in WebKit's synthetic key stream.
  await browser.$(".main-content textarea.block-editor").addValue("{{upcoming}}");
  if (await browser.$(".main-content textarea.block-editor").getValue() !== "{{upcoming}}") throw new Error("Harness did not enter the literal Upcoming macro");
  await browser.$(".main-content h1.page-title").click();
  const sectionsFile = fs.readdirSync(path.join(GRAPH, "pages")).find(file => file.toLowerCase() === "journal sections.md");
  await waitForFileText(path.join(GRAPH, "pages", sectionsFile), text => text.split("\n").includes("- {{upcoming}}"));
  await openJournals(browser);
  await browser.$(".journal-upcoming").waitForExist({ timeout: 20_000 });
  proof.steps.push("ordinary sections-page editing saved the macro and showed Upcoming; Guide opened in the right sidebar");
  const collapsed = await browser.execute(() => {
    const group = document.querySelector(".journal-upcoming");
    if (!group || group.hasAttribute("open")) return false;
    group.querySelector("summary").click();
    return true;
  });
  if (!collapsed) throw new Error("Upcoming was absent or initially expanded");
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".journal-upcoming details")), { timeout: 10_000 });
  await browser.execute(() => document.querySelector(".journal-upcoming details summary").click());
  await browser.waitUntil(() => browser.execute(() => {
    const group = document.querySelector(".journal-upcoming");
    group.scrollIntoView({ block: "center" });
    return group.querySelector(".references-count")?.textContent === "1";
  }), { timeout: 20_000, interval: 150 });
  if (fs.readdirSync(path.join(GRAPH, "journals")).length) throw new Error("viewing the feed wrote a journal file");
  proof.steps.push("past counted references and collapsed Upcoming rendered without journal files");
  await openPageByLink(browser, "Jan 3rd, 2099");
  if (fs.readdirSync(path.join(GRAPH, "journals")).length) throw new Error("opening the day wrote a journal file");
  await browser.execute(() => {
    for (const close of document.querySelectorAll(".toast-sticky .toast-close")) close.click();
  });
  await browser.$(".main-content .page-section .block-content-wrapper").click();
  const editor = await browser.$(".main-content textarea.block-editor");
  await editor.waitForExist({ timeout: 10_000 });
  await browser.keys("first future entry");
  await browser.$(".main-content h1.page-title").click();
  await waitForFileText(path.join(GRAPH, "journals/2099_01_03.md"), text => text.includes("first future entry"));
  if (fs.readdirSync(path.join(GRAPH, "journals")).join() !== "2099_01_03.md") throw new Error("first edit wrote another day");
  proof.steps.push("future title opened its editable day and first typing created only that journal");
  if (fs.readFileSync(path.join(GRAPH, "logseq/config.edn"), "utf8") !== config) throw new Error("config.edn bytes changed");
  proof.steps.push("config.edn is byte-unchanged");
  console.log("PASS", JSON.stringify(proof));
} catch (error) {
  proof.observed = String(error);
  proof.classification = "ambiguous; inspect DOM and driver log";
  if (browser) {
    fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
    await browser.saveScreenshot(path.join(TMP, "failure.png")).catch(() => {});
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
