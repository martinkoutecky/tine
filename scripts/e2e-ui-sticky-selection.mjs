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
const env = { ...process.env, TINE_GRAPH: graph, HOME: path.join(out, "home"),
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const app = process.env.TINE_APP || path.resolve("target/debug/tine");
const port = Number(process.env.E2E_DRIVER_PORT || 4894);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", webdriverServerArgs(port, port + 1, "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
const failures = [];
const proof = { commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(), app, observations: [] };
const shot = name => execFileSync("import", ["-window", "root", path.join(out, `${name}.png`)], { env });
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
    return (await browser.$("[data-seen-header]")).isExisting();
  }, { timeout: 30000, interval: 400 });
  await browser.execute(() => { document.querySelector(".main-content").scrollTop = 400; });
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".main-content").scrollTop >= 390));
  const seen = await browser.execute(() => {
    const header = document.querySelector("[data-seen-header]").getBoundingClientRect();
    const pane = document.querySelector(".main-content").getBoundingClientRect();
    return { top: header.top, bottom: header.bottom, paneTop: pane.top };
  });
  proof.observations.push({ seen }); shot("seen-scrolled");
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
  await browser.$(".main-content textarea.block-editor").waitForExist();
  await browser.execute(() => {
    const ta = document.querySelector(".main-content textarea.block-editor");
    ta.focus({ preventScroll: true });
    const at = ta.value.indexOf("Selection target");
    if (at < 0) throw Error("fixture lost the target line");
    ta.setSelectionRange(at, at + "Selection target".length);
    const lh = parseFloat(getComputedStyle(ta).lineHeight);
    const y = ta.getBoundingClientRect().top + ta.value.slice(0, at).split("\n").length * lh;
    document.querySelector(".main-content").scrollTop += y - 450;
    ta.dispatchEvent(new Event("select", { bubbles: true }));
  });
  await browser.$(".main-content .sel-toolbar").waitForExist();
  const measure = () => browser.execute(() => {
    const ta = document.querySelector(".main-content textarea.block-editor");
    const toolbar = document.querySelector(".main-content .sel-toolbar").getBoundingClientRect();
    const pane = document.querySelector(".main-content").getBoundingClientRect();
    const cs = getComputedStyle(ta), lh = parseFloat(cs.lineHeight), box = ta.getBoundingClientRect();
    const top = box.top + parseFloat(cs.paddingTop) + ta.value.slice(0, ta.selectionStart).split("\n").length * lh - lh - ta.scrollTop;
    return { toolbar: { top: toolbar.top, bottom: toolbar.bottom, left: toolbar.left, right: toolbar.right, height: toolbar.height },
      selection: { top, bottom: top + lh }, blockTop: box.top, pane: { left: pane.left, right: pane.right } };
  });
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
  shot("selection-bottom");
  // Moving the range while the toolbar is already open must re-anchor it;
  // putting that first line against the pane top must flip it below the range.
  await browser.execute(() => {
    const ta = document.querySelector(".main-content textarea.block-editor");
    const at = ta.value.indexOf("Short line 30");
    ta.setSelectionRange(at, at + 13);
    const cs = getComputedStyle(ta), lh = parseFloat(cs.lineHeight);
    const y = ta.getBoundingClientRect().top + parseFloat(cs.paddingTop) + ta.value.slice(0, at).split("\n").length * lh - lh;
    const pane = document.querySelector(".main-content"), notice = document.querySelector("[data-seen-header]");
    pane.scrollTop += y - notice.getBoundingClientRect().bottom - 12;
    ta.dispatchEvent(new Event("select", { bubbles: true }));
  });
  await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
  const flipped = await measure(); proof.observations.push({ flipped });
  if (flipped.toolbar.top < flipped.selection.bottom || flipped.toolbar.top - flipped.selection.bottom > flipped.toolbar.height)
    failures.push(`SEL-TOOLBAR: did not follow changed selection and flip below: ${JSON.stringify(flipped)}`);
  shot("selection-flipped");
  if (failures.length) throw Error(failures.join("\n"));
  proof.outcome = "PASS";
  console.log(`PASS UI-STICKY-SELTOOLBAR: ${out}`);
} catch (error) {
  try { shot("failure"); } catch {}
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
