// Native journey: inject one DOM render throw, repair the block through the
// visible raw fallback, verify guarded persistence, then navigate between pages.
// No production fault hook; the one-shot DOM double is scoped to this WebView.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import net from "node:net";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/fix-741-f1f2");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const file = path.join(graph, "pages/Isolation.md");
const clocked = "- DOING\n  :LOGBOOK:\n  CLOCK: [2026-06-25 Thu 09:00:00]\n  :END:\n";
fs.writeFileSync(file, "- Native failing block\n- Healthy sibling\n  - Healthy child\n" + clocked);
fs.writeFileSync(path.join(graph, "pages/Healthy.md"), "- Navigation survived\n");
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const fixture = accountHomeFixture(out, out, { ...process.env, HOME: path.join(out, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" });
async function freePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
const port = Number(process.env.E2E_DRIVER_PORT || await freePort());
let nativePort = Number(process.env.E2E_NATIVE_PORT || await freePort());
while (nativePort === port) nativePort = await freePort();
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, nativePort, "/usr/bin/WebKitWebDriver"),
  { env: fixture.env, stdio: ["ignore", log, log], detached: true });
const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"), outcome: "pending" };
let browser;
let stage = "startup";
try {
  // A shared-host port collision must fail before contacting a foreign driver.
  await sleep(200);
  assert.equal(driver.exitCode, null, "this journey's WebDriver exited before session creation");
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(app) });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await assertAppLease(fixture);
  await openPageByName(browser, "Healthy");
  // Fail exactly the target block's first dynamic text insertion. Once consumed,
  // the fallback can show those same source bytes without a second artificial throw.
  await browser.execute(() => {
    const create = document.createTextNode.bind(document);
    window.__gh741Faults = [];
    window.__gh741Target = { text: "Native failing block", scope: "block" };
    const check = value => {
      const target = window.__gh741Target;
      if (target && value === target.text) {
        window.__gh741Faults.push(target.scope);
        window.__gh741Target = null;
        throw new Error(`GH741 injected native ${target.scope} render failure`);
      }
    };
    document.createTextNode = value => {
      check(value);
      return create(value);
    };
    for (const [prototype, property] of [[Node.prototype, "textContent"], [CharacterData.prototype, "data"]]) {
      const descriptor = Object.getOwnPropertyDescriptor(prototype, property);
      Object.defineProperty(prototype, property, { ...descriptor,
        set(value) { check(value); descriptor.set.call(this, value); } });
    }
  });
  stage = "block error leaves siblings and raw repair live";
  await openPageByName(browser, "Isolation");
  await browser.$(".main-content .block-render-raw").waitForDisplayed({ timeout: 15_000 });
  assert.equal(await browser.$('[data-failed-region="This page"]').isExisting(), false);
  const text = await browser.$(".main-content").getText();
  assert.ok(text.includes("Healthy sibling") && text.includes("Healthy child"), text);
  // Backend-seeded task facets must agree with the corrected frontend AST.
  assert.ok(text.includes("DOING"), "bare clocked task lost its marker on native load");
  assert.deepEqual(await browser.execute(() => window.__gh741Faults), ["block"]);
  await browser.saveScreenshot(path.join(out, "block-failure.png"));
  await browser.$(".main-content .block-render-raw").click();
  const editor = await browser.$(".main-content textarea.block-editor");
  await editor.waitForExist({ timeout: 5000 });
  assert.equal(await editor.getValue(), "Native failing block");
  await browser.keys(["Control", "a"]); await browser.keys(["Control"]);
  for (const ch of "Repaired block") await browser.keys([ch]);
  await browser.keys(["Escape"]);
  await waitForFileText(file, text => text === "- Repaired block\n- Healthy sibling\n  - Healthy child\n" + clocked);
  assert.equal(await browser.$(".main-content .block-render-raw").isExisting(), false);
  stage = "navigation after block failure";
  await openPageByName(browser, "Healthy");
  assert.ok((await browser.$(".main-content").getText()).includes("Navigation survived"));
  stage = "return to repaired page";
  await openPageByName(browser, "Isolation");
  assert.ok((await browser.$(".main-content").getText()).includes("Repaired block"));
  assert.equal(await browser.$(".main-content .region-failure").isExisting(), false);
  proof.outcome = "PASS";
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  fixture.verify();
  console.log(`PASS GH741 render isolation: ${out}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(out, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "One block fails inline; its raw editor repairs persisted bytes; siblings and navigation away and back survive",
    observed: String(error.stack || error), classification: stage === "startup" ? "infrastructure" : "ambiguous (inspect DOM, screenshot and driver log)" }, null, 2) + "\n");
  throw error;
} finally {
  fixture.close();
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
