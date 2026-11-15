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
const artifacts = process.env.E2E_ARTIFACT_DIR || path.join(root, "artifacts/cm6-esc-865");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const cases = [
  ["Math following", "md", "$$fdfdfdf$$\ndddddfafafafd"],
  ["Math final", "md", "intro\n$$fdfdfdf$$"],
  ["Markdown code", "md", "```js\nx\n```"],
  ["Org code", "org", "#+BEGIN_SRC js\nx\n#+END_SRC"],
];
const cm6Cases = [
  ["CM6 Markdown code", "md", "```js\nx\n```"],
  ["CM6 Org code", "org", "#+BEGIN_SRC js\nx\n#+END_SRC"],
];
for (const [name, format, raw] of [...cases, ...cm6Cases]) fs.writeFileSync(path.join(graph, `pages/${name}.${format}`),
  (format === "md" ? "- " + raw.replaceAll("\n", "\n  ") : "* " + raw) + "\n");
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
try {
  await sleep(300);
  assert.equal(driver.exitCode, null);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(app) });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  await chooseEditor(browser, "textarea");
  for (const [name, format, raw] of cases) {
    stage = name;
    console.log(`Case: ${name}`);
    await openPageByName(browser, name);
    await browser.$(".main-content .block-content").click();
    const editor = await browser.$(".main-content textarea.block-editor");
    await editor.waitForDisplayed();
    await browser.keys(["Control", "End"]);
    await browser.releaseActions();
    if (name.startsWith("Math")) {
      await browser.keys("Enter");
      const success = await browser.waitUntil(async () => (await browser.$(".main-content textarea.block-editor").getValue()) === "", { timeout: 3000 }).then(() => true, () => false);
      proof.outcomes.push({ name, success });
      if (success) {
        await browser.keys("continued below formula");
        await browser.keys("Escape");
        await waitForFileText(path.join(graph, `pages/${name}.${format}`), text => text.includes("continued below formula"));
      }
    } else {
      assert.equal(await editor.getValue(), "x");
      await browser.keys("Escape");
      const success = await browser.waitUntil(async () => {
        const el = await browser.$(".main-content textarea.block-editor");
        return await el.isExisting() && await el.getValue() === raw;
      }, { timeout: 3000 }).then(() => true, () => false);
      proof.outcomes.push({ name, success });
      if (success) {
        shot(`${name}-raw.png`);
        await browser.keys(["Control", "Home"]);
        await browser.releaseActions();
        await browser.keys("End");
        await browser.keys(" raw continuation");
        await browser.keys("Escape");
        await waitForFileText(path.join(graph, `pages/${name}.${format}`), text => text.includes("raw continuation"));
        await browser.$(".main-content .block-content").click();
        await browser.waitUntil(async () => (await browser.$(".main-content textarea.block-editor").getValue()) === "x");
        await browser.keys("Escape");
        await browser.keys("Escape");
      }
    }
    shot(`${name}.png`);
  }
  stage = "Enable CodeMirror in Settings";
  await chooseEditor(browser, "cm6");
  const cmText = () => browser.execute(() => [...document.querySelectorAll(".main-content .cm-content .cm-line")].map(line => line.textContent).join("\n"));
  for (const [name, format, raw] of cm6Cases) {
    stage = name;
    console.log(`Case: ${name}`);
    await openPageByName(browser, name);
    await browser.$(".main-content .block-content").click();
    await browser.$('.main-content [data-block-editor="cm6"]').waitForDisplayed();
    assert.equal(await cmText(), "x");
    await browser.keys(["Control", "End"]);
    await browser.releaseActions();
    await browser.keys("y");
    await browser.keys("Escape");
    const expected = raw.replace("\nx\n", "\nxy\n");
    const success = await browser.waitUntil(async () => await cmText() === expected, { timeout: 3000 }).then(() => true, () => false);
    proof.outcomes.push({ name, success });
    assert.ok(success, `${name}: Escape must reveal raw source`);
    shot(`${name}-raw.png`);
    // Edit the opener while preserving a complete code wrapper for reopening.
    await browser.keys(["Control", "Home"]);
    await browser.releaseActions();
    await browser.keys("End");
    await browser.keys(" raw continuation");
    assert.equal(await cmText(), expected.replace("js\n", "js raw continuation\n"));
    await browser.keys("Escape");
    await browser.waitUntil(async () => !await browser.$('.main-content [data-block-editor="cm6"]').isExisting());
    await waitForFileText(path.join(graph, `pages/${name}.${format}`), text => text.includes("xy") && text.includes("raw continuation"));
    await browser.$(".main-content .block-content").click();
    await browser.waitUntil(async () => await cmText() === "xy");
    await browser.keys("Escape");
    await browser.keys("Escape");
  }
  assert.ok(proof.outcomes.every(x => x.success), JSON.stringify(proof.outcomes));
  fixture.verify();
  proof.outcome = "PASS";
} catch (error) {
  proof.outcome = "FAIL"; proof.stage = stage; proof.observed = String(error.stack);
  proof.expected = "Enter creates a new block after closed math; Escape reveals editable source delimiters";
  proof.classification = error.code === "ERR_ASSERTION" && proof.outcomes.some(x => !x.success)
    ? "product" : "ambiguous; inspect screenshot and driver log";
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
