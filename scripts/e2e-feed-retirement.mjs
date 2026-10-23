// L10-03: real Journals scrolling, bounded model-backed Find, geometry and save.
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { openJournals, openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const base = execFileSync("rtk", ["git", "rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
const artifacts = path.join(ROOT, "artifacts/l10-03");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "tmp"]) fs.mkdirSync(path.join(run, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(graph, "pages/Other.md"), "- Other route\n");
const days = Array.from({ length: 60 }, (_, i) => {
  const date = new Date(); date.setDate(date.getDate() - i);
  const stem = `${date.getFullYear()}_${String(date.getMonth() + 1).padStart(2, "0")}_${String(date.getDate()).padStart(2, "0")}`;
  const file = path.join(graph, "journals", `${stem}.md`);
  fs.writeFileSync(file, Array.from({ length: 30 }, (_, b) => `- Feed marker day ${i} row ${b}`).join("\n") + "\n");
  return file;
});
const port = Number(process.env.E2E_DRIVER_PORT || 4791);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), TMPDIR: path.join(run, "tmp"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "/opt/cargo", "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
let journey = "startup";
const proof = { base, workingDiff: true, samples: [] };
const state = () => browser.execute(() => ({
  days: document.querySelectorAll("[data-feed-day]").length,
  retired: document.querySelectorAll("[data-feed-retired]").length,
  blocks: document.querySelectorAll(".journal-feed-day .ls-block").length,
  top: document.querySelector(".main-content")?.scrollTop,
  height: document.querySelector(".main-content")?.scrollHeight,
}));
const frames = (n = 5) => browser.executeAsync((count, done) => {
  const tick = () => { if (--count <= 0) done(); else requestAnimationFrame(tick); }; requestAnimationFrame(tick);
}, n);
try {
  const deadline = Date.now() + 15000;
  while (true) {
    const ready = await new Promise(resolve => {
      const socket = net.connect({ host: "127.0.0.1", port }, () => { socket.destroy(); resolve(true); });
      socket.on("error", () => resolve(false));
    });
    if (ready) break;
    if (Date.now() > deadline) throw new Error("tauri-driver did not listen");
    await delay(100);
  }
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 1,
    connectionRetryTimeout: 60000, capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP || path.join(ROOT, "target/debug/tine") } } });
  await ensureMainWindow(browser);
  await browser.$(".page-section").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Other");
  await openJournals(browser);
  journey = "scroll long feed down with stable reading position through retirement";
  while ((await state()).days < 60) {
    const before = await state();
    await browser.execute(() => { const sc = document.querySelector(".main-content"); sc.scrollTop = sc.scrollHeight; });
    await frames();
    const scrolled = await state();
    await browser.waitUntil(async () => (await state()).days > before.days, { timeout: 15000, interval: 100 });
    await frames();
    const after = await state();
    // Appending and retiring above the reader must not move the scroll offset.
    if (Math.abs(after.top - scrolled.top) > 2) throw new Error(`retire moved scroll: ${JSON.stringify({ scrolled, after })}`);
    proof.samples.push(after);
  }
  await browser.waitUntil(async () => (await state()).retired >= 48, { timeout: 10000 });
  const down = await state();
  if (down.blocks > 360) throw new Error(`unbounded live feed: ${JSON.stringify(down)}`);
  journey = "Find loaded documents and omitted-day hint";
  await browser.keys(["Control", "f"]);
  await browser.$(".inpage-find-input").waitForExist({ timeout: 5000 });
  await browser.$(".inpage-find-input").setValue("Feed marker");
  await browser.waitUntil(async () => /\d+ \/ \d+/.test(await browser.$(".inpage-find-count").getText()), { timeout: 5000 });
  let findSnapshot;
  await browser.waitUntil(async () => {
    // Query reveal can load adjacent days between WebDriver round trips. Read
    // the reactive answer and its omission count in one application turn.
    findSnapshot = await browser.execute(() => ({
      count: Number(document.querySelector(".inpage-find-count")?.textContent.split("/")[1]),
      hint: document.querySelector(".inpage-find-retired")?.textContent,
      retired: document.querySelectorAll("[data-feed-retired]").length,
    }));
    return findSnapshot.count > 0 && findSnapshot.count <= 360
      && findSnapshot.hint.includes(`${findSnapshot.retired} older days not searched`);
  }, { timeout: 10000, timeoutMsg: "Find did not show bounded loaded matches with the current retired-day count" });
  const { count, hint } = findSnapshot;
  await browser.keys(["Escape"]);
  journey = "return to a retired day without scroll displacement";
  await browser.execute(() => {
    const sc = document.querySelector(".main-content");
    window.__feedReloadTrace = [];
    const sample = () => window.__feedReloadTrace.push({ top: sc.scrollTop, height: sc.scrollHeight,
      active: document.activeElement?.className,
      days: [...document.querySelectorAll("[data-feed-day]")].slice(0, 3).map(el => ({
        height: el.getBoundingClientRect().height, retired: el.hasAttribute("data-feed-retired"), top: el.getBoundingClientRect().top })) });
    sc.scrollTop = 100; sample();
    let left = 30;
    const frame = () => { sample(); if (--left) requestAnimationFrame(frame); }; requestAnimationFrame(frame);
  });
  const reloading = await state();
  await browser.waitUntil(() => browser.execute(() => document.querySelector("[data-feed-day]")?.textContent.includes("Feed marker day 0 row 0")), { timeout: 10000 });
  await frames();
  const returned = await state();
  if (Math.abs(returned.top - reloading.top) > 2 || Math.abs(returned.height - reloading.height) > 2) {
    throw new Error(`reload changed geometry: ${JSON.stringify({ reloading, returned })}`);
  }
  await browser.saveScreenshot(path.join(run, "reloaded.png"));
  journey = "edit retired-then-reloaded day and verify guarded save";
  await browser.$("[data-feed-day] .block-content").click();
  const editor = await browser.$(".block-editor"); await editor.waitForExist({ timeout: 5000 });
  await editor.click();
  await browser.keys(["End"]);
  await browser.keys(" edited after reload");
  await browser.keys(["Escape"]);
  await waitForFileText(days[0], text => text.includes("edited after reload"));
  // Unrelated row bytes remain in the reloaded document's persisted snapshot.
  const saved = fs.readFileSync(days[0], "utf8");
  if (!saved.includes("Feed marker day 0 row 29")) throw new Error("reload/save lost a sibling");
  proof.down = down; proof.returned = returned; proof.findCount = count; proof.hint = hint;
  proof.outcome = "passed";
  fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS L10-03: ${JSON.stringify({ down, returned, count, hint, run })}`);
} catch (error) {
  try { fs.writeFileSync(path.join(run, "reload-trace.json"), JSON.stringify(await browser.execute(() => window.__feedReloadTrace), null, 2)); } catch {}
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${base} + working diff\nJourney: ${journey}\nExpected: bounded documents, stable viewport, reload/save and Find hint\nObserved: ${error.stack}\nClassification: ambiguous; inspect driver/DOM\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
