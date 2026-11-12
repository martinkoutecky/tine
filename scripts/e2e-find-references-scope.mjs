// GH #935: Find in page counts the page's own content; Linked References, whose groups mount
// while the reader scrolls, are named as left out and searched through their own filter.
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const base = execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
const artifacts = path.join(ROOT, "target/artifacts/og-935");
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "tmp"]) fs.mkdirSync(path.join(run, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(graph, "pages/Target.md"), "- Quokka on the page itself\n");
fs.writeFileSync(path.join(graph, "pages/Other.md"), "- Other route\n");
// 40 source pages, 2 referencing blocks each (80 < the 100-block fold threshold): a long
// Linked References section whose groups mount only near the viewport. The last group
// holds a word that appears nowhere else.
const GROUPS = 40;
for (let i = 0; i < GROUPS; i++) {
  const rare = i === GROUPS - 1 ? " rare-final-marker" : "";
  fs.writeFileSync(path.join(graph, "pages", `Src${String(i).padStart(2, "0")}.md`),
    `- [[Target]] Quokka sighting ${i} a${rare}\n- [[Target]] Quokka sighting ${i} b\n`);
}
const port = Number(process.env.E2E_DRIVER_PORT || 4793);
const env = { ...process.env, HOME: path.join(run, "home"), TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), TMPDIR: path.join(run, "tmp"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "/opt/cargo", "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
let journey = "startup";
const proof = { base, workingDiff: true, samples: [] };
const frames = (n = 5) => browser.executeAsync((count, done) => {
  const tick = () => { if (--count <= 0) done(); else requestAnimationFrame(tick); }; requestAnimationFrame(tick);
}, n);
const snapshot = () => browser.execute(() => ({
  count: document.querySelector(".inpage-find-count")?.textContent ?? null,
  hints: [...document.querySelectorAll(".inpage-find-retired")].map(el => el.textContent),
  mountedRows: document.querySelectorAll(".linked-references .reference-blocks .ls-block").length,
  groups: document.querySelectorAll(".linked-references .reference-group").length,
  top: document.querySelector(".main-content")?.scrollTop,
}));
const openFind = async (query) => {
  await browser.keys(["Control", "f"]);
  await browser.$(".inpage-find-input").waitForExist({ timeout: 5000 });
  await browser.$(".inpage-find-input").setValue(query);
};
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
      "tauri:options": { application: process.env.TINE_APP || path.join(ROOT, "target/release/tine") } } });
  await ensureMainWindow(browser);
  await browser.$(".page-section").waitForExist({ timeout: 30000 });

  journey = "a word only in a never-mounted group: Find says what it left out, the section search finds it";
  await openPageByName(browser, "Target");
  await browser.$(".linked-references .reference-group").waitForExist({ timeout: 20000 });
  await browser.waitUntil(async () => (await snapshot()).groups === GROUPS, { timeout: 20000, timeoutMsg: "linked reference groups did not list" });
  await openFind("rare-final-marker");
  await browser.waitUntil(async () => (await snapshot()).hints.some(h => h.includes("Linked references not searched")), { timeout: 8000,
    timeoutMsg: "Find did not name the Linked References panel it leaves out" });
  const rare = await snapshot();
  // The marker group sits far below the fold and is not mounted, so no Find that counted DOM could see it.
  if (rare.count !== "No results") throw new Error(`Find should count only the page for the rare word: ${JSON.stringify(rare)}`);
  await browser.saveScreenshot(path.join(run, "find-hint.png"));
  await browser.$(".inpage-find-retired button").click();
  await browser.$(".reference-filter-search").waitForExist({ timeout: 5000 });
  await browser.waitUntil(() => browser.execute(() => document.querySelector(".reference-filter-summary")?.textContent.startsWith("1 of ")), { timeout: 8000,
    timeoutMsg: "the section search did not find the unmounted match" });
  const handed = await browser.execute(() => ({
    value: document.querySelector(".reference-filter-search")?.value,
    summary: document.querySelector(".reference-filter-summary")?.textContent,
    findOpen: !!document.querySelector(".inpage-find-bar"),
    focused: document.activeElement?.classList.contains("reference-filter-search"),
    groupTexts: [...document.querySelectorAll(".linked-references .reference-group")].map(el => el.textContent),
  }));
  if (handed.value !== "rare-final-marker" || handed.findOpen || !handed.focused || handed.groupTexts.length !== 1 || !handed.groupTexts[0].includes("Src39"))
    throw new Error(`hand-off failed: ${JSON.stringify(handed)}`);
  await frames();
  await browser.saveScreenshot(path.join(run, "section-search.png"));
  proof.handed = handed;

  journey = "the count never changes while Linked References mount on scroll";
  await openPageByName(browser, "Other");
  await openPageByName(browser, "Target");
  await browser.$(".linked-references .reference-group").waitForExist({ timeout: 20000 });
  await openFind("Quokka");
  await browser.waitUntil(async () => (await snapshot()).count === "1 / 1", { timeout: 8000, timeoutMsg: "Find did not count only the page's own match" });
  const first = await snapshot();
  let maxMounted = first.mountedRows;
  for (let step = 0; step < 24; step++) {
    await browser.execute(() => { const sc = document.querySelector(".main-content"); sc.scrollTop += Math.round(sc.clientHeight * 0.8); });
    await frames(4);
    const now = await snapshot();
    proof.samples.push(now);
    if (now.count !== "1 / 1") throw new Error(`count changed during scroll: ${JSON.stringify({ first, now })}`);
    if (!now.hints.some(h => h.includes("Linked references not searched"))) throw new Error(`hint missing during scroll: ${JSON.stringify(now)}`);
    maxMounted = Math.max(maxMounted, now.mountedRows);
  }
  // Groups really did load while the count held; otherwise this journey proves nothing.
  if (maxMounted <= first.mountedRows) throw new Error(`no reference group mounted during scroll: ${JSON.stringify({ first, maxMounted })}`);
  await browser.saveScreenshot(path.join(run, "scrolled.png"));
  proof.first = first; proof.maxMounted = maxMounted;

  journey = "a folded Linked References panel is still named, and the hand-off unfolds it";
  await browser.execute(() => {
    const header = document.querySelector(".linked-references .references-header");
    header.scrollIntoView({ block: "center" });
    header.click();
  });
  await browser.waitUntil(async () => (await snapshot()).groups === 0, { timeout: 8000, timeoutMsg: "the panel did not fold" });
  await frames(4);
  const folded = await snapshot();
  if (folded.count !== "1 / 1" || !folded.hints.some(h => h.includes("Linked references not searched")))
    throw new Error(`a folded panel was not named: ${JSON.stringify(folded)}`);
  await browser.$(".inpage-find-retired button").click();
  await browser.$(".reference-filter-search").waitForExist({ timeout: 5000 });
  await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".linked-references .reference-filter-summary").length === 1
    && document.querySelector(".linked-references .reference-filter-summary").textContent.startsWith("80 of 80 ")
    && document.querySelector(".reference-filter-search")?.value === "Quokka"), { timeout: 8000,
    timeoutMsg: "the hand-off did not unfold the panel and search it" });
  proof.folded = folded;
  proof.outcome = "passed";
  fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS GH935: ${JSON.stringify({ first, maxMounted, handed, run })}`);
} catch (error) {
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${base} + working diff\nJourney: ${journey}\nExpected: Find count fixed while references mount; reference panel named; section search finds unmounted match\nObserved: ${error.stack}\nClassification: ambiguous; inspect driver/DOM\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
