// REG-COST-JOURNAL-EDIT-NO-FEED-RESTART (D-20261102-PERF): editing an older
// journal's text in a scrolled Journals feed keeps the feed. The reader stays
// on the edited day, every loaded day stays loaded, and the edited body is
// both shown and saved. Before the fix, the edit's change summary carried the
// `days` hint, so ending the edit restarted the feed at its first page.
//
// UI-FEED-FIRST-CHANGE-RESTART: the edit is the session's first change. The
// window takes its change-summary baseline from the open answer, so that
// first summary is a delta; without it, it read as full and restarted the feed.
//
// "Nothing happened" has no event of its own. The settle point is a later,
// ordered one: after the edit is saved, ticking a task on the same day saves
// again, and the day's `(task DONE)` query shows that task only once the
// later publication is applied. Any restart the edit's summary caused started
// before it.
// GH #987: a persisted fold above today's Scheduled & Deadline section must
// also keep the older day on screen after the next edit is saved by a click.
// A real planning change refreshes the feed. Adding a scheduled date in the
// editor and changing it from disk must both keep the loaded window in place.
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
import { seedClassicEditorProfile } from "./lib/e2e-classic-profile.mjs";
import { editorState } from "./lib/e2e-block-editor.mjs";

ensurePrivateSessionBus();

const ROOT = path.resolve(import.meta.dirname, "..");
const base = process.env.TINE_SOURCE_REVISION || execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
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
const dateAt = offset => {
  const date = new Date();
  date.setDate(date.getDate() + offset);
  return date;
};
const stampFor = date => `<${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")} ${date.toLocaleDateString("en-US", { weekday: "short" })}>`;
const days = Array.from({ length: DAYS }, (_, i) => {
  const date = dateAt(-i);
  const stem = `${date.getFullYear()}_${String(date.getMonth() + 1).padStart(2, "0")}_${String(date.getDate()).padStart(2, "0")}`;
  const file = path.join(graph, "journals", `${stem}.md`);
  const query = i === EDITED || i === 0 ? ["- {{query (task DONE)}}"] : [];
  const rows = Array.from({ length: 24 }, (_, b) => {
    const isTask = i === 0 && b === 5 || i === EDITED && [5, 6, 7].includes(b);
    return `- ${isTask ? "TODO " : ""}Feed day ${i} row ${b}`;
  });
  if (i === 0) {
    rows.push("- Fold above agenda", "  - Foldable child", `- TODO Scheduled fixture\n  SCHEDULED: ${stampFor(date)}`);
  }
  fs.writeFileSync(file, [rows[0], ...query, ...rows.slice(1)].join("\n") + "\n");
  return file;
});
// A real owner lease (not the arbitration fallback) for app and driver.
const account = installAccountHome(run);
const port = Number(process.env.E2E_DRIVER_PORT || 4797);
const env = { ...process.env, TINE_GRAPH: graph, XDG_DATA_HOME: path.join(run, "data"),
  XDG_CONFIG_HOME: path.join(run, "config"), XDG_CACHE_HOME: path.join(run, "cache"), TMPDIR: path.join(run, "tmp"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
seedClassicEditorProfile(env);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || path.join(process.env.CARGO_HOME || "/opt/cargo", "bin/tauri-driver"),
  ["--port", String(port), "--native-port", String(port + 1), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
let browser;
let journey = "startup";
const proof = { base, application: process.env.TINE_APP, editor: process.env.E2E_CLASSIC_PROFILE === "1" ? "classic" : "CodeMirror" };
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
    at: performance.now(),
  };
}, marker);
const frames = (n = 5) => browser.executeAsync((count, done) => {
  const tick = () => { if (--count <= 0) done(); else requestAnimationFrame(tick); }; requestAnimationFrame(tick);
}, n);
async function scrollToOlderDay() {
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
}
async function clickMargin() {
  const margin = await browser.execute(() => {
    const box = document.querySelector(".main-content").getBoundingClientRect();
    return { x: Math.round(box.right - 24), y: Math.round(box.top + box.height / 2) };
  });
  await browser.performActions([{ type: "pointer", id: "journal-margin", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", origin: "viewport", duration: 0, ...margin },
    { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 },
  ] }]);
  await browser.releaseActions();
}
async function installTrace() {
  const installed = await browser.executeAsync(done => {
    const scroller = document.querySelector(".main-content");
    const trace = window.__journalEditTrace = { reads: [], publications: [], scrolls: [] };
    scroller.addEventListener("scroll", () => trace.scrolls.push({ at: performance.now(), top: scroller.scrollTop }));
    const fetch = window.fetch;
    window.fetch = function(input, init) {
      const url = new URL(typeof input === "string" ? input : input.url);
      if ((url.protocol !== "ipc:" && url.hostname !== "ipc.localhost") || url.pathname !== "/journal_feed_page") {
        return fetch.call(this, input, init);
      }
      const read = { at: performance.now(), request: JSON.parse(init?.body || "{}") };
      trace.reads.push(read);
      return fetch.call(this, input, init).finally(() => { read.finished = performance.now(); });
    };
    const ipc = window.__TAURI_INTERNALS__;
    const listen = event => ipc.invoke("plugin:event|listen", { event, target: { kind: "Any" },
      handler: ipc.transformCallback(({ payload }) => {
        const summary = payload?.answers?.summary;
        trace.publications.push({ at: performance.now(), event, summary });
      }) });
    Promise.all(["graph-changed", "graph-changed-bulk"].map(listen)).then(() => done(true), error => done(String(error)));
  });
  if (installed !== true) throw new Error(`could not trace native publications: ${installed}`);
}
async function waitForRefresh(before) {
  // Both the initial two-stage read and a refresh of the complete window
  // deliver the referenced days via an unpaged request.
  await browser.waitUntil(() => browser.execute(at => {
    const reads = window.__journalEditTrace.reads.filter(read => read.at >= at && read.request.beforeDay === null);
    return reads.some(read => read.request.waitForReferences) && reads.every(read => read.finished);
  }, before.at), { timeout: 20000, interval: 100, timeoutMsg: "the planning change did not settle a feed refresh" });
  await frames();
}
function lostPlace(before, after) {
  return after.removed !== before.removed || before.names.some(name => !after.names.includes(name)) ||
    after.editedTop === null || Math.abs(after.editedTop - before.editedTop) > before.viewport / 4;
}
async function tickAndWait(name, row) {
  const task = `Feed day ${EDITED} row ${row}`;
  // The day's own task. The checkbox toggles on mousedown; find and press in one turn.
  const ticked = await browser.execute((name, task) => {
    const day = document.querySelector(`[data-feed-day="${name}"]`);
    if (!day || window.__feedDaysRemoved) return "feed-changed";
    const box = [...day.querySelectorAll(".block-content")]
      .find((el) => !el.closest(".query-block") && el.textContent.includes(task))?.querySelector(".block-task-checkbox");
    box?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    return Boolean(box);
  }, name, task);
  if (ticked === "feed-changed") return;
  if (!ticked) throw new Error(`the edited day has no checkbox for ${task}`);
  await waitForFileText(days[EDITED], (text) => text.includes(`DONE ${task}`));
  await browser.waitUntil(() => browser.execute((name, task) => {
    const day = document.querySelector(`[data-feed-day="${name}"]`);
    return !day || [...day.querySelectorAll(".query-block .block-content")].some((el) => el.textContent.includes(task));
  }, name, task), { timeout: 20000, interval: 100, timeoutMsg: "the edited day's DONE query never showed the ticked task" });
  await frames();
}
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

  journey = "scroll the Journals feed past its first page to an older day";
  await installTrace();
  await scrollToOlderDay();
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
  await browser.waitUntil(async () => (await editorState(browser, ".main-content"))?.focused, { timeout: 5000 });
  await browser.keys(["End"]);
  await browser.keys(" edited in a journal");
  await browser.keys(["Escape"]);
  await browser.waitUntil(async () => !(await editorState(browser, ".main-content")),
    { timeout: 5000, timeoutMsg: "editing did not end" });
  const saved = await waitForFileText(days[EDITED], (text) => text.includes(`${marker} edited in a journal`));
  if (!saved.includes(`Feed day ${EDITED} row 23`)) throw new Error("save lost a sibling row");

  journey = "settle on a later ordered publication";
  await tickAndWait(before.editedName, 5);
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

  journey = "fold today's outline above the agenda, then click away from an older edit";
  await browser.execute(() => { document.querySelector(".main-content").scrollTop = 0; });
  const foldHost = () => browser.execute(() => {
    const today = document.querySelector("[data-feed-day]");
    const row = [...today.querySelectorAll(".block-content")].find(el => el.textContent === "Fold above agenda");
    const agenda = today.querySelector(".agenda-block .query-title");
    return row && agenda && Boolean(row.compareDocumentPosition(agenda) & Node.DOCUMENT_POSITION_FOLLOWING);
  });
  await browser.waitUntil(foldHost, { timeout: 15000, timeoutMsg: "today's foldable block and agenda did not render" });
  const positioned = await browser.execute(name => {
    const scroller = document.querySelector(".main-content");
    const today = scroller.querySelector("[data-feed-day]");
    const row = [...today.querySelectorAll(".block-content")].find(el => el.textContent === "Fold above agenda");
    row.closest("[data-block-id]").querySelector(".collapse-toggle.has-children").click();
    const day = scroller.querySelector(`[data-feed-day="${name}"]`);
    if (!day) return false;
    day.scrollIntoView({ block: "start" });
    scroller.scrollBy(0, -40);
    return true;
  }, before.editedName);
  if (!positioned) throw new Error("folding today's outline removed the older day");
  const edited = await browser.$(`[data-feed-day="${before.editedName}"] .block-content`);
  await edited.waitForExist({ timeout: 5000 });
  await edited.click();
  await browser.waitUntil(async () => (await editorState(browser, ".main-content"))?.focused, { timeout: 5000 });
  const foldedBefore = await state();
  proof.foldedBefore = foldedBefore;
  await browser.keys(["End"]);
  await browser.keys(" after today's fold");
  await clickMargin();
  await waitForFileText(days[0], text => text.includes("collapsed:: true"));
  const foldedText = `${marker} edited in a journal after today's fold`;
  await waitForFileText(days[EDITED], text => text.includes(foldedText));
  if ((await state()).removed === 0) await tickAndWait(before.editedName, 6);
  const foldedAfter = await state();
  proof.afterFolding = { before: foldedBefore, after: foldedAfter };
  const foldLost = foldedAfter.removed !== 0 || foldedAfter.editedTop === null ||
      Math.abs(foldedAfter.editedTop - foldedBefore.editedTop) > foldedBefore.viewport / 4 ||
      !foldedAfter.editedText.includes(foldedText);
  if (foldLost) {
    await browser.saveScreenshot(path.join(run, "fold-failure.png"));
    await scrollToOlderDay();
  }

  journey = "add a scheduled date to the older journal and click away";
  const task = `Feed day ${EDITED} row 7`;
  const taskDay = await browser.$(`[data-feed-day="${before.editedName}"]`);
  const taskRow = await taskDay.$(`.//*[contains(concat(" ", normalize-space(@class), " "), " block-content ") and contains(., "${task}")]`);
  await taskRow.click();
  await browser.waitUntil(async () => (await editorState(browser, ".main-content"))?.focused, { timeout: 5000 });
  const planningBefore = await state();
  const scheduledStamp = stampFor(dateAt(0));
  const scheduled = `SCHEDULED: ${scheduledStamp}`;
  await browser.keys(["End"]);
  await browser.keys(["Shift", "Enter"]);
  const typing = browser.action("key");
  for (const key of scheduled) typing.down(key).up(key);
  await typing.perform();
  await browser.waitUntil(async () => (await editorState(browser, ".main-content"))?.value.includes(scheduled),
    { timeout: 5000, timeoutMsg: "the scheduled date did not reach the editor" });
  await clickMargin();
  await waitForFileText(days[EDITED], text => text.includes(scheduled));
  await waitForRefresh(planningBefore);
  const planningAfter = await state();
  proof.planning = { before: planningBefore, after: planningAfter, kept: !lostPlace(planningBefore, planningAfter) };
  if (proof.planning.kept && !planningAfter.editedText.includes(scheduledStamp.slice(1, -1))) {
    throw new Error("the older journal did not show the saved scheduled date");
  }
  if (!proof.planning.kept) await browser.saveScreenshot(path.join(run, "planning-failure.png"));

  journey = "change that older journal's scheduled date from an external editor";
  if (!proof.planning.kept) await scrollToOlderDay();
  const externalBefore = await state();
  const externalTask = `${task} updated on disk`;
  const externalStamp = stampFor(dateAt(1));
  const externalDate = `SCHEDULED: ${externalStamp}`;
  const replacement = path.join(run, "external-journal.md");
  const current = fs.readFileSync(days[EDITED], "utf8");
  fs.writeFileSync(replacement, current.replace(task, externalTask).replace(scheduled, externalDate));
  fs.renameSync(replacement, days[EDITED]);
  await waitForRefresh(externalBefore);
  await browser.waitUntil(async () => {
    const after = await state();
    return lostPlace(externalBefore, after) ||
      (after.editedText.includes(externalTask) && after.editedText.includes(externalStamp.slice(1, -1)));
  }, { timeout: 20000, interval: 100, timeoutMsg: "the outside editor's change did not appear" });
  const externalAfter = await state();
  proof.external = { before: externalBefore, after: externalAfter, kept: !lostPlace(externalBefore, externalAfter) };
  const failures = [foldLost && "afterFolding", ...["planning", "external"].filter(key => !proof[key].kept)].filter(Boolean);
  if (failures.length) {
    proof.regressions = failures;
    journey = "keep the older journal in view across the three refresh cases";
    throw new Error(failures.map(key => {
      const { before, after } = proof[key];
      return `${key}: scroll ${before.top} -> ${after.top}, days removed ${after.removed - before.removed}`;
    }).join("; "));
  }
  await browser.saveScreenshot(path.join(run, "kept.png"));
  proof.outcome = "passed";
  proof.trace = await browser.execute(() => window.__journalEditTrace);
  fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify(proof, null, 2));
  console.log(`PASS journal-edit-feed-kept: ${JSON.stringify({ before: proof.before, after: proof.after, run })}`);
} catch (error) {
  try { proof.trace = await browser.execute(() => window.__journalEditTrace); } catch {}
  try { proof.failedState = await state(); } catch {}
  try { await browser?.saveScreenshot(path.join(run, "failure.png")); } catch {}
  try { fs.writeFileSync(path.join(run, "failure-dom.html"), await browser.getPageSource()); } catch {}
  fs.writeFileSync(path.join(run, "proof.json"), JSON.stringify(proof, null, 2));
  const classification = proof.regressions ? "product" : "ambiguous";
  fs.writeFileSync(path.join(run, "failure.txt"), `Tested: ${base} (${proof.application ?? "target/debug/tine"})\nJourney: ${journey}\nExpected: the scrolled feed keeps its loaded days, the edited day's position and the edited body after a journal edit\nObserved: ${error.stack}\nClassification: ${classification}; inspect proof.json, driver.log and failure-dom.html\n`);
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
