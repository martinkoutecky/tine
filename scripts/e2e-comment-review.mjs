// MC9: page review commands keep the passage visible in margin and popup display.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import assert from "node:assert/strict";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/mc9");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
const ids = [1, 2, 3].map(i => `09000000-0000-4000-8000-${String(i).padStart(12, "0")}`);
// A shared passage near the start of a block taller than the viewport: centering
// the whole block when reviewing its second thread would hide the passage.
const body = `- Passage 1 worth reviewing\n${Array.from({ length: 90 }, (_, n) => `  Context paragraph ${n} within the same tall block.\n`).join("")}` +
  ids.slice(0, 2).map((id, i) => `  - Thread ${i + 1}\n    id:: ${id}\n    tine.quote:: Passage 1\n`).join("") +
  `- Passage 3 worth reviewing\n  - Thread 3\n    id:: ${ids[2]}\n    tine.quote:: Passage 3\n`;
const file = path.join(graph, "pages/Review.md"); fs.writeFileSync(file, body);
const messageFile = path.join(graph, "pages/Messages.md");
const messageBody = "- A passage\n  - First message\n    tine.quote:: passage\n    tine.date:: 2026-11-07T13:37Z\n    - Evidence\n      - Nested evidence\n    - Second message\n      tine.date:: 2026-11-07T13:38Z\n      - Reply evidence\n  - Another thread\n    tine.quote:: A\n    tine.date:: 2026-11-07T13:39Z\n";
fs.writeFileSync(messageFile, messageBody);
fs.writeFileSync(path.join(graph, "pages/Other page.md"), "- Another page\n");
const fixture = accountHomeFixture(out, out, { ...process.env, TINE_GRAPH: graph, HOME: path.join(out, "home"),
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" });
async function freePort() {
  const server = net.createServer(); await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port; await new Promise(resolve => server.close(resolve)); return port;
}
const port = await freePort(), nativePort = await freePort();
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(nativePort), "--native-driver", "/usr/bin/WebKitWebDriver"],
  { env: fixture.env, stdio: ["ignore", log, log], detached: true });
const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(), candidate: "working tree", outcomes: [] };
let browser, stage = "startup";
async function keys(...value) { await browser.keys(value); await browser.releaseActions(); }
async function command(label) {
  await keys("Control", "Shift", "p");
  const search = await browser.$(".switcher-input"); await search.waitForExist(); await search.setValue(label);
  await browser.waitUntil(() => browser.execute(label => [...document.querySelectorAll(".switcher-row")].some(el => el.textContent.includes(label)), label),
    { timeout: 5000, timeoutMsg: `Palette must offer ${label}` });
  await browser.execute(() => {
    window.__mc9StepMs = null;
    const measure = event => {
      if (event.key !== "Enter") return;
      document.removeEventListener("keydown", measure, true);
      const start = performance.now();
      requestAnimationFrame(() => { window.__mc9StepMs = performance.now() - start; });
    };
    document.addEventListener("keydown", measure, true);
  });
  await keys("Enter"); await browser.$(".switcher").waitForExist({ reverse: true });
  await browser.waitUntil(() => browser.execute(() => window.__mc9StepMs !== null));
  const ms = await browser.execute(() => window.__mc9StepMs);
  assert.ok(ms < 500, `Review step must not block the UI for 500 ms: ${ms}`);
  proof.outcomes.push({ command: label, ms });
}
async function assertThread(id, display) {
  await browser.waitUntil(() => browser.execute((id, display) => {
    const active = document.activeElement;
    const card = active?.closest(".comment-card");
    if (card?.dataset.blockRef !== id || !active.closest(display === "margin" ? ".margin-column" : ".comment-thread-popup")) return false;
    const parent = [...document.querySelectorAll(".page-blocks .comment-quote-anchor")].find(el => el.textContent === (id.endsWith("3") ? "Passage 3" : "Passage 1"));
    if (!parent) return false;
    const r = parent.getBoundingClientRect(), pane = parent.closest(".main-content").getBoundingClientRect();
    return r.top >= pane.top && r.bottom <= Math.min(pane.bottom, innerHeight);
  }, id, display), { timeout: 5000, timeoutMsg: `Focus ${id} and keep its passage in view (${display})` });
  proof.outcomes.push({ display, focused: id, anchorInView: true });
}
try {
  const deadline = Date.now() + 10000;
  for (;;) {
    try { if ((await fetch(`http://127.0.0.1:${port}/status`)).ok) break; } catch {}
    if (Date.now() > deadline) throw new Error("Driver did not become ready");
    await delay(50);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: process.env.TINE_APP || path.join(root, "target/release/tine") } } });
  await ensureMainWindow(browser); await browser.$(".app-container").waitForExist({ timeout: 30000 }); await assertAppLease(fixture);
  await browser.setWindowSize(1800, 1000);
  for (const display of ["margin", "popup"]) {
    stage = `${display}: routed page`;
    await openPageByName(browser, "Review");
    assert.ok(await browser.execute(() => {
      const row = document.querySelector(".page-blocks .block-main");
      return row && row.getBoundingClientRect().height > row.closest(".main-content").getBoundingClientRect().height;
    }), "The shared passage fixture must be taller than its viewport");
    if (display === "popup") {
      await browser.$('button[title^="Settings"]').click();
      await browser.$(".settings-search-input").waitForExist();
      await browser.$(".settings-search-input").setValue("Comments");
      await browser.$('[data-setting-target="settings-comments-display"]').click();
      await browser.$('[data-setting-id="settings-comments-display"] select').selectByAttribute("value", "popup");
      await keys("Escape");
    }
    stage = `${display}: next next previous`;
    // The first Next establishes the first thread; the review then steps Next/Next/Previous.
    await command("Next comment"); await assertThread(ids[0], display);
    await command("Next comment"); await assertThread(ids[1], display);
    await command("Next comment"); await assertThread(ids[2], display);
    await command("Previous comment"); await assertThread(ids[1], display);
    if (display === "popup") {
      await browser.$(".comment-next-thread").click(); await assertThread(ids[2], display);
      await browser.$(".comment-prev-thread").click(); await assertThread(ids[1], display);
    }
    execFileSync("import", ["-window", "root", path.join(out, `${display}.png`)], { timeout: 10000 });
    assert.equal(fs.readFileSync(file, "utf8"), body, "Review leaves graph bytes unchanged");
    // A fresh scope resets the review cursor for the next display.
    await openPageByName(browser, "Other page");
  }
  stage = "message outlines and stored collapse";
  await openPageByName(browser, "Messages");
  await command("Next comment");
  await command("Next comment"); // the earlier single-word passage belongs to the other thread
  const first = await browser.$(".comment-card"); await first.waitForExist();
  assert.equal((await first.$$(".comment-header")).length, 2, "Body bullets are not separate messages");
  assert.equal((await first.$$(".bullet-container")).length, 3, "Message bodies keep ordinary bullets");
  assert.equal(fs.readFileSync(messageFile, "utf8"), messageBody, "Opening dated threads preserves bytes");
  const folds = await first.$$(".comment-collapse"); await folds[1].click();
  const bodyVisible = text => browser.execute(text => [...document.querySelectorAll(".block-content")].some(el => el.textContent.trim() === text), text);
  await browser.waitUntil(async () => !await bodyVisible("Reply evidence"), { timeout: 5000 });
  await browser.waitUntil(() => fs.readFileSync(messageFile, "utf8").includes("collapsed:: true"), { timeout: 5000 });
  await (await first.$$(".comment-collapse"))[1].click();
  await browser.waitUntil(() => bodyVisible("Reply evidence"), { timeout: 5000 });
  await first.$(":scope > .comment-header .comment-collapse").click();
  await first.$("button=1 reply").waitForExist();
  await first.$("button=1 reply").click();
  await browser.waitUntil(() => bodyVisible("Nested evidence"), { timeout: 5000 });
  await browser.execute(() => {
    const handle = document.querySelector(".comment-card > .comment-header .comment-handle");
    const rect = handle.getBoundingClientRect();
    handle.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: rect.left, clientY: rect.top }));
  });
  await browser.$("div=Collapse all comments on this block").waitForExist();
  await browser.$("div=Collapse all comments on this block").click();
  await browser.waitUntil(() => (fs.readFileSync(messageFile, "utf8").match(/collapsed:: true/g) ?? []).length === 2, { timeout: 5000 });
  await keys("Control", "z");
  await browser.waitUntil(() => fs.readFileSync(messageFile, "utf8") === messageBody, { timeout: 5000, timeoutMsg: "One undo restores both thread roots and their exact bytes" });
  proof.outcomes.push({ messageOutlines: true, storedMessageAndThreadFolds: true, collapseAllOneUndo: true, openingWritesNothing: true });
  execFileSync("import", ["-window", "root", path.join(out, "messages.png")], { timeout: 10000 });
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS: routed page review in margin and popup display. Artifacts: ${out}`);
} catch (error) {
  try { execFileSync("import", ["-window", "root", path.join(out, "failure.png")], { timeout: 10000 }); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage, expected: "Focused live thread and visible passage after each page review step", observed: String(error), classification: "ambiguous" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log); fixture.close?.(); fixture.verify();
}
