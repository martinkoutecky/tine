// GH #858: "Review" on the changed-on-disk banner brings the page's conflict
// review into view. The reporter had a long page open, scrolled down, while its
// file changed on disk; Review opened the page already open, which did nothing,
// so the review at the top of the page stayed out of sight. Here: a long page,
// scrolled down, unsaved input, an external write; Review must show the
// resolver (scrolled to it, or unrolled from its docked bar) and write nothing.
// Synthetic scratch graph and app data only. Run a custom-protocol debug build
// under xvfb-run (TINE_APP overrides target/debug/tine).
import "./lib/e2e-auto-account-home.mjs";
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { currentPageTitle, openJournals, openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "target/artifacts/conflict-review-scroll");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const LINES = 160;
const lines = Array.from({ length: LINES }, (_, i) => `line ${i + 1} of the trip plan`);
const file = path.join(graph, "pages/Long Trip.md");
fs.writeFileSync(file, lines.map(line => `- ${line}\n`).join(""));
const app = process.env.TINE_APP || path.join(root, "target/debug/tine");
const env = { ...process.env, TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4912);
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver",
  webdriverServerArgs(port, port + 1, "/usr/bin/WebKitWebDriver"),
  { env, stdio: ["ignore", log, log], detached: true });
const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex") };
let browser, stage = "startup";
const shot = name => {
  try { execFileSync("import", ["-window", "root", path.join(out, name)], { env, timeout: 10000 }); } catch {}
};
/** Whether the page's resolver is on screen: inside the pane's visible area,
 *  or unrolled in the docked sheet (fixed to the pane's top). */
const resolverOnScreen = () => browser.execute(() => {
  const scroller = document.querySelector(".main-content");
  const panel = document.querySelector(".main-content .page-conflict");
  if (!scroller || !panel) return { visible: false, why: "no resolver" };
  const r = panel.getBoundingClientRect();
  const s = scroller.getBoundingClientRect();
  const visible = r.height > 0 && r.bottom > s.top + 8 && r.top < s.bottom - 8;
  return { visible, sheet: Boolean(panel.closest(".page-conflict-sheet")), top: Math.round(r.top), bottom: Math.round(r.bottom),
    paneTop: Math.round(s.top), paneBottom: Math.round(s.bottom), scrollTop: Math.round(scroller.scrollTop) };
});
try {
  await sleep(1200);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error",
    connectionRetryCount: 0, connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(app, "conflict-review-scroll") });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await openPageByName(browser, "Long Trip");

  // The reporter's state: editing far down a long page while another writer
  // changes its file. The write races the send debounce; a lost race (the
  // edit saved first) only reloads the page, so try again.
  stage = "unsaved input far down a long page meets a file changed on disk";
  let conflicted = false;
  for (let attempt = 1; attempt <= 6 && !conflicted; attempt++) {
    const target = lines[LINES - 5];
    await browser.waitUntil(() => browser.execute(target => {
      for (const node of document.querySelectorAll("[data-review-target]")) node.removeAttribute("data-review-target");
      const scroller = document.querySelector(".main-content");
      if (scroller) scroller.scrollTop = scroller.scrollHeight;
      const el = [...document.querySelectorAll(".main-content .ls-block .block-content-wrapper")]
        .find(node => node.textContent.trim().startsWith(target));
      el?.setAttribute("data-review-target", "1");
      el?.scrollIntoView({ block: "center" });
      return Boolean(el);
    }, target), { timeout: 15000, timeoutMsg: "the block near the end of the page never rendered" });
    await browser.$("[data-review-target]").click();
    await browser.$(".main-content textarea.block-editor").waitForExist({ timeout: 5000 });
    await browser.keys(["End"]);
    await browser.keys(" MINE");
    fs.writeFileSync(file, `- line 1 of the trip plan DISK${attempt}\n` + lines.slice(1).map(line => `- ${line}\n`).join(""));
    conflicted = await browser.waitUntil(() => browser.execute(() =>
      [...document.querySelectorAll(".conflict-banner")].some(el => el.textContent.includes("changed on disk while you were editing"))),
    { timeout: 6000 }).then(() => true, () => false);
    if (!conflicted) {
      await browser.keys("Escape");
      await sleep(1500);
    }
  }
  assert.ok(conflicted, "no save conflict could be produced");
  await browser.keys("Escape");
  const diskBefore = fs.readFileSync(file, "utf8");

  stage = "the resolver is out of sight before Review";
  await browser.waitUntil(() => browser.execute(() => Boolean(document.querySelector(".main-content .page-conflict"))),
    { timeout: 15000, timeoutMsg: "the page never mounted its conflict review" });
  await sleep(500);
  const before = await resolverOnScreen();
  assert.equal(before.visible, false, `precondition: the review must start off screen on a page scrolled down: ${JSON.stringify(before)}`);
  shot("before-review.png");

  stage = "Review on the banner shows the resolver";
  assert.ok(await browser.execute(() => {
    const button = [...document.querySelectorAll(".conflict-banner button")].find(el => el.textContent.trim() === "Review");
    button?.click();
    return Boolean(button);
  }), "the banner has no Review button");
  let after;
  await browser.waitUntil(async () => (after = await resolverOnScreen()).visible, { timeout: 5000,
    timeoutMsg: `Review did not bring the conflict review into view (GH #858): ${JSON.stringify(await resolverOnScreen())}` });
  shot("after-review.png");
  assert.ok(await browser.execute(() => [...document.querySelectorAll(".conflict-banner")]
    .some(el => el.textContent.includes("changed on disk while you were editing"))), "Review must not resolve the conflict by itself");
  await sleep(1000);
  assert.equal(fs.readFileSync(file, "utf8"), diskBefore, "Review wrote to the page file");

  stage = "Review from another page opens the page and shows the resolver";
  await openJournals(browser);
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".main-content .page-conflict")),
    { timeout: 10000, timeoutMsg: "the journals view still shows the page's review" });
  assert.ok(await browser.execute(() => {
    const button = [...document.querySelectorAll(".conflict-banner button")].find(el => el.textContent.trim() === "Review");
    button?.click();
    return Boolean(button);
  }), "the banner lost its Review button away from the page");
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === "Long Trip", { timeout: 10000,
    timeoutMsg: "Review did not open the conflicted page" });
  let elsewhere;
  await browser.waitUntil(async () => (elsewhere = await resolverOnScreen()).visible, { timeout: 5000,
    timeoutMsg: `Review from another page did not show the review: ${JSON.stringify(await resolverOnScreen())}` });
  shot("after-review-from-journals.png");
  assert.equal(fs.readFileSync(file, "utf8"), diskBefore, "Review wrote to the page file");
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify({ ...proof, outcome: "PASS", before, after, elsewhere }, null, 2));
  console.log(`PASS CONFLICT-REVIEW-SCROLL: ${out} ${JSON.stringify(after)}`);
} catch (error) {
  shot("failure.png");
  try { fs.writeFileSync(path.join(out, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage,
    expected: "Review on the changed-on-disk banner shows the page's conflict review on a long page scrolled down, and writes nothing",
    observed: String(error.stack), classification: "ambiguous; inspect driver log and DOM" }, null, 2));
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
