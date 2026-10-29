// Martin 2026-10-28: actual routed git page, long-block selection and seen notice.
// Run under xvfb-run -s "-screen 0 1920x1080x24"; uses only a disposable profile.
import fs from "node:fs";
import path from "node:path";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";
import { editorKind, editorSelector, scratchProfile } from "./lib/native-editor-bench.mjs";

fs.mkdirSync(path.resolve("artifacts/ui-sticky"), { recursive: true });
const out = fs.mkdtempSync(path.resolve("artifacts/ui-sticky/native-"));
const graph = path.join(out, "graph");
for (const dir of ["graph/pages", "graph/journals", "graph/logseq", "home", "data", "config", "cache", "state"])
  fs.mkdirSync(path.join(out, dir), { recursive: true });
const file = path.join(graph, "pages/Sticky Demo.md");
const long = ["Long block", ...Array.from({ length: 36 }, (_, i) => `Short line ${i}`), "Selection target near bottom", "Last line"];
const content = `- ${long.join("\n  ")}\n${Array.from({ length: 80 }, (_, i) => `- Filler ${i}\n`).join("")}`;
fs.writeFileSync(file, content);
const git = (...args) => execFileSync("git", ["-C", graph, ...args], { encoding: "utf8" }).trim();
git("init", "-q", "-b", "main"); git("config", "user.name", "Me"); git("config", "user.email", "me@example.invalid");
git("add", "."); git("commit", "-qm", "baseline");
const env = { ...process.env, ...scratchProfile(out), TINE_GRAPH: graph,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const app = process.env.TINE_APP || path.resolve("target/debug/tine");
const port = Number(process.env.E2E_DRIVER_PORT || 4894);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
const failures = [];
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), editor: editorKind, app, observations: [] };
const shot = async name => browser.saveScreenshot(path.join(out, `${name}.png`));
// Use the browser's public DOM selection for either engine. Fixture lines are
// plain text, so no editor-private API or geometry approximation is needed for CM.
async function selectText(label, length, targetY) {
  await browser.execute((selector, label, length, targetY) => {
    const editor = document.querySelector(`.main-content ${selector}`);
    editor.focus({ preventScroll: true });
    let y;
    if (editor instanceof HTMLTextAreaElement) {
      const at = editor.value.indexOf(label);
      if (at < 0) throw Error("fixture lost selection target");
      editor.setSelectionRange(at, at + length);
      const cs = getComputedStyle(editor), lh = parseFloat(cs.lineHeight);
      y = editor.getBoundingClientRect().top + parseFloat(cs.paddingTop) + editor.value.slice(0, at).split("\n").length * lh - lh;
      editor.dispatchEvent(new Event("select", { bubbles: true }));
    } else {
      const line = [...editor.querySelectorAll(".cm-line")].find(el => el.textContent.includes(label));
      if (!line) throw Error("fixture lost CM selection target");
      const walker = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
      let text, offset = line.textContent.indexOf(label);
      while ((text = walker.nextNode()) && offset > text.length) offset -= text.length;
      if (!text || offset + length > text.length) throw Error("unexpected fixture text split");
      const range = document.createRange(); range.setStart(text, offset); range.setEnd(text, offset + length);
      const selection = window.getSelection(); selection.removeAllRanges(); selection.addRange(range);
      document.dispatchEvent(new Event("selectionchange"));
      y = range.getBoundingClientRect().top;
    }
    const pane = document.querySelector(".main-content");
    const notice = document.querySelector("[data-seen-header]");
    pane.scrollTop += y - (targetY ?? notice.getBoundingClientRect().bottom + 12);
  }, editorSelector, label, length, targetY);
  await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
}
try {
  await sleep(1500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0,
    capabilities: tauriCapabilities(app, "ui-sticky") });
  await ensureMainWindow(browser);
  await browser.setWindowSize(1280, 900);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Sticky Demo");
  await browser.$(".main-content .block-content").waitForExist();
  // Wait for the initial seen request to finish before another author commits.
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".page-loading")), { timeout: 10000 });
  fs.writeFileSync(file, content.replace("Filler 0\n", "Filler 0 changed by Jo\n"));
  git("add", "."); git("-c", "user.name=Jo", "-c", "user.email=jo@example.invalid", "commit", "-qm", "Jo changes filler");
  await browser.waitUntil(async () => {
    await browser.execute(() => window.dispatchEvent(new Event("focus")));
    // The external-file watcher can still be replacing the old page when its
    // first seen response arrives. Wait for the new content and its notice in
    // one snapshot, rather than measuring a transient header from the old page.
    return browser.execute(() => !!document.querySelector("[data-seen-header]")
      && document.querySelector(".main-content")?.textContent.includes("Filler 0 changed by Jo")
      && !document.querySelector(".page-loading"));
  }, { timeout: 30000, interval: 400 });
  await browser.execute(() => { document.querySelector(".main-content").scrollTop = 400; });
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".main-content").scrollTop >= 390));
  let seen;
  await browser.waitUntil(async () => {
    seen = await browser.execute(() => {
      const notice = document.querySelector("[data-seen-header]");
      if (!notice) return null;
      const header = notice.getBoundingClientRect();
      const pane = document.querySelector(".main-content").getBoundingClientRect();
      return { top: header.top, bottom: header.bottom, paneTop: pane.top };
    });
    return !!seen;
  }, { timeout: 10000 });
  proof.observations.push({ seen }); await shot("seen-scrolled");
  if (Math.abs(seen.top - seen.paneTop) > 3) failures.push(`SEEN-STICKY: notice left pane top: ${JSON.stringify(seen)}`);
  const revealed = await browser.execute(() => {
    const target = [...document.querySelectorAll(".main-content .block-content")].find(el => el.textContent.trim() === "Filler 5");
    if (!target) throw Error("missing reveal target");
    target.scrollIntoView({ block: "start" });
    return { targetTop: target.getBoundingClientRect().top, noticeBottom: document.querySelector("[data-seen-header]").getBoundingClientRect().bottom };
  });
  proof.observations.push({ revealed });
  if (revealed.targetTop < revealed.noticeBottom) failures.push(`SEEN-STICKY: revealed block is covered: ${JSON.stringify(revealed)}`);
  await browser.execute(() => { document.querySelector(".main-content").scrollTop = 0; });
  await browser.execute(() => {
    const block = [...document.querySelectorAll(".main-content .block-content")].find(el => el.textContent.includes("Long block"));
    block.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  });
  await browser.$(`.main-content ${editorSelector}`).waitForExist();
  await selectText("Selection target", "Selection target".length, 450);
  await browser.$(".main-content .sel-toolbar").waitForExist();
  const measure = () => browser.execute(selector => {
    const ta = document.querySelector(`.main-content ${selector}`);
    const toolbar = document.querySelector(".main-content .sel-toolbar").getBoundingClientRect();
    const pane = document.querySelector(".main-content").getBoundingClientRect();
    const cs = getComputedStyle(ta), lh = parseFloat(cs.lineHeight), box = ta.getBoundingClientRect();
    const range = ta instanceof HTMLTextAreaElement ? null : window.getSelection().getRangeAt(0).getBoundingClientRect();
    const top = range?.top ?? box.top + parseFloat(cs.paddingTop) + ta.value.slice(0, ta.selectionStart).split("\n").length * lh - lh - ta.scrollTop;
    return { toolbar: { top: toolbar.top, bottom: toolbar.bottom, left: toolbar.left, right: toolbar.right, height: toolbar.height },
      selection: { top, bottom: range?.bottom ?? top + lh }, blockTop: box.top, pane: { left: pane.left, right: pane.right } };
  }, editorSelector);
  // Wait a frame for coalesced positioning, without making a failing oracle a timeout.
  await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
  for (const delta of [0, 65]) {
    if (delta) {
      await browser.execute(d => { document.querySelector(".main-content").scrollTop += d; }, delta);
      await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
    }
    const m = await measure(); proof.observations.push(m);
    const gap = Math.min(Math.abs(m.toolbar.bottom - m.selection.top), Math.abs(m.toolbar.top - m.selection.bottom));
    if (gap > m.toolbar.height || m.toolbar.left < m.pane.left || m.toolbar.right > m.pane.right)
      failures.push(`SEL-TOOLBAR: toolbar is far from selected line or outside pane: ${JSON.stringify(m)}`);
  }
  await shot("selection-bottom");
  // Moving the range while the toolbar is already open must re-anchor it;
  // putting that first line against the pane top must flip it below the range.
  await selectText("Short line 30", 13, null);
  await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
  const flipped = await measure(); proof.observations.push({ flipped });
  if (flipped.toolbar.top < flipped.selection.bottom || flipped.toolbar.top - flipped.selection.bottom > flipped.toolbar.height)
    failures.push(`SEL-TOOLBAR: did not follow changed selection and flip below: ${JSON.stringify(flipped)}`);
  await shot("selection-flipped");
  if (failures.length) throw Error(failures.join("\n"));
  proof.outcome = "PASS";
  console.log(`PASS UI-STICKY-SELTOOLBAR: ${out}`);
} catch (error) {
  try { await shot("failure"); } catch {}
  proof.outcome = "FAIL"; proof.expected = "Notice remains at pane top; toolbar follows selection near bottom of long block";
  proof.observed = String(error.stack); proof.classification = failures.length ? "product" : "ambiguous";
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fs.closeSync(log);
}
