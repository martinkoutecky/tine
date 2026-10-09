// Real-app journey for margin dialogue slice 1 (vision 2026-10 §3.7): on a
// routed named page, select words in a block, press Ctrl+R, type a reply, and
// the page file on disk holds a child bullet with the reply and `quote::`. The
// app must not reload on Ctrl+R (the webview's default for that chord). The
// window is narrow, so the comment renders inline (the margin column is
// scripts/e2e-margin-column.mjs).
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
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4518);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4519);
const TMP = "/tmp/tine-margin-comment-e2e";
const GRAPH = `${TMP}/graph`;
const PAGE_NAME = "Margin Test";
const PAGE_FILE = `${GRAPH}/pages/${PAGE_NAME}.md`;
const PHRASE = "a phrase worth disputing";
// No doubled letters: WebDriver key actions drop a repeated key ("ee" types "e").
const REPLY = "I doubt this claim";

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
  // A window too narrow for the margin column (slice 2): comments stay inline.
  await browser.setWindowSize(1000, 820);
  await openPageByName(browser, PAGE_NAME);

  // The agent block renders with its author chip, not an `author` row.
  const agent = await browser.$(`.page-blocks .ls-block.authored`);
  await agent.waitForExist({ timeout: 10_000 });
  await (await agent.$(".block-content")).click();
  const editor = await browser.$(".page-blocks textarea.block-editor");
  await editor.waitForExist({ timeout: 5_000 });

  // A marker that a webview reload would erase.
  await browser.execute(() => { window.__marginE2E = "alive"; });
  // Select the phrase in the editor, as a mouse or Shift+arrow selection would.
  const selected = await browser.execute((phrase) => {
    const ta = document.querySelector(".page-blocks textarea.block-editor");
    const at = ta.value.indexOf(phrase);
    ta.focus();
    ta.setSelectionRange(at, at + phrase.length);
    return ta.value.slice(ta.selectionStart, ta.selectionEnd);
  }, PHRASE);
  if (selected !== PHRASE) throw new Error(`selection was ${JSON.stringify(selected)}`);
  await browser.keys(["Control", "r"]);

  // The caret moves into the new comment's empty body line.
  await browser.waitUntil(() => browser.execute((phrase) => {
    const ta = document.querySelector(".page-blocks textarea.block-editor");
    return !!ta && ta.value.includes(`quote:: ${phrase}`) && ta.selectionStart === 0 && document.activeElement === ta;
  }, PHRASE), { timeout: 5_000, timeoutMsg: "Ctrl+R did not open an editor on a new comment quoting the selection" });
  if ((await browser.execute(() => window.__marginE2E)) !== "alive") throw new Error("Ctrl+R reloaded the webview");

  await browser.keys(REPLY);
  await browser.keys(["Escape"]);

  // On disk: the reply is a child of the agent block and carries quote::.
  const saved = await waitForFileText(PAGE_FILE, (text) => text.includes(REPLY) && text.includes(`quote:: ${PHRASE}`), { timeoutMs: 20_000 });
  const lines = saved.split("\n");
  const replyLine = lines.findIndex((line) => line.trimStart() === `- ${REPLY}`);
  if (replyLine < 0) throw new Error(`no reply bullet in ${JSON.stringify(saved)}`);
  const indent = (line) => line.length - line.trimStart().length;
  if (indent(lines[replyLine]) === 0) throw new Error(`reply is not a child bullet: ${JSON.stringify(saved)}`);
  if (lines[replyLine + 1]?.trim() !== `quote:: ${PHRASE}`) throw new Error(`quote:: does not follow the reply: ${JSON.stringify(saved)}`);
  const agentLine = lines.findIndex((line) => line.startsWith(`- An agent wrote ${PHRASE}`));
  const secondLine = lines.findIndex((line) => line.startsWith("- A second block"));
  if (!(agentLine < replyLine && replyLine < secondLine)) throw new Error(`reply is not under the agent block: ${JSON.stringify(saved)}`);
  if (!saved.includes(`- An agent wrote ${PHRASE} in this paragraph.\n  author:: claude\n`)) {
    throw new Error(`the commented block changed on disk: ${JSON.stringify(saved)}`);
  }

  // Rendered: a comment card with the quote, and the phrase marked in the parent.
  await browser.waitUntil(() => browser.execute((phrase) =>
    document.querySelector(".page-blocks .ls-block.comment-card .comment-quote")?.textContent === phrase
      && document.querySelector(".page-blocks .comment-quote-anchor")?.textContent === phrase, PHRASE), {
    timeout: 5_000, timeoutMsg: "the comment did not render as a card with its quote and a marked passage",
  });
  console.log("PASS: Ctrl+R on a selection saves a child comment with quote:: and renders it as a card");
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
