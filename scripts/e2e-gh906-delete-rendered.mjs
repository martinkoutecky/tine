// GH #906: real textarea/selection/context-menu deletes and merges of a rule.
// Fixtures, account HOME and XDG directories are disposable; writes under test
// come only from normal editor actions and the application's guarded save path.
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
import { openPageByName, openJournals } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/fix-beta-hr-delete-906"));
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const actions = ["selected-delete", "selected-backspace", "context-delete", "cut", "clear-backspace", "backspace-start", "delete-forward"];
const cases = [];
const expectedFiles = new Map();
const line = (format, depth, raw) => (format === "org" ? "*".repeat(depth + 1) : "  ".repeat(depth) + "-") + (raw ? " " + raw : "") + "\n";
function specimen(format, position, children, depth = 0) {
  const rows = [{ raw: "Before", children: [] }, { raw: "After", children: [] }];
  rows.splice(position, 0, { raw: "---", children: children ? ["Child"] : [] });
  return { rows, text: rows.map(row => line(format, depth, row.raw) + row.children.map(raw => line(format, depth + 1, raw)).join("")).join("") };
}
function resultText(test) {
  const { format, position, children, action, depth = 0 } = test;
  const { rows } = specimen(format, position, children, depth);
  if (action === "clear-backspace" && position === 0 && children) {
    rows[0].raw = "";
    return rows.map(row => line(format, depth, row.raw) + row.children.map(raw => line(format, depth + 1, raw)).join("")).join("");
  }
  if (action === "backspace-start" && position === 0) return specimen(format, position, children, depth).text;
  if (action === "delete-forward" && position === 0) return specimen(format, position, children, depth).text;
  const merge = action === "backspace-start" || action === "delete-forward" || action === "clear-backspace";
  if (merge && position > 0) {
    rows[position - 1].raw += action === "clear-backspace" ? "" : "---";
    rows[position - 1].children.push(...rows[position].children);
  }
  rows.splice(position, 1);
  return rows.map(row => line(format, depth, row.raw) + row.children.map(raw => line(format, depth + 1, raw)).join("")).join("");
}
for (const format of ["md", "org"]) {
  for (const action of actions) for (const position of [1, 0, 2]) for (const children of [false, true]) {
    if (action === "delete-forward" && position === 0) continue;
    const name = `Rule ${format} ${action} ${position} ${children ? "children" : "leaf"}`;
    const file = path.join(graph, "pages", `${name}.${format}`);
    const { text } = specimen(format, position, children);
    fs.writeFileSync(file, text);
    expectedFiles.set(file, text);
    cases.push({ name, file, format, action, position, children, surface: "page" });
  }
  const date = new Date();
  if (format === "org") date.setDate(date.getDate() - 1);
  const stem = `${date.getFullYear()}_${String(date.getMonth() + 1).padStart(2, "0")}_${String(date.getDate()).padStart(2, "0")}`;
  const file = path.join(graph, "journals", `${stem}.${format}`);
  let text = "";
  for (const action of actions) {
    const name = `Journal ${format} ${action}`;
    text += line(format, 0, name) + specimen(format, 1, false, 1).text;
    cases.push({ name, file, format, action, position: 1, children: false, depth: 1, surface: "journal" });
  }
  fs.writeFileSync(file, text);
  expectedFiles.set(file, text);
}
const app = path.resolve(process.env.TINE_APP || path.join(root, "target/release/tine"));
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
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"), passed: [], outcome: "pending" };
let browser;
let stage = "startup";
let lastSurface;
try {
  await sleep(200);
  assert.equal(driver.exitCode, null, "this journey's WebDriver exited before session creation");
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2,
    connectionRetryTimeout: 30_000, capabilities: tauriCapabilities(app) });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30_000 });
  await assertAppLease(fixture);
  for (const test of cases.filter(test => !process.env.GH906_CASE || test.name.includes(process.env.GH906_CASE))) {
    stage = test.name;
    console.log(`RUN ${stage}`);
    if (test.surface === "page") await openPageByName(browser, test.name);
    else if (lastSurface !== "journal") await openJournals(browser);
    lastSurface = test.surface;
    const ids = await browser.execute((surface, name, position) => {
      let rows = [...document.querySelectorAll(".main-content .page-section .ls-block")];
      if (surface === "journal") {
        const group = rows.find(row => row.querySelector(":scope > .block-main .block-content")?.textContent?.trim() === name);
        if (!group) throw new Error(`Missing journal specimen ${name}`);
        rows = [...group.querySelectorAll(".ls-block")].filter(row => row.parentElement.closest(".ls-block") === group);
      } else rows = rows.filter(row => !row.parentElement.closest(".ls-block"));
      const target = rows[position];
      target.scrollIntoView({ block: "center" });
      return { target: target.dataset.blockId, before: rows[position - 1]?.dataset.blockId,
        siblings: rows.filter(row => row !== target).map(row => row.dataset.blockId) };
    }, test.surface, test.name, test.position);
    assert(ids.target, `target block absent: ${stage}`);
    if (test.action === "context-delete") {
      await browser.execute(id => {
        document.querySelector(`[data-block-id="${id}"] > .block-main .block-content-wrapper`)
          .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
        document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      }, ids.target);
      await browser.$(".main-content textarea.block-editor").waitForExist({ timeout: 5000 });
      await browser.execute(id => document.querySelector(`[data-block-id="${id}"] > .block-main`)
        .dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 })), ids.target);
      await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".ctx-item")].some(el => el.textContent.trim() === "Delete block")), { timeout: 5000 });
      await browser.execute(() => [...document.querySelectorAll(".ctx-item")].find(el => el.textContent.trim() === "Delete block").click());
    } else {
      const forward = test.action === "delete-forward";
      await browser.execute(id => {
        const target = document.querySelector(`[data-block-id="${id}"] > .block-main .block-content-wrapper`);
        target.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
        document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
      }, forward ? ids.before : ids.target);
      const editor = await browser.$(".main-content textarea.block-editor");
      await editor.waitForExist({ timeout: 5000 });
      await browser.waitUntil(() => browser.execute(id => document.activeElement?.closest("[data-block-id]")?.dataset.blockId === id,
        forward ? ids.before : ids.target), { timeout: 5000, timeoutMsg: `${stage}: the target editor did not acquire focus` });
      if (process.env.GH906_RETYPE_RULE && test.action === "backspace-start") {
        for (const raw of ["----", "---"]) {
          await browser.keys(["Control", "a"]); await browser.keys(["Control"]);
          for (const ch of raw) await browser.keys([ch]);
          await waitForFileText(test.file, text => raw === "----" ? text.includes("----") : text === expectedFiles.get(test.file));
        }
      }
      if (test.action.startsWith("selected") || test.action === "cut") {
        await browser.keys(["Escape"]);
        await browser.waitUntil(() => browser.execute(id => !document.querySelector(".main-content textarea.block-editor")
          && document.querySelector(`[data-block-id="${id}"] > .block-main`)?.classList.contains("selected"), ids.target),
        { timeout: 5000, timeoutMsg: `${stage}: Escape did not select the target block` });
        if (test.action === "cut") { await browser.keys(["Control", "x"]); await browser.keys(["Control"]); }
        else await browser.keys([test.action === "selected-delete" ? "Delete" : "Backspace"]);
      } else {
        if (test.action === "clear-backspace") {
          await browser.keys(["Control", "a"]); await browser.keys(["Control"]); await browser.keys(["Backspace"]);
        }
        await browser.execute(forward => {
          const editor = document.querySelector(".main-content textarea.block-editor");
          editor.focus(); editor.setSelectionRange(forward ? editor.value.length : 0, forward ? editor.value.length : 0);
        }, forward);
        await browser.keys([forward ? "Delete" : "Backspace"]);
      }
    }
    const visible = await browser.execute(ids => ({
      failures: [...document.querySelectorAll(".main-content .region-failure")].map(el => el.textContent),
      siblings: ids.siblings.every(id => document.querySelector(`[data-block-id="${id}"]`)),
    }), ids);
    assert.deepEqual(visible.failures, [], `${stage}: a block deletion took down a rendered region`);
    assert(visible.siblings, `${stage}: a surviving sibling disappeared`);
    const replacement = resultText(test);
    if (replacement !== null) {
      const before = expectedFiles.get(test.file);
      const expected = test.surface === "page" ? replacement : before.replace(
        line(test.format, 0, test.name) + specimen(test.format, 1, false, 1).text,
        line(test.format, 0, test.name) + replacement);
      await waitForFileText(test.file, text => text === expected);
      expectedFiles.set(test.file, expected);
    }
    // Capture representative routes and both journal formats for visual review.
    if (test.position === 1 && !test.children && (test.action === "backspace-start" || test.surface === "journal" && test.action === "selected-delete")) {
      await browser.saveScreenshot(path.join(out, `${test.name}.png`));
    }
    if (await browser.$(".main-content textarea.block-editor").isExisting()) await browser.keys(["Escape"]);
    proof.passed.push(stage);
    fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  }
  for (const [file, expected] of expectedFiles) assert.equal(fs.readFileSync(file, "utf8"), expected, `Unexpected changes in ${file}`);
  proof.outcome = "PASS";
  fixture.verify();
  console.log(`PASS GH906 rendered-block deletion (${proof.passed.length} cases): ${out}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(out, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  proof.failureCapsule = { testedCommit: proof.base, journey: stage,
    expected: "Deletion/merge preserves the page and remaining blocks; only the intended outline change reaches disk",
    observed: String(error.stack || error), evidence: ["driver.log", "failure-dom.html", "failure.png"].filter(file => fs.existsSync(path.join(out, file))),
    classification: stage === "startup" ? "infrastructure" : String(error).includes("took down a rendered region") ? "product"
      : String(error).includes("screenshot") ? "harness" : "ambiguous (inspect DOM, screenshot and driver log)" };
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  fixture.close();
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
