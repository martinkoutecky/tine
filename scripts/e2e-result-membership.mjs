// GH #659: a routed named-page query cannot evict its editor during autosave.
// The second query witnesses an actual saved-revision recomputation while the
// first keeps the edited occurrence. Final bytes and membership agree on blur.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const ARTIFACT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/og-w5-refs/native"));
fs.mkdirSync(ARTIFACT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(ARTIFACT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
const TASK = path.join(GRAPH, "pages/Tasks.md");
fs.writeFileSync(TASK, "- TODO my task\n");
fs.writeFileSync(path.join(GRAPH, "pages/Queries.md"), [
  "- {{tine-query @block AND task = 'TODO'}}",
  "- {{tine-query @block AND content like '%CANC%'}}",
  "",
].join("\n"));
const now = new Date();
const day = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${day}.md`), "- Open [[Queries]]\n");
const env = {
  ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const port = Number(process.env.E2E_DRIVER_PORT || 4792);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { app: APP, binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), graph: GRAPH, steps: [] };
async function waitQuery(index, text) {
  await browser.waitUntil(() => browser.execute((i, expected) => {
    const query = document.querySelectorAll(".main-content .page-blocks .query-block")[i];
    query?.scrollIntoView({ block: "center" });
    return query?.textContent.includes(expected) ?? false;
  }, index, text), { timeout: 20_000, interval: 150, timeoutMsg: `query ${index} never showed ${text}` });
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await openPageByName(browser, "Queries");
  await waitQuery(0, "my task");
  await browser.execute(() => {
    for (const close of document.querySelectorAll(".toast-sticky .toast-close")) close.click();
  });
  await browser.$(".main-content .query-block .live-ref-group .block-content-wrapper").click();
  const selector = ".main-content .query-block textarea.block-editor";
  await browser.$(selector).waitForExist({ timeout: 10_000 });
  // Native select-all and typing keep one editor session. WebKit's WebDriver
  // clearElement command blurs an empty block before elementSendKeys can run.
  await browser.$(selector).click();
  await browser.keys(["Control", "a"]);
  await browser.keys("CANC my task");
  await waitForFileText(TASK, (text) => text.includes("CANC my task"));
  await waitQuery(1, "CANC my task");
  // Locate the current editor afresh after the witness query renders.
  const retained = await browser.execute(() => {
    const query = document.querySelectorAll(".main-content .page-blocks .query-block")[0];
    const editor = query?.querySelector("textarea.block-editor");
    return editor?.value;
  });
  if (retained !== "CANC my task") throw new Error(`autosave evicted or replaced the editor: ${JSON.stringify(retained)}`);
  proof.steps.push("saved partial marker recomputed the witness query while TODO query retained its editor");
  await browser.$(selector).click();
  await browser.keys(["Control", "a"]);
  await browser.keys("CANCELED my task");
  await browser.$(".main-content h1.page-title").click();
  await waitForFileText(TASK, (text) => text.includes("CANCELED my task") && !text.includes("CANC my task"));
  await browser.waitUntil(() => browser.execute(() => {
    const query = document.querySelectorAll(".main-content .page-blocks .query-block")[0];
    return !!query && !query.querySelector("textarea.block-editor") && !query.textContent.includes("my task");
  }), { timeout: 20_000, interval: 150, timeoutMsg: "settled canceled task remained in the TODO query after blur" });
  if (fs.readFileSync(TASK, "utf8").trim() !== "- CANCELED my task") throw new Error("final source bytes differ from the completed edit");
  proof.steps.push("blur saved the full CANCELED marker and removed the row from the TODO answer");
  // The semantic observation is saved bytes plus the rendered answer. Native
  // WebKit screenshot requests can stall after a keyed answer is removed.
  fs.writeFileSync(path.join(TMP, "after-blur-dom.html"), await browser.execute(() => document.body.innerHTML));
  console.log(`PASS ${JSON.stringify(proof)}`);
} catch (error) {
  proof.error = String(error);
  proof.classification = "ambiguous; inspect DOM and driver log";
  if (browser) {
    fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  }
  throw error;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
