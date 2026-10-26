import os from "node:os";
// GH #741: typing DOING writes the :LOGBOOK: clock-in; the rest of the typed text stays on the task line, the
// block clocks in once, and a damaged logbook (CLOCK line outside a recognised drawer) renders instead of
// replacing the page with "This page could not be displayed".
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const DRIVER = process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "", "bin/tauri-driver");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(os.tmpdir(), "tine-e2e-gh741"));
fs.mkdirSync(artifacts, { recursive: true });
const TMP = fs.mkdtempSync(path.join(artifacts, "run-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
const clock = "CLOCK: [2026-10-21 Wed 08:54:45]";
// The two literal samples of the report, as a user's file would hold them.
fs.writeFileSync(path.join(GRAPH, "pages/Typing.md"), "- seed\n");
fs.writeFileSync(path.join(GRAPH, "pages/AfterEnd.md"), `- DOING\n  :LOGBOOK:\n  ${clock}\n  :END:  test\n- sibling one\n`);
fs.writeFileSync(path.join(GRAPH, "pages/Backtick.md"), `- \`DOING\n  :LOGBOOK:\n  ${clock}\n  :END:  \`\n- sibling two\n`);
const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4741);
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let stage = "startup";
const testedBase = process.env.TINE_TESTED_BASE || "release/0.8 batch/fix-gh741";
const connect = () => remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
  capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
// E2E_PAGE=AfterEnd|Backtick narrows the damaged stage. E2E_ONLY=typing|damaged runs one stage (a failed page region blocks navigation, so the stages cannot share a run before the fix).
const only = process.env.E2E_ONLY || "";
const failureRegion = () => browser.execute(() => !!document.querySelector(".region-failure"));
try {
  browser = await connect();
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });

  stage = "damaged pages render";
  const problems = [];
  for (const [name, sibling] of (only === "typing" ? [] : [["AfterEnd", "sibling one"], ["Backtick", "sibling two"]]).filter(([n]) => !process.env.E2E_PAGE || n === process.env.E2E_PAGE)) {
   try {
    stage = `open damaged ${name}`;
    await openPageByName(browser, name);
    const row = await browser.$(`//div[contains(@class, 'block-content') and contains(normalize-space(.), '${sibling}')]`);
    await row.waitForDisplayed({ timeout: 10_000, timeoutMsg: `${name}: page did not render its blocks (region failure: ${await failureRegion()})` });
    if (await failureRegion()) throw new Error(`${name}: a failure region is showing`);
    const text = await browser.execute(() => document.body.innerText);
    if (!text.includes("DOING")) throw new Error(`${name}: the damaged block's text is not shown`);
   } catch (error) {
    const region = await browser.execute(() => document.querySelector(".region-failure")?.textContent ?? null).catch(() => null);
    problems.push(`${name}: ${error.message.split("\n")[0]}${region ? ` [page shows: ${region}]` : ""}`);
   }
  }

  stage = "type DOING with real key events";
  if (only !== "damaged") {
  await openPageByName(browser, "Typing");
  try {
  const seed = await browser.$("//div[contains(@class, 'block-content') and normalize-space(.) = 'seed']");
  await seed.waitForDisplayed({ timeout: 10_000 });
  await seed.click();
  const editor = await browser.$("textarea.block-editor");
  await editor.waitForExist({ timeout: 10_000 });
  await browser.keys(["Control", "a"]);
  await browser.keys(["Control"]);
  await browser.keys([..."DOING test"]);
  const seen = await browser.execute(() => {
    const t = document.querySelector("textarea.block-editor");
    return { value: t.value, start: t.selectionStart, end: t.selectionEnd };
  });
  if (!/^DOING test\n:LOGBOOK:\nCLOCK: \[[^\]]+\]\n:END:$/.test(seen.value)) throw new Error(`typed text did not stay on the task line / clocked in other than once: ${JSON.stringify(seen)}`);
  if (seen.start !== "DOING test".length || seen.end !== seen.start) throw new Error(`caret left the task line: ${JSON.stringify(seen)}`);
  await browser.keys(["Escape"]);
  const typing = path.join(GRAPH, "pages/Typing.md");
  await browser.waitUntil(() => /DOING test\n\s*:LOGBOOK:/.test(fs.readFileSync(typing, "utf8")), { timeout: 10_000, timeoutMsg: `Typing.md did not hold the task line: ${fs.readFileSync(typing, "utf8")}` });
  if (await failureRegion()) throw new Error("page failed after typing DOING");
  const saved = fs.readFileSync(typing, "utf8");
  if ((saved.match(/CLOCK:/g) || []).length !== 1 || /:END:\s*\S/.test(saved)) throw new Error(`saved file wrong:\n${saved}`);
  } catch (error) { problems.push(`typing: ${error.message.split("\n")[0].slice(0, 300)}`); }
  }
  if (problems.length) throw new Error(problems.join("\n"));

  fs.writeFileSync(path.join(TMP, "proof.json"), JSON.stringify({ testedBase, app: APP, outcome: "PASS" }));
  console.log(`PASS GH 741: ${TMP}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(TMP, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(TMP, "failure.txt"), `Tested: ${testedBase}\nStage: ${stage}. Observed: ${error.stack}\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
