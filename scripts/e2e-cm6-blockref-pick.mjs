import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import net from "node:net";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { chooseEditor } from "./lib/e2e-editor-choice.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/cm6-blockref-pick");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");

fs.writeFileSync(path.join(graph, "pages/Target.md"), "- pick me zebracorn\n");
fs.writeFileSync(path.join(graph, "pages/Source.md"), "- seed\n");
const app = process.env.TINE_APP || path.join(root, "target/release/tine");
const fixture = accountHomeFixture(out, out, { ...process.env, HOME: path.join(out, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" });
async function freePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
const port = await freePort(), nativePort = await freePort();
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn("tauri-driver", webdriverServerArgs(port, nativePort, "/usr/bin/WebKitWebDriver"),
  { env: fixture.env, stdio: ["ignore", log, log], detached: true });
const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"), outcomes: [] };
let browser, stage = "startup";
const shot = name => execFileSync("import", ["-window", "root", path.join(out, name)], { env: fixture.env });
const UUID = "[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}";
try {
  await sleep(300);
  assert.equal(driver.exitCode, null);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(app) });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  stage = "Enable CodeMirror in Settings";
  await chooseEditor(browser, "cm6");
  stage = "Pick an id-less block in the CM6 (( picker";
  await openPageByName(browser, "Source");
  await browser.$(".main-content .block-content").click();
  await browser.$('.main-content [data-block-editor="cm6"]').waitForDisplayed();
  await browser.keys(["Control", "End"]);
  await browser.releaseActions();
  // Human pace: WebDriver's burst delivers both "(" in one frame, and CM6's autopair sees only one (not seen at 60 ms per key; card follows).
  for (const ch of " ((zebracorn") { await browser.keys(ch); await sleep(60); }
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".autocomplete .ac-label")]
    .some(label => label.textContent.includes("pick me zebracorn"))), { timeout: 10000, timeoutMsg: "the picker never offered the target" });
  shot("picker.png");
  await browser.keys("Enter");
  stage = "The reference names the id stamped on the target";
  const targetFile = path.join(graph, "pages/Target.md"), sourceFile = path.join(graph, "pages/Source.md");
  const targetText = await waitForFileText(targetFile, text => new RegExp(`id:: ${UUID}`).test(text));
  const stamped = new RegExp(`id:: (${UUID})`).exec(targetText)[1];
  await browser.keys("Escape");
  // Autosave can land mid-typing ("((zebra))"); wait for the inserted id reference, then compare it with the stamp.
  const sourceText = await waitForFileText(sourceFile, text => new RegExp(`\\(\\(${UUID}\\)\\)`).test(text));
  const inserted = /\(\(([^)]+)\)\)/.exec(sourceText)?.[1];
  proof.stamped = stamped; proof.inserted = inserted;
  assert.equal(inserted, stamped, `Source references ((${inserted})) but Target was stamped id:: ${stamped}`);
  stage = "The reference renders the target";
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".main-content .block-ref")]
    .some(ref => ref.textContent.includes("pick me zebracorn"))), { timeout: 10000, timeoutMsg: "the reference does not render its target" });
  shot("rendered.png");
  fixture.verify();
  proof.outcome = "PASS";
} catch (error) {
  proof.outcome = "FAIL"; proof.stage = stage; proof.observed = String(error.stack);
  proof.expected = "A CM6 (( pick of an id-less block references exactly the id:: stamped on the target, and the reference renders it";
  proof.classification = error.code === "ERR_ASSERTION" ? "product" : "ambiguous; inspect screenshot and driver log";
  try { shot("failure.png"); } catch {}
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`${proof.outcome}: ${out}`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fixture.close();
  fs.closeSync(log);
}
