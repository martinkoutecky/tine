// Durable margin-comment journey. Baseline mode records missing affordances and
// continues via the existing shortcut so later defects can also be observed.
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { openPageByName, openJournals } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE = process.env.MARGIN_BASELINE === "1";
const OUT = path.join(ROOT, "artifacts/margin-polish", process.env.MARGIN_RUN || (BASELINE ? "baseline" : "candidate"));
const GRAPH = path.join(OUT, "graph");
const PAGE = "Margin Polish";
const FILE = path.join(GRAPH, "pages", `${PAGE}.md`);
const PHRASE = "a phrase worth disputing";
const COMMENT = "I doubt this claim";
const REPLY = "Consider this evidence";
const NESTED = "That changes my view";
const defects = [];
fs.mkdirSync(OUT, { recursive: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(GRAPH, dir), { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(path.join(OUT, "xdg", dir), { recursive: true });
fs.writeFileSync(path.join(GRAPH, "logseq/config.edn"), "{}\n");
fs.writeFileSync(FILE, `- An agent wrote ${PHRASE} in this paragraph.\n  author:: claude\n- An independent block.\n  - Keep this remark\n    quote:: independent\n    author:: Ada Test  \t\n`);
const now = new Date();
const day = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
const JOURNAL = path.join(GRAPH, "journals", `${day}.md`);
fs.writeFileSync(JOURNAL, "- A journal passage worth discussing.\n");
const driverPort = Number(process.env.E2E_DRIVER_PORT || 4680);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4681);
const env = { ...process.env, TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: path.join(OUT, "xdg/data"), XDG_CONFIG_HOME: path.join(OUT, "xdg/config"), XDG_CACHE_HOME: path.join(OUT, "xdg/cache"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const log = fs.openSync(path.join(OUT, "driver.log"), "w");
const td = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(driverPort), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser;
let step = "startup";
async function shot(name) {
  step = name;
  await browser.executeAsync((done) => requestAnimationFrame(() => requestAnimationFrame(() => done())));
  // WebKitGTK's WebDriver screenshot endpoint can hang with software
  // compositing disabled. Capture the actual Xvfb display instead.
  execFileSync("import", ["-window", "root", path.join(OUT, `${name}.png`)], { env: process.env });
  console.log(`screenshot: ${name}`);
}
function check(ok, message) {
  if (ok) return;
  defects.push({ step, message, screenshot: path.join(OUT, `${step}.png`) });
  if (!BASELINE) throw new Error(message);
}
async function readyEditor() {
  await browser.$("textarea.block-editor").waitForExist({ timeout: 10_000 });
  await browser.waitUntil(() => browser.execute(() => document.activeElement?.matches("textarea.block-editor")), { timeout: 5000 });
}
async function select(phrase) {
  const selected = await browser.execute((words) => {
    const ta = document.querySelector("textarea.block-editor");
    const at = ta.value.indexOf(words);
    if (at < 0) return null;
    ta.focus(); ta.setSelectionRange(at, at + words.length);
    ta.dispatchEvent(new Event("select", { bubbles: true }));
    return ta.value.slice(ta.selectionStart, ta.selectionEnd);
  }, phrase);
  if (selected !== phrase) throw new Error(`selection failed: ${selected}`);
}
async function leave() {
  // Escape can first dismiss a selection/formatting layer.
  for (let n = 0; n < 3 && await browser.$("textarea.block-editor").isExisting(); n++) await browser.keys(["Escape"]);
  await browser.$("textarea.block-editor").waitForExist({ reverse: true, timeout: 5000 });
}
async function type(text) { await readyEditor(); await browser.keys(text); }
async function editText(text) {
  const found = await browser.execute((text) => {
    const el = [...document.querySelectorAll(".block-content")].find(el => el.textContent.includes(text));
    if (!el) return false;
    el.dispatchEvent(new MouseEvent("mousedown", { button: 0, bubbles: true, cancelable: true }));
    return true;
  }, text);
  if (!found) throw new Error(`no rendered block: ${text}`);
  await readyEditor();
}
async function action(text, cls) {
  return browser.execute((text, cls) => {
    const row = [...document.querySelectorAll(".ls-block")].find(el => el.querySelector(":scope > .block-main .block-content")?.textContent.includes(text));
    const button = row?.querySelector(`:scope > .comment-actions .${cls}`);
    if (!button) return false;
    button.click(); return true;
  }, text, cls);
}
async function persisted(...words) {
  return waitForFileText(FILE, text => words.every(word => text.includes(word)), { timeoutMs: 20_000 });
}
try {
  await new Promise((resolve, reject) => {
    const deadline = Date.now() + 10_000;
    async function poll() {
      try { if ((await fetch(`http://127.0.0.1:${driverPort}/status`)).ok) return resolve(); } catch {}
      if (Date.now() > deadline) return reject(new Error("tauri-driver did not start"));
      setTimeout(poll, 100);
    }
    poll();
  });
  browser = await remote({ hostname: "127.0.0.1", port: driverPort, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: process.env.TINE_APP || path.join(ROOT, "target/release/tine") } } });
  console.log("native session created");
  await ensureMainWindow(browser);
  console.log(`main window: ${await browser.getUrl()}`);
  await browser.setTimeout({ implicit: 0, script: 10_000, pageLoad: 30_000 });
  console.log(await browser.execute(() => ({ ready: document.readyState, body: document.body?.innerText.slice(0, 1500) })));
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 30_000 });
  await browser.setWindowSize(1600, 1000);
  await openPageByName(browser, PAGE);
  await editText(PHRASE);
  await select(PHRASE);
  await shot("01-selection-toolbar");
  const toolbar = await browser.$('[data-selection-action="comment"]').isExisting();
  check(toolbar, "Selection toolbar has no Comment action");
  if (toolbar) check(await browser.execute(() => {
    const button = document.querySelector('[data-selection-action="comment"]');
    return button.scrollWidth <= button.clientWidth;
  }), "The Comment label overflows its button and overlaps adjacent formatting actions");
  if (toolbar) await browser.$('[data-selection-action="comment"]').click();
  else await browser.keys(["Control", "r"]);
  await readyEditor();
  await type(COMMENT);
  await shot("02-comment-editor");
  check(await browser.execute(() => !document.querySelector("textarea.block-editor").value.includes("quote::")), "Quote metadata is visible while editing");
  await leave();
  await persisted(COMMENT, `quote:: ${PHRASE}`);
  await browser.$(".margin-column .comment-card").waitForExist({ timeout: 5000 });
  await shot("03-margin-card");
  check(await browser.execute((phrase) => document.querySelector(".page-blocks .comment-quote-anchor")?.textContent === phrase, PHRASE), "Passage is not marked");
  await editText(COMMENT);
  await shot("04-reopened-comment");
  check(await browser.execute(() => !document.querySelector("textarea.block-editor").value.includes("quote::")), "Reopening exposes quote metadata");
  await leave();
  const replyButton = await action(COMMENT, "comment-reply");
  check(replyButton, "Card has no Reply affordance");
  if (!replyButton) {
    await editText(COMMENT);
    await browser.execute((text) => { const ta = document.querySelector("textarea.block-editor"); ta.setSelectionRange(text.length, text.length); }, COMMENT);
    await browser.keys(["Enter"]);
  }
  await type(REPLY); await leave(); await persisted(REPLY);
  await shot("05-reply");
  await editText(REPLY); await select("this evidence");
  await browser.execute(() => { window.__marginPolishAlive = true; });
  await browser.keys(["Control", "r"]);
  await type(NESTED); await leave(); await persisted(NESTED, "quote:: this evidence");
  check(await browser.execute(() => window.__marginPolishAlive === true), "Ctrl+R reloaded the app");
  await shot("06-quoted-reply");
  const overlap = await browser.execute(() => [...document.querySelectorAll(".comment-in-thread")].some(row => {
    const header = row.querySelector(":scope > .comment-quote");
    const main = row.querySelector(":scope > .block-main");
    return header && main && header.getBoundingClientRect().bottom > main.getBoundingClientRect().top;
  }));
  check(!overlap, "A thread quote overlaps its comment body");
  check(await browser.execute(() => document.querySelectorAll(".margin-thread").length === 2), "A quoted reply became a separate margin card");
  const canDelete = await action(NESTED, "comment-delete");
  check(canDelete, "Reply has no Delete affordance");
  if (canDelete) {
    await waitForFileText(FILE, text => !text.includes(NESTED) && text.includes(REPLY), { timeoutMs: 20_000 });
    await shot("07-reply-deleted");
    await action(COMMENT, "comment-delete");
    await waitForFileText(FILE, text => !text.includes(COMMENT) && !text.includes(REPLY), { timeoutMs: 20_000 });
    await shot("08-thread-deleted");
    await browser.keys(["Control", "z"]);
    await persisted(COMMENT, REPLY);
    await shot("09-thread-undo");
    check(await browser.execute((reply) => [...document.querySelectorAll(".margin-thread")].some(thread => thread.textContent.includes(reply)), REPLY), "Undo did not restore the thread and replies");
  }
  await browser.setWindowSize(1000, 820);
  await browser.$(".page-blocks .comment-card").waitForExist({ timeout: 5000 });
  await shot("10-narrow-inline");
  check(!await browser.$(".margin-column").isExisting(), "Narrow pane still has a margin column");
  await browser.setWindowSize(1600, 1000);
  // The existing split command is reached through the visible command palette.
  await browser.keys(["Control", "Shift", "p"]);
  const palette = await browser.$(".switcher-input");
  await palette.waitForExist({ timeout: 5000 });
  await palette.setValue("Split right");
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".switcher-row")].some(el => /split.*right/i.test(el.textContent))), { timeout: 5000 });
  const split = await browser.execute(() => {
    const row = [...document.querySelectorAll(".switcher-row")].find(el => /split.*right/i.test(el.textContent));
    if (!row) return false;
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })); return true;
  });
  if (split) {
    await browser.waitUntil(() => browser.execute((comment, reply) => {
      const panes = [...document.querySelectorAll(".pane-leaf")];
      return panes.length === 2 && panes.every(pane => {
        const card = pane.querySelector(".page-blocks .comment-card");
        return card?.textContent.includes(comment) && card.textContent.includes(reply);
      });
    }, COMMENT, REPLY), { timeout: 10_000, timeoutMsg: "split panes did not show the restored thread inline" });
    await shot("11-split-inline");
    check(!await browser.$(".margin-column").isExisting(), "Split pane still draws margin threads");
    // Reload before continuing with the persisted outline.
    await browser.refresh();
    await browser.$(".ls-block").waitForExist({ timeout: 20_000 });
  } else { await browser.keys(["Escape"]); throw new Error("Split right palette command not found"); }
  await openPageByName(browser, PAGE);
  await editText(PHRASE); await select(PHRASE);
  await browser.keys("a revised claim"); await leave();
  await persisted("a revised claim", `quote:: ${PHRASE}`, COMMENT);
  await browser.$(".comment-quote.stale").waitForExist({ timeout: 5000 });
  await shot("12-stale-quote");
  await editText("Keep this remark"); await select("remark");
  await browser.keys("revised remark"); await leave();
  const editedBytes = await persisted("Keep this revised remark");
  await shot("12b-preserved-metadata");
  check(editedBytes.includes("author:: Ada Test  \t"), "Saving a comment trimmed bytes from its final hidden metadata line");
  await openJournals(browser);
  await editText("A journal passage"); await select("journal passage");
  await browser.keys(["Control", "r"]); await type("A journal remark"); await leave();
  const journalBytes = await waitForFileText(JOURNAL, text => text.includes("A journal remark") && text.includes("quote:: journal passage"), { timeoutMs: 20_000 });
  await shot("13-journal-comment");
  const bytes = fs.readFileSync(FILE);
  await browser.refresh();
  await browser.$(".ls-block").waitForExist({ timeout: 20_000 });
  await openPageByName(browser, PAGE);
  await browser.$(".comment-quote.stale").waitForExist({ timeout: 5000 });
  await shot("14-reloaded");
  check(bytes.equals(fs.readFileSync(FILE)), "Reload changed the page bytes");
  check(journalBytes === fs.readFileSync(JOURNAL, "utf8"), "Reload changed the journal bytes");
  if (!BASELINE) {
    if (!await action("Keep this revised remark", "comment-reply")) throw new Error("Reply unavailable for handwritten metadata check");
    await type("A hand note");
    await browser.keys(["Shift", "Enter"]);
    // Individual native key commands release duplicate ':' keys reliably.
    for (const character of "quote:: revised remark") await browser.keys(character);
    await shot("15-handwritten-quote-draft");
    await leave(); await persisted("A hand note", "quote:: revised remark");
    await editText("A hand note");
    check(await browser.execute(() => document.querySelector("textarea.block-editor").value === "A hand note"), "Handwritten quote did not save its value and reopen hidden");
    await shot("16-handwritten-quote-hidden");
    await leave();
  }
  console.log(JSON.stringify({ result: defects.length ? "baseline defects" : "PASS", defects }, null, 2));
} catch (error) {
  console.error(`Failure in ${step}: ${String(error)}`);
  if (browser) { try { console.error(await browser.execute(() => document.body?.innerText.slice(0, 3000))); } catch {} }
  if (browser) { try { await shot(`failure-${step}`); } catch {} }
  fs.writeFileSync(path.join(OUT, "failure.json"), JSON.stringify({ commit: "3d21fc343fa546bb3578f141ab8f3e5a2ebd380f + working diff", journey: step, expected: "durable visible comment thread workflow", observed: String(error), classification: "ambiguous", defects }, null, 2));
  throw error;
} finally {
  fs.writeFileSync(path.join(OUT, "defects.json"), JSON.stringify(defects, null, 2));
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
