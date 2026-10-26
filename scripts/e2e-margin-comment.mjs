// Real-app journey for margin dialogue slice 1 (vision 2026-10 §3.7): on a
// routed named page, select words in a block, click Comment, type a reply, and
// the page file on disk holds a child bullet with the reply and `quote::`. The
// app must not reload on Ctrl+R (the webview's default for that chord). The
// window is narrow, so the comment renders inline (the margin column is
// scripts/e2e-margin-column.mjs). The user sets "Your name" in Settings first, so
// the comment and the reply written with Enter both carry `author:: <name>`.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { APP_ID } from "./lib/app-identity.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { marginFirstLineProblems } from "./lib/margin-first-line.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4518);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4519);
const TMP = process.env.E2E_MARGIN_TMP || "/tmp/tine-margin-comment-e2e";
const GRAPH = `${TMP}/graph`;
const PAGE_NAME = "Margin Test";
const PAGE_FILE = `${GRAPH}/pages/${PAGE_NAME}.md`;
const PHRASE = "a phrase worth disputing";
// No doubled letters: WebDriver key actions drop a repeated key ("ee" types "e").
const REPLY = "I doubt this claim";
const REPLY_2 = "A reply from me";
const MY_NAME = "Ada Test";
const SETTINGS = `${TMP}/xdg/data/${APP_ID}/tine-settings.json`;

if (process.env.E2E_MARGIN_TMP) {
  if (fs.existsSync(TMP)) throw new Error("E2E_MARGIN_TMP must be a fresh fixture directory");
} else fs.rmSync(TMP, { recursive: true, force: true });
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

// Settings -> Editor -> "Your name", saved on Enter through the device settings.
async function setMyName(browser) {
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForExist({ timeout: 5_000 });
  await browser.$("//button[contains(concat(' ', normalize-space(@class), ' '), ' settings-nav-item ') and normalize-space(.)='Editor']").click();
  const input = await browser.$('input[aria-label="Your name"]');
  await input.waitForExist({ timeout: 5_000 });
  await input.setValue(MY_NAME);
  await browser.keys(["Enter"]);
  await browser.waitUntil(() => fs.existsSync(SETTINGS) && JSON.parse(fs.readFileSync(SETTINGS, "utf8")).comment_author_name === MY_NAME, {
    timeout: 5_000, timeoutMsg: "Settings did not persist the name",
  });
  await browser.keys(["Escape"]);
  await browser.$(".settings-modal").waitForExist({ timeout: 5_000, reverse: true });
}

let browser;
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  console.log("app bundle:", await browser.execute(() => document.querySelector('link[rel="modulepreload"][href*="app-"]')?.getAttribute("href")));
  // A window too narrow for the margin column (slice 2): comments stay inline.
  await browser.setWindowSize(1000, 820);
  await setMyName(browser);
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
  await browser.execute(() => document.querySelector("textarea.block-editor").dispatchEvent(new Event("select", { bubbles: true })));
  const toolbar = await browser.execute(() => {
    const button = document.querySelector('[data-selection-action="comment"]');
    return !!button?.querySelector("svg") && button === button.closest(".sel-toolbar").querySelector(":scope > button:last-of-type")
      && button.title.includes("Ctrl/Cmd+R");
  });
  if (!toolbar) throw new Error("Comment must be the rightmost toolbar icon with its shortcut");
  await browser.$('[data-selection-action="comment"]').click();

  // The caret moves into the new comment's empty body line.
  await browser.waitUntil(() => browser.execute(() => {
    const ta = document.querySelector(".page-blocks textarea.block-editor");
    return !!ta && ta.closest(".comment-in-thread") && ta.value === "" && ta.selectionStart === 0 && document.activeElement === ta;
  }), { timeout: 5_000, timeoutMsg: "Ctrl+R did not open an empty body editor on the new comment" });
  if ((await browser.execute(() => window.__marginE2E)) !== "alive") throw new Error("Ctrl+R reloaded the webview");

  await browser.keys(REPLY);
  // Enter in the comment's text starts a reply as its last child.
  await browser.keys(["Enter"]);
  await browser.keys(REPLY_2);
  await browser.keys(["Escape"]);

  // On disk: the reply is a child of the agent block and carries quote::.
  const saved = await waitForFileText(PAGE_FILE, (text) => text.includes(REPLY_2) && text.includes(`quote:: ${PHRASE}`), { timeoutMs: 20_000 });
  const lines = saved.split("\n");
  const replyLine = lines.findIndex((line) => line.trimStart() === `- ${REPLY}`);
  if (replyLine < 0) throw new Error(`no reply bullet in ${JSON.stringify(saved)}`);
  const indent = (line) => line.length - line.trimStart().length;
  if (indent(lines[replyLine]) === 0) throw new Error(`reply is not a child bullet: ${JSON.stringify(saved)}`);
  if (lines[replyLine + 1]?.trim() !== `quote:: ${PHRASE}`) throw new Error(`quote:: does not follow the reply: ${JSON.stringify(saved)}`);
  // The comment's own property lines: quote::, its context (ADR 0075, MC2), then author::.
  const props = [];
  for (let i = replyLine + 1; i < lines.length && !lines[i].trimStart().startsWith("- "); i++) props.push(lines[i].trim());
  if (!props.some((line) => line.startsWith("quote-prefix:: "))) throw new Error(`the comment carries no quote-prefix:: (MC2): ${JSON.stringify(saved)}`);
  // Your name: the comment carries it, and so does the reply written with Enter.
  if (props[props.length - 1] !== `author:: ${MY_NAME}`) throw new Error(`the comment does not carry author:: ${MY_NAME}: ${JSON.stringify(saved)}`);
  const secondReply = lines.findIndex((line) => line.trimStart() === `- ${REPLY_2}`);
  if (secondReply < 0 || indent(lines[secondReply]) <= indent(lines[replyLine]) || lines[secondReply + 1]?.trim() !== `author:: ${MY_NAME}`) {
    throw new Error(`the reply does not nest under the comment with author:: ${MY_NAME}: ${JSON.stringify(saved)}`);
  }
  const agentLine = lines.findIndex((line) => line.startsWith(`- An agent wrote ${PHRASE}`));
  const secondLine = lines.findIndex((line) => line.startsWith("- A second block"));
  if (!(agentLine < replyLine && replyLine < secondLine)) throw new Error(`reply is not under the agent block: ${JSON.stringify(saved)}`);
  if (!saved.includes(`- An agent wrote ${PHRASE} in this paragraph.\n  author:: claude\n`)) {
    throw new Error(`the commented block changed on disk: ${JSON.stringify(saved)}`);
  }

  // Rendered: a comment card with the quote, and the phrase marked in the parent.
  await browser.waitUntil(() => browser.execute((phrase) =>
    document.querySelector(".page-blocks .ls-block.comment-card .comment-quote-text")?.textContent === phrase
      && document.querySelector(".page-blocks .comment-quote-anchor")?.textContent === phrase, PHRASE), {
    timeout: 5_000, timeoutMsg: "the comment did not render as a card with its quote and a marked passage",
  });
  // Your own name renders as "you", not as an agent.
  const chips = await browser.execute(() => [...document.querySelectorAll(".page-blocks .ls-block.comment-in-thread .author-chip")]
    .map((el) => ({ text: el.textContent, agent: el.classList.contains("agent") })));
  if (!chips.length || chips.some((chip) => chip.text !== "you" || chip.agent)) throw new Error(`own comments are not shown as you: ${JSON.stringify(chips)}`);
  const beforeDraft = fs.statSync(PAGE_FILE).mtimeMs;
  await browser.execute(() => document.querySelector(".comment-card").dispatchEvent(new MouseEvent("mouseenter")));
  // Focus via the native pointer after hovering the inline thread.
  await browser.$(".comment-card").moveTo();
  const draft = await browser.$(".comment-card > .comment-composer textarea");
  await draft.click();
  if (await draft.getAttribute("rows") !== "3") throw new Error("reply draft did not expand");
  await browser.keys(["Control", "r"]);
  if (await browser.execute(() => window.__marginE2E) !== "alive") throw new Error("Ctrl+R in an empty draft reloaded the app");
  await browser.execute(() => document.activeElement.blur());
  if (await browser.$(".comment-composer.expanded").isExisting()) throw new Error("empty reply did not collapse on blur");
  await draft.click();
  await browser.keys(["Escape"]);
  // Moving focus and cancelling again must not insert a block or save the page.
  await browser.$(".comment-card").moveTo();
  await draft.click();
  await browser.$(".comment-composer-footer .dialog-actions-main button:first-child").click();
  if (fs.readFileSync(PAGE_FILE, "utf8") !== saved || fs.statSync(PAGE_FILE).mtimeMs !== beforeDraft) {
    throw new Error("abandoned empty reply changed or saved the page");
  }
  await browser.$(".comment-card").moveTo();
  await draft.click();
  await browser.keys("A draft reply");
  await browser.keys(["Enter"]);
  const drafted = await waitForFileText(PAGE_FILE, text => text.includes("A draft reply"), { timeoutMs: 20_000 });
  const draftLines = drafted.split("\n");
  const draftAt = draftLines.findIndex(line => line.trim() === "- A draft reply");
  if (draftAt < 0 || draftLines[draftAt + 1]?.trim() !== `author:: ${MY_NAME}`) throw new Error("draft submission lost the configured author");
  await browser.setWindowSize(1600, 1000);
  await browser.$(".margin-thread .comment-card").waitForExist({ timeout: 5000 });
  const content = await browser.$(".comment-card > .block-main .block-content");
  await content.click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  await browser.execute(() => {
    const ta = document.querySelector("textarea.block-editor");
    ta.focus(); ta.setSelectionRange(0, 1);
  });
  await browser.keys(["Control", "r"]);
  await browser.waitUntil(() => browser.execute(() => document.activeElement?.matches("textarea.block-editor") && document.activeElement.value === ""), { timeout: 5000 });
  await browser.keys("A quoted child");
  await browser.keys(["Escape"]);
  await waitForFileText(PAGE_FILE, text => text.includes("A quoted child") && text.includes("quote:: I"), { timeoutMs: 20_000 });
  if ((await browser.$$(".margin-thread")).length !== 1) throw new Error("Ctrl+R in a comment created a separate thread");
  if (await browser.execute(() => window.__marginE2E) !== "alive") throw new Error("Ctrl+R in a comment reloaded the app");
  await browser.$(".comment-card").moveTo();
  const wideDraft = await browser.$(".comment-card > .comment-composer textarea");
  await wideDraft.click();
  await browser.keys("A draft worth thought");
  await browser.execute(() => document.querySelector(".comment-reply-draft").setSelectionRange(2, 7));
  await browser.keys(["Control", "r"]);
  await browser.waitUntil(() => browser.execute(() => document.activeElement?.matches("textarea.block-editor") && document.activeElement.value === ""), { timeout: 5000 });
  await browser.keys("A nested draft remark");
  await browser.keys(["Escape"]);
  await waitForFileText(PAGE_FILE, text => text.includes("A draft worth thought") && text.includes("A nested draft remark") && text.includes("quote:: draft"), { timeoutMs: 20_000 });
  if (await browser.execute(() => window.__marginE2E) !== "alive") throw new Error("Ctrl+R in a nonempty draft reloaded the app");
  const problems = await browser.execute(marginFirstLineProblems);
  if (problems.length) throw new Error(problems.join("; "));
  console.log("PASS: rightmost Comment icon, Enter reply, empty draft cancellation without a save, draft submission with author, top delete, and Ctrl+R quoted reply in the wide thread");
} catch (error) {
  console.error(JSON.stringify({ base: "c39d28a37 + margin-r4 working diff", journey: "margin-comment", expected: "local empty draft, ordinary submitted reply, delete on first content line, rightmost icon", observed: String(error), classification: "ambiguous; inspect log and screenshot", screenshot: `${TMP}/failure.png` }));
  // External capture avoids WebKitGTK's occasionally hanging screenshot endpoint.
  const { execFileSync } = await import("node:child_process");
  try { execFileSync("import", ["-window", "root", `${TMP}/failure.png`]); } catch {}
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
