// REG-COST-JOURNAL-EDIT-NO-FEED-RESTART (D-20261102-PERF): editing an older
// journal's text in a scrolled Journals feed keeps the feed. The reader stays
// on the edited day, every loaded day stays loaded, and the edited body is
// both shown and saved. Before the fix, the edit's change summary carried the
// `days` hint, so ending the edit restarted the feed at its first page.
//
// "Nothing happened" has no event of its own. The settle point is a later,
// ordered one: after the edit is saved, ticking a task on the same day saves
// again, and the day's `(task DONE)` query shows that task only once the
// later publication is applied. Any restart the edit's summary caused started
// before it.
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { remote } from "webdriverio";
import { openJournals, openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensurePrivateSessionBus } from "./lib/e2e-session-bus.mjs";

ensurePrivateSessionBus();

const ROOT = path.resolve(import.meta.dirname, "..");
const base = execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
const artifacts = path.resolve(process.env.E2E_ARTIFACT_DIR || path.join(ROOT, "artifacts/journal-edit-feed-kept"));
fs.mkdirSync(artifacts, { recursive: true });
const run = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["data", "config", "cache", "tmp"]) fs.mkdirSync(path.join(run, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
fs.writeFileSync(path.join(graph, "pages/Other.md"), "- Other route\n");
const DAYS = 14;
const EDITED = 7; // past the first feed page (FEED_PAGE = 3)
const days = Array.from({ length: DAYS }, (_, i) => {
  const date = new Date(); date.setDate(date.getDate() - i);
  const stem = `${date.getFullYear()}_${String(date.getMonth() + 1).padStart(2, "0")}_${String(date.getDate()).padStart(2, "0")}`;
  const file = path.join(graph, "journals", `${stem}.md`);
  const query = i === EDITED || i === 0 ? ["- {{query (task DONE)}}"] : [];
  const rows = Array.from({ length: 24 }, (_, b) => `- ${(i === EDITED || i === 0) && b === 5 ? "TODO " : ""}Feed day ${i} row ${b}`);
  fs.writeFileSync(file, [rows[0], ...query, ...rows.slice(1)].join("\n") + "\n");
  return file;
});
// A real owner lease (not the arbitration fallback) for app and driver.
const account = installAccountHome(run);
const port = Number(process.env.E2E_DRIVER_PORT || 4797);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), TMPDIR: path.join(run, "tmp"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "/opt/cargo", "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
let journey = "startup";
const proof = { base, workingDiff: true };
const marker = `Feed day ${EDITED} row 0`;
// The feed's loaded days and where the edited day sits in the viewport.
const state = () => browser.execute((marker) => {
  const sc = document.querySelector(".main-content");
  const feed = [...document.querySelectorAll("[data-feed-day]")];
  const edited = feed.find((el) => el.textContent.includes(marker));
  const view = sc.getBoundingClientRect();
  return {
    names: feed.map((el) => el.getAttribute("data-feed-day")),
    editedName: edited?.getAttribute("data-feed-day") ?? null,
    editedTop: edited ? edited.getBoundingClientRect().top - view.top : null,
    editedText: edited?.textContent ?? "",
    viewport: sc.clientHeight,
    top: sc.scrollTop,
    removed: window.__feedDaysRemoved ?? null,
  };
}, marker);
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
  await assertAppLease(account);
  await openPageByName(browser, "Other");
  await openJournals(browser);

  // The session's first publication carries no baseline revision, so the
  // frontend treats it as full (changeSummary.ts) and the feed restarts once.
  // Spend it before scrolling: tick today's task and wait for today's DONE
  // query to list it; the scroll below takes several more round trips.
  journey = "first publication of the session";
  const today = await browser.execute(() => document.querySelector("[data-feed-day]")?.getAttribute("data-feed-day"));
  await browser.$(`[data-feed-day="${today}"] .block-task-checkbox`).click();
  await waitForFileText(days[0], (text) => text.includes("DONE Feed day 0 row 5"));
  await browser.waitUntil(() => browser.execute((name) =>
    [...document.querySelectorAll(`[data-feed-day="${name}"] .query-block .block-content`)].some((el) => el.textContent.includes("Feed day 0 row 5")),
  today), { timeout: 20000, interval: 100, timeoutMsg: "today's DONE query never showed the ticked task" });

  journey = "scroll the Journals feed past its first page to an older day";
  for (let before = await state(); before.names.length < Math.min(EDITED + 3, DAYS); before = await state()) {
    await browser.execute(() => { const sc = document.querySelector(".main-content"); sc.scrollTop = sc.scrollHeight; });
    await browser.waitUntil(async () => (await state()).names.length > before.names.length,
      { timeout: 15000, interval: 100, timeoutMsg: `feed did not load past ${before.names.length} days` });
  }
  await browser.execute((marker) => {
    const day = [...document.querySelectorAll("[data-feed-day]")].find((el) => el.textContent.includes(marker));
    day.scrollIntoView({ block: "start" });
    document.querySelector(".main-content").scrollBy(0, -40);
  }, marker);
  await frames();
  // Count every feed day leaving the DOM from here on (a restart replaces the
  // feed window with its first page).
  await browser.execute(() => {
    window.__feedDaysRemoved = 0;
    const feed = document.querySelector("[data-feed-day]").parentElement;
    new MutationObserver((records) => {
      for (const record of records) for (const node of record.removedNodes) {
        if (node instanceof Element && (node.matches("[data-feed-day]") || node.querySelector("[data-feed-day]"))) {
          window.__feedDaysRemoved += 1;
        }
      }
    }).observe(feed.parentElement ?? feed, { childList: true, subtree: true });
  });
  const before = await state();
  if (before.names.indexOf(before.editedName) < 3) throw new Error(`edited day is on the first page: ${JSON.stringify(before)}`);
  proof.before = { ...before, editedText: undefined };

  journey = "edit the older journal's text and end editing";
  await browser.$(`[data-feed-day="${before.editedName}"] .block-content`).click();
  const editor = await browser.$(".block-editor");
  await editor.waitForExist({ timeout: 5000 });
  await browser.keys(["End"]);
  await browser.keys(" edited in a journal");
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".block-editor")),
    { timeout: 5000, timeoutMsg: "editing did not end" });
  const saved = await waitForFileText(days[EDITED], (text) => text.includes(`${marker} edited in a journal`));
  if (!saved.includes(`Feed day ${EDITED} row 23`)) throw new Error("save lost a sibling row");

  journey = "settle on a later ordered publication";
  // The day's own task, not today's task listed by its DONE query. The
  // checkbox toggles on mousedown (Rendered.tsx); find and press in one turn.
  const ticked = await browser.execute((name) => {
    const box = [...document.querySelectorAll(`[data-feed-day="${name}"] .block-task-checkbox`)]
      .find((el) => !el.closest(".query-block"));
    box?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    return Boolean(box);
  }, before.editedName);
  if (!ticked) throw new Error("the edited day has no task checkbox");
  await waitForFileText(days[EDITED], (text) => text.includes(`DONE Feed day ${EDITED} row 5`));
  // Settled when the day's DONE query lists the task, or when the day has
  // left the feed (a restart; reported below).
  await browser.waitUntil(() => browser.execute((name, task) => {
    const day = document.querySelector(`[data-feed-day="${name}"]`);
    return !day || [...day.querySelectorAll(".query-block .block-content")].some((el) => el.textContent.includes(task));
  }, before.editedName, `Feed day ${EDITED} row 5`),
  { timeout: 20000, interval: 100, timeoutMsg: "the edited day's DONE query never showed the ticked task" });
  await frames();
  const after = await state();
  proof.after = { ...after, editedText: undefined };

  journey = "the feed kept its days, the reader's position and the edited body";
  const lost = before.names.filter((name) => !after.names.includes(name));
  if (lost.length || after.removed !== 0) {
    throw new Error(`the feed restarted: lost ${JSON.stringify(lost)}, removed ${after.removed}; ${JSON.stringify({ before: before.names, after: after.names })}`);
  }
  if (after.editedTop === null || Math.abs(after.editedTop - before.editedTop) > before.viewport / 4) {
    throw new Error(`the edited day moved in the viewport: ${before.editedTop} -> ${after.editedTop} (viewport ${before.viewport})`);
  }
  if (!after.editedText.includes(`${marker} edited in a journal`)) throw new Error("the feed does not show the edited body");
  await browser.saveScreenshot(path.join(run, "kept.png"));
  proof.outcome = "passed";
  fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS journal-edit-feed-kept: ${JSON.stringify({ before: proof.before, after: proof.after, run })}`);
} catch (error) {
  try { proof.failedState = await state(); } catch {}
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify(proof, null, 2));
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${base} + working diff\nJourney: ${journey}\nExpected: the scrolled feed keeps its loaded days, the edited day's position and the edited body after a journal edit\nObserved: ${error.stack}\nClassification: ambiguous; inspect proof.json, driver.log and failure-dom.html\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
