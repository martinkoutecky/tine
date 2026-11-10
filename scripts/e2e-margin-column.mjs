// Real-app journey for margin dialogue slice 2 (vision 2026-10 §3.7 "Layout"):
// on a wide window, Ctrl+R on a selection creates a comment that is drawn in the
// right-hand margin column (not in the outline); clicking it edits it there,
// The reply composer adds a reply, and the page file holds it nested under
// the comment, which is nested under the commented block. GH #845: t then m
// hides every thread (and leaves the file alone), the choice survives a reload,
// and t then m shows them again.
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { seedMarginThreads, checkMarginThreads } from "./lib/margin-thread-journey.mjs";
import { checkCommentStructure, STRUCTURE_INITIAL, STRUCTURE_PAGE } from "./lib/e2e-comment-structure.mjs";
import { saveJourneyScreenshot } from "./lib/e2e-screenshot.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4520);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4521);
const TMP = process.env.E2E_MARGIN_TMP || "/tmp/tine-margin-column-e2e";
const GRAPH = `${TMP}/graph`;
const PAGE_NAME = "Margin Column Test";
const PAGE_FILE = `${GRAPH}/pages/${PAGE_NAME}.md`;
const PHRASE = "a phrase worth disputing";
// No doubled letters: WebDriver key actions drop a repeated key ("ee" types "e").
const COMMENT = "Is this right";
const REPLY = "Yes, it holds";

if (process.env.E2E_MARGIN_TMP) {
  if (fs.existsSync(TMP)) throw new Error("E2E_MARGIN_TMP must be a fresh fixture directory");
} else fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(`${TMP}/home`, { recursive: true });
fs.mkdirSync(`${TMP}/xdg/state`, { recursive: true });
fs.mkdirSync(`${TMP}/xdg/runtime`, { recursive: true, mode: 0o700 });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
// Isolate git discovery from a checkout containing the scratch fixture.
execFileSync("git", ["-C", GRAPH, "init", "-q", "-b", "main"]);
fs.writeFileSync(PAGE_FILE, `- An agent wrote ${PHRASE} in this paragraph.\n  tine.author:: claude\n- A second block stays as it is.\n`);
seedMarginThreads(GRAPH);
fs.writeFileSync(`${GRAPH}/pages/${STRUCTURE_PAGE}.md`, STRUCTURE_INITIAL);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, `- open [[${PAGE_NAME}]]\n`);
const account = accountHomeFixture(TMP, TMP);
const env = {
  ...account.env,
  HOME: `${TMP}/home`, XDG_STATE_HOME: `${TMP}/xdg/state`, XDG_RUNTIME_DIR: `${TMP}/xdg/runtime`,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
const log = fs.openSync(`${TMP}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  await assertAppLease(account);
  // Wide enough for the margin column with the left sidebar open.
  await browser.setWindowSize(1600, 1000);
  await checkMarginThreads(browser, GRAPH, TMP);
  if (process.env.E2E_MARGIN_THREADS_ONLY !== "1") {
  await openPageByName(browser, PAGE_NAME);

  const agent = await browser.$(`.page-blocks .ls-block.authored`);
  await agent.waitForExist({ timeout: 10_000 });
  await (await agent.$(".block-content")).click();
  await (await browser.$(".page-blocks textarea.block-editor")).waitForExist({ timeout: 5_000 });
  const selected = await browser.execute((phrase) => {
    const ta = document.querySelector(".page-blocks textarea.block-editor");
    const at = ta.value.indexOf(phrase);
    ta.focus();
    ta.setSelectionRange(at, at + phrase.length);
    return ta.value.slice(ta.selectionStart, ta.selectionEnd);
  }, PHRASE);
  if (selected !== PHRASE) throw new Error(`selection was ${JSON.stringify(selected)}`);
  await browser.keys(["Control", "r"]);
  await browser.waitUntil(() => browser.execute(() =>
    !!document.querySelector(".comment-name-prompt, .margin-column textarea.block-editor")),
  { timeout: 5000, timeoutMsg: "Comment opened neither a name prompt nor an editor" });
  if (await browser.$(".comment-name-prompt").isExisting()) await browser.$(".comment-name-prompt .export-btn-secondary").click();

  // The new comment opens for editing in the margin column, not in the outline.
  await browser.waitUntil(() => browser.execute(() => {
    const ta = document.querySelector(".margin-column .margin-thread textarea.block-editor");
    return !!ta && ta.value === "" && document.activeElement === ta
      && !document.querySelector(".page-blocks .ls-block.comment-card");
  }), { timeout: 5_000, timeoutMsg: "Ctrl+R did not open the new comment in the margin column" });
  await browser.keys(COMMENT);
  await browser.keys(["Escape"]);
  await browser.keys(["Escape"]);

  // Visible margin threads need no duplicate count badge (MC3 §7).
  await browser.waitUntil(() => browser.execute((comment) => {
    const thread = document.querySelector(".margin-column .margin-thread .ls-block.comment-card");
    return !!thread && thread.textContent.includes(comment)
      && !document.querySelector(".page-blocks .ls-block.authored button.margin-marker")
      && !document.querySelector("textarea.block-editor");
  }, COMMENT), { timeout: 5_000, timeoutMsg: "the comment is not drawn in the margin without a duplicate badge" });
  const geometry = await browser.execute(() => {
    const thread = document.querySelector(".margin-thread").getBoundingClientRect();
    const outline = document.querySelector(".page-blocks").getBoundingClientRect();
    const mark = document.querySelector(".page-blocks .comment-quote-anchor")?.getBoundingClientRect();
    return { threadLeft: thread.left, threadTop: thread.top, outlineRight: outline.right, markTop: mark?.top ?? null };
  });
  if (!(geometry.threadLeft >= geometry.outlineRight)) throw new Error(`the thread overlaps the outline: ${JSON.stringify(geometry)}`);
  if (geometry.markTop === null || Math.abs(geometry.threadTop - geometry.markTop) > 2) throw new Error(`the thread is not beside its passage: ${JSON.stringify(geometry)}`);

  // Click the comment in the margin: it edits in place. Finish editing, then
  // send a reply from its composer (plain Enter now inserts a line break).
  await (await browser.$(".margin-column .margin-thread .ls-block.comment-card > .block-main .block-content")).click();
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".margin-column textarea.block-editor")), {
    timeout: 5_000, timeoutMsg: "clicking the margin comment did not open its editor there",
  });
  await browser.execute((comment) => {
    const ta = document.querySelector(".margin-column textarea.block-editor");
    ta.focus();
    ta.setSelectionRange(comment.length, comment.length);
  }, COMMENT);
  await browser.keys(["Control", "Enter"]);
  await browser.$(".margin-column .comment-card").moveTo();
  await browser.$(".margin-column .comment-reply-draft").click();
  await browser.keys(REPLY);
  await browser.keys(["Control", "Enter"]);

  // On disk: block > comment (tine.quote::) > reply.
  const saved = await waitForFileText(PAGE_FILE, (text) => text.includes(REPLY) && text.includes(COMMENT), { timeoutMs: 20_000 });
  const lines = saved.split("\n");
  const indent = (line) => line.length - line.trimStart().length;
  const agentLine = lines.findIndex((line) => line.startsWith(`- An agent wrote ${PHRASE}`));
  const commentLine = lines.findIndex((line) => line.trimStart() === `- ${COMMENT}`);
  const replyLine = lines.findIndex((line) => line.trimStart() === `- ${REPLY}`);
  if (agentLine < 0 || commentLine < 0 || replyLine < 0) throw new Error(`missing bullets in ${JSON.stringify(saved)}`);
  if (!(agentLine < commentLine && commentLine < replyLine)) throw new Error(`bullets out of order: ${JSON.stringify(saved)}`);
  if (!(indent(lines[commentLine]) > indent(lines[agentLine]) && indent(lines[replyLine]) > indent(lines[commentLine]))) {
    throw new Error(`the reply is not nested under the comment under the block: ${JSON.stringify(saved)}`);
  }
  if (lines[commentLine + 1]?.trim() !== `tine.quote:: ${PHRASE}`) throw new Error(`the comment lost its quote: ${JSON.stringify(saved)}`);
  if (!saved.includes(`- An agent wrote ${PHRASE} in this paragraph.\n  tine.author:: claude\n`)) {
    throw new Error(`the commented block changed on disk: ${JSON.stringify(saved)}`);
  }
  // The reply is drawn in the same margin thread.
  await browser.waitUntil(() => browser.execute((reply) =>
    [...document.querySelectorAll(".margin-column .margin-thread")].some((t) => t.textContent.includes(reply))
      && ![...document.querySelectorAll(".page-blocks .ls-block")].some((b) => b.textContent.includes(reply)), REPLY), {
    timeout: 5_000, timeoutMsg: "the reply is not drawn in the margin thread",
  });
  // The thread draws no bullets; its author chips keep the block menu, while
  // the commented body block keeps its bullet and drag handle (DESIGN §1).
  const bullets = await browser.execute(() => ({
    inThread: document.querySelectorAll(".margin-thread .bullet-container, .margin-thread .collapse-toggle").length,
    handles: document.querySelectorAll(".margin-thread .comment-handle").length,
    outline: document.querySelectorAll(".page-blocks .ls-block > .block-main > .block-controls > .bullet-container").length,
  }));
  if (bullets.inThread !== 0 || bullets.handles < 2 || bullets.outline < 1) throw new Error(`thread bullets: ${JSON.stringify(bullets)}`);
  // GH #845: hide the margin with the default shortcut (t then m, no editor focused).
  const drawn = () => browser.execute((comment, reply) => ({
    column: !!document.querySelector(".margin-column"),
    threads: document.querySelectorAll(".margin-thread").length,
    cards: document.querySelectorAll(".ls-block.comment-card").length,
    markers: document.querySelectorAll(".margin-marker").length,
    marks: document.querySelectorAll(".comment-quote-anchor").length,
    text: (document.querySelector(".page-blocks")?.textContent ?? "").includes(comment) || (document.querySelector(".page-blocks")?.textContent ?? "").includes(reply)
      || [...document.querySelectorAll(".margin-column")].some((el) => el.textContent.includes(comment)),
  }), COMMENT, REPLY);
  const nothingDrawn = (f) => !f.column && f.threads === 0 && f.cards === 0 && f.markers === 0 && f.marks === 0 && !f.text;
  const pressTM = async () => { await browser.keys(["t"]); await browser.keys(["m"]); };
  await pressTM();
  await browser.waitUntil(async () => nothingDrawn(await drawn()), { timeout: 5_000, timeoutMsg: "t m did not hide every comment thread" });
  if (fs.readFileSync(PAGE_FILE, "utf8") !== saved) throw new Error("hiding the margin changed the page file");
  // The choice is a device preference: it survives a reload.
  await browser.refresh();
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  await openPageByName(browser, PAGE_NAME);
  await (await browser.$(`.page-blocks .ls-block.authored`)).waitForExist({ timeout: 10_000 });
  await browser.pause(800);
  if (!nothingDrawn(await drawn())) throw new Error("the margin came back after a reload although it was hidden");
  await pressTM();
  await browser.waitUntil(() => browser.execute((comment, reply) => {
    const threads = [...document.querySelectorAll(".margin-column .margin-thread")];
    return threads.some((t) => t.textContent.includes(comment)) && threads.some((t) => t.textContent.includes(reply))
      && !document.querySelector(".page-blocks .ls-block.authored button.margin-marker");
  }, COMMENT, REPLY), { timeout: 5_000, timeoutMsg: "t m did not show the comment threads again" });
  if (fs.readFileSync(PAGE_FILE, "utf8") !== saved) throw new Error("showing the margin changed the page file");
  console.log("PASS: on a wide window a Ctrl+R comment is drawn and edited in the margin; its reply nests under it on disk; t m hides and shows every thread, across a reload, without touching the file");
  }
  await checkCommentStructure(browser, `${GRAPH}/pages/${STRUCTURE_PAGE}.md`, TMP);
} catch (error) {
  try { if (browser) await saveJourneyScreenshot(browser, `${TMP}/failure.png`); } catch {}
  fs.writeFileSync(path.join(TMP, "failure.json"), JSON.stringify({
    journey: "scripts/e2e-margin-column.mjs",
    expected: "Comment creation, editing, guarded persistence, margin threads and body structure preserve whole comment threads",
    observed: String(error), classification: error.classification ?? "ambiguous",
    testedCommit: process.env.E2E_COMMIT ?? "see invocation log",
  }, null, 2) + "\n");
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
  account.close?.();
  account.verify();
}
