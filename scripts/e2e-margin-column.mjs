// Real-app journey for margin dialogue slice 2 (vision 2026-10 §3.7 "Layout"):
// on a wide window, Ctrl+R on a selection creates a comment that is drawn in the
// right-hand margin column (not in the outline); clicking it edits it there,
// Enter adds a reply, and the page file on disk holds the reply nested under
// the comment, which is nested under the commented block.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4520);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4521);
const TMP = "/tmp/tine-margin-column-e2e";
const GRAPH = `${TMP}/graph`;
const PAGE_NAME = "Margin Column Test";
const PAGE_FILE = `${GRAPH}/pages/${PAGE_NAME}.md`;
const PHRASE = "a phrase worth disputing";
// No doubled letters: WebDriver key actions drop a repeated key ("ee" types "e").
const COMMENT = "Is this right";
const REPLY = "Yes, it holds";

fs.rmSync(TMP, { recursive: true, force: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(PAGE_FILE, `- An agent wrote ${PHRASE} in this paragraph.\n  author:: claude\n- A second block stays as it is.\n`);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, `- open [[${PAGE_NAME}]]\n`);

const env = {
  ...process.env,
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
  // Wide enough for the margin column with the left sidebar open.
  await browser.setWindowSize(1600, 1000);
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

  // The new comment opens for editing in the margin column, not in the outline.
  await browser.waitUntil(() => browser.execute((phrase) => {
    const ta = document.querySelector(".margin-column .margin-thread textarea.block-editor");
    return !!ta && ta.value.includes(`quote:: ${phrase}`) && document.activeElement === ta
      && !document.querySelector(".page-blocks .ls-block.comment-card");
  }, PHRASE), { timeout: 5_000, timeoutMsg: "Ctrl+R did not open the new comment in the margin column" });
  await browser.keys(COMMENT);
  await browser.keys(["Escape"]);
  await browser.keys(["Escape"]);

  // Drawn in the margin, beside a count on the commented block.
  await browser.waitUntil(() => browser.execute((comment) => {
    const thread = document.querySelector(".margin-column .margin-thread .ls-block.comment-card");
    return !!thread && thread.textContent.includes(comment)
      && document.querySelector(".page-blocks .ls-block.authored .margin-marker")?.textContent === "1"
      && !document.querySelector("textarea.block-editor");
  }, COMMENT), { timeout: 5_000, timeoutMsg: "the comment is not drawn in the margin with a count marker" });
  const geometry = await browser.execute(() => {
    const thread = document.querySelector(".margin-thread").getBoundingClientRect();
    const outline = document.querySelector(".page-blocks").getBoundingClientRect();
    const mark = document.querySelector(".page-blocks .comment-quote-anchor")?.getBoundingClientRect();
    return { threadLeft: thread.left, threadTop: thread.top, outlineRight: outline.right, markTop: mark?.top ?? null };
  });
  if (!(geometry.threadLeft >= geometry.outlineRight)) throw new Error(`the thread overlaps the outline: ${JSON.stringify(geometry)}`);
  if (geometry.markTop === null || Math.abs(geometry.threadTop - geometry.markTop) > 2) throw new Error(`the thread is not beside its passage: ${JSON.stringify(geometry)}`);

  // Click the comment in the margin: it edits in place. Enter at the end of its
  // text adds a reply.
  await (await browser.$(".margin-column .margin-thread .ls-block.comment-card > .block-main .block-content")).click();
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".margin-column textarea.block-editor")), {
    timeout: 5_000, timeoutMsg: "clicking the margin comment did not open its editor there",
  });
  await browser.execute((comment) => {
    const ta = document.querySelector(".margin-column textarea.block-editor");
    ta.focus();
    ta.setSelectionRange(comment.length, comment.length);
  }, COMMENT);
  await browser.keys(["Enter"]);
  await browser.waitUntil(() => browser.execute(() => {
    const ta = document.querySelector(".margin-column textarea.block-editor");
    return !!ta && ta.value === "" && document.activeElement === ta;
  }), { timeout: 5_000, timeoutMsg: "Enter in the margin comment did not open an empty reply in the thread" });
  await browser.keys(REPLY);
  await browser.keys(["Escape"]);

  // On disk: block > comment (quote::) > reply.
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
  if (lines[commentLine + 1]?.trim() !== `quote:: ${PHRASE}`) throw new Error(`the comment lost its quote: ${JSON.stringify(saved)}`);
  if (!saved.includes(`- An agent wrote ${PHRASE} in this paragraph.\n  author:: claude\n`)) {
    throw new Error(`the commented block changed on disk: ${JSON.stringify(saved)}`);
  }
  // The reply is drawn in the same margin thread.
  await browser.waitUntil(() => browser.execute((reply) =>
    [...document.querySelectorAll(".margin-column .margin-thread")].some((t) => t.textContent.includes(reply))
      && ![...document.querySelectorAll(".page-blocks .ls-block")].some((b) => b.textContent.includes(reply)), REPLY), {
    timeout: 5_000, timeoutMsg: "the reply is not drawn in the margin thread",
  });
  console.log("PASS: on a wide window a Ctrl+R comment is drawn and edited in the margin; its reply nests under it on disk");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
