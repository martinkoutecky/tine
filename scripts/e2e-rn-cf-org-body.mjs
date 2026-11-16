// RN-C / L3-04: mixed-format, unloaded Org references retain their body.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const ART = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/rn-cf/native"));
fs.mkdirSync(ART, { recursive: true });
const run = fs.mkdtempSync(path.join(ART, "run-"));
const graph = path.join(run, "graph");
for (const d of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, d), { recursive: true });
for (const d of ["data", "config", "cache"]) fs.mkdirSync(path.join(run, d));
const uuid = "6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e6f";
const source = "* Note:: see [[Foo]]\n:PROPERTIES:\n:id: " + uuid + "\n:END:\n** Child witness\n";
const org = path.join(graph, "pages/Org notes.ORG");
fs.writeFileSync(org, source);
fs.writeFileSync(path.join(graph, "pages/Host.md"), `- ((${uuid}))\n`);
fs.writeFileSync(path.join(graph, "pages/Foo.md"), "- Destination witness\n");
const running = path.join(graph, "pages/Running.md");
fs.writeFileSync(running, "- DOING\n  :LOGBOOK:\n  CLOCK: [2026-01-01 Thu 09:00:00]\n  :END:\n");
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:preferred-format :markdown}\n");
const port = Number(process.env.E2E_DRIVER_PORT || 4829);
const display = Number(process.env.E2E_X_DISPLAY || 1829);
const env = { ...process.env, TINE_GRAPH: graph, DISPLAY: `127.0.0.1:${display}`,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
// TCP-only and no lock file keep runtime artifacts inside the assigned worktree.
const xvfb = spawn("Xvfb", [`:${display}`, "-screen", "0", "1920x1080x24", "-nolock", "-nolisten", "unix", "-listen", "tcp", "-ac"], { env, stdio: ["ignore", log, log] });
const driver = spawn(process.env.TAURI_DRIVER || "/usr/local/bin/tauri-driver", ["--port", String(port), "--native-port", String(port + 1), "--native-driver", "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let stage = "startup";
const proof = { base: process.env.TINE_TESTED_BASE || "70fdc61d13c6ee8224f593b1c9016e272c84d4f8 + RN-C/F working delta", steps: [] };
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP || path.join(ROOT, "target/debug/tine") } } });
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  stage = "unloaded Org block reference on routed Host";
  await openPageByName(browser, "Host");
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".main-content .block-ref")?.textContent.includes("Note:: see")), { timeout: 20_000 });
  proof.steps.push(stage);
  stage = "Org linked reference on routed Foo";
  await openPageByName(browser, "Foo");
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".main-content .linked-references")?.textContent.includes("Note:: see")), { timeout: 20_000 });
  proof.steps.push(stage);
  stage = "source remains byte-identical";
  if (fs.readFileSync(org, "utf8") !== source) throw new Error("read surfaces changed the Org source");
  proof.steps.push(stage);
  stage = "editor extends a bare running task without clocking in twice";
  await openPageByName(browser, "Running");
  const task = await browser.$("//div[contains(@class, 'block-content') and normalize-space(.) = 'DOING']");
  await task.waitForDisplayed({ timeout: 10_000 });
  await task.click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 10_000 });
  await browser.keys(["Control", "Home"]); await browser.keys(["Control"]);
  await browser.keys(["End"]); await browser.keys([..." work"]);
  const edited = await browser.$("textarea.block-editor").getValue();
  if (!edited.startsWith("DOING work\n") || edited.split("CLOCK:").length !== 2) throw new Error(`wrong task buffer: ${edited}`);
  await browser.keys(["Escape"]);
  await waitForFileText(running, text => text.includes("DOING work"));
  proof.steps.push(stage);
  stage = "checkbox completes the task and closes the same clock";
  await browser.$(".main-content .block-task-checkbox").click();
  await waitForFileText(running, text => text.includes("]--["));
  if (fs.readFileSync(running, "utf8").split("CLOCK:").length !== 2) throw new Error("task has more than one clock");
  proof.steps.push(stage);
  fs.writeFileSync(path.join(run, "final-dom.html"), await browser.getPageSource());
  proof.outcome = "PASS";
  console.log(`PASS RN-C: ${run}`);
} catch (error) {
  Object.assign(proof, { outcome: "FAIL", stage, expected: "Org body stays visible and source bytes stay unchanged", observed: String(error), classification: "ambiguous; inspect driver log and DOM" });
  try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.getPageSource()); } catch {}
  throw error;
} finally {
  fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  xvfb.kill("SIGTERM");
  fs.closeSync(log);
}
