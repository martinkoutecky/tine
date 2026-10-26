// GH #668: planning insertion keeps the original line visible in the open editor.
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName, openJournals } from "./lib/e2e-navigation.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/debug/tine");
const ARTIFACT = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/og-w6-deadline/native"));
fs.mkdirSync(ARTIFACT, { recursive: true });
const TMP = fs.mkdtempSync(path.join(ARTIFACT, "journey-"));
const GRAPH = path.join(TMP, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(TMP, dir));
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(GRAPH, "pages/Planning.md"), "- above\n- something\n");
const now = new Date();
const day = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(path.join(GRAPH, "journals", `${day}.md`), "- above\n- something\n");
const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(TMP, "data"), XDG_CONFIG_HOME: path.join(TMP, "config"), XDG_CACHE_HOME: path.join(TMP, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4796);
const DRIVER = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const log = fs.openSync(path.join(TMP, "driver.log"), "w");
const driver = spawn(DRIVER, ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
const proof = { app: APP, binarySha256: createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), observations: [] };
async function journey(surface, pane = ".main-content", command = "deadline", method = "done", options = false) {
  const id = await browser.execute((scope) => {
    const block = [...document.querySelectorAll(`${scope} .block-content-wrapper`)].find(el => el.textContent.trim().startsWith("something"));
    if (!block) throw new Error("fixture block absent");
    return block.closest(".ls-block").getAttribute("data-block-id");
  }, pane);
  await browser.$(`${pane} [data-block-id="${id}"] .block-content [data-so]`).click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 10000 });
  await browser.keys(["Control", "Home"]);
  await browser.keys(["End"]);
  await browser.keys(` /${command}`);
  await browser.waitUntil(() => browser.execute((label) => [...document.querySelectorAll(".ac-label")].some(el => el.textContent === label), command === "deadline" ? "Deadline" : "Scheduled"), { timeout: 10000 });
  await browser.keys(["Enter"]);
  await browser.$('.date-picker [data-day="12"]').waitForExist({ timeout: 10000 });
  await browser.$('.date-picker [data-day="12"]').click();
  if (options) {
    await browser.$(".dp-addtime").click();
    await browser.$(".dp-time-input").setValue("10:00");
    await browser.$(".dp-rep-unit").selectByAttribute("value", "w");
  }
  if (method === "enter") {
    await browser.$(".dp-time-input").click();
    await browser.keys(["Enter"]);
  } else if (method === "outside") {
    await browser.execute(() => document.querySelector(".dp-overlay").click());
  } else await browser.execute(() => [...document.querySelectorAll(".dp-btn")].find(el => el.textContent === "Done").click());
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".date-picker")), { timeout: 5000 });
  // Await layout frames, without clicking away or allowing blur to hide the harm.
  await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
  const observation = await browser.execute((scope) => {
    const editor = document.querySelector(`${scope} textarea.block-editor`);
    return editor ? { value: editor.value, scrollTop: editor.scrollTop, clientHeight: editor.clientHeight, scrollHeight: editor.scrollHeight, focused: document.activeElement === editor } : null;
  }, pane);
  proof.observations.push({ surface, command, method, ...observation });
  if (!observation?.value.startsWith("something") || !observation.value.includes(`\n${command.toUpperCase()}:`)) throw new Error(`${surface}: planning insertion lost the original text`);
  if (options && !observation.value.includes("10:00 +1w")) throw new Error(`${surface}: time/repeat draft was lost`);
  if (observation.scrollTop !== 0 || observation.clientHeight < observation.scrollHeight) {
    proof.failed = true;
    console.error(`FAIL ${surface}: original line is clipped: ${JSON.stringify(observation)}`);
  }
  await browser.$(".main-content h1.page-title").click();
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } } });
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Planning");
  await journey("routed named page");
  await openJournals(browser);
  await journey("journal feed");
  if (!proof.failed) {
    await openPageByName(browser, "Planning");
    await browser.execute(() => document.querySelector(".main-content h1.page-title").dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true })));
    await browser.$(".right-sidebar .block-content-wrapper").waitForExist({ timeout: 10000 });
    await journey("right sidebar", ".right-sidebar", "scheduled", "enter", true);
    await journey("routed page outside commit", ".main-content", "scheduled", "outside");
  }
  if (proof.failed) throw new Error("GH #668: open editor height does not cover its inserted content");
  console.log(`PASS ${JSON.stringify(proof)}`);
} catch (error) {
  proof.error = String(error);
  proof.classification = proof.failed ? "product" : "ambiguous; inspect DOM and driver log";
  if (browser) fs.writeFileSync(path.join(TMP, "failure-dom.html"), await browser.execute(() => document.body.innerHTML).catch(() => ""));
  throw error;
} finally {
  fs.writeFileSync(path.join(TMP, "proof.json"), `${JSON.stringify(proof, null, 2)}\n`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
