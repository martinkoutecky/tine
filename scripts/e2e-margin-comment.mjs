// Real-app journey for margin dialogue slice 1 (vision 2026-10 §3.7): on a
// routed named page, select words in a block, click Comment, type a reply, and
// the page file on disk holds a child bullet with the reply and `tine.quote::`. The
// app must not reload on Ctrl+R (the webview's default for that chord). The
// window is narrow, so the comment opens in a popover (the margin column is
// scripts/e2e-margin-column.mjs). The graph is under git and no name is set, so
// the first comment asks for "Your name" once (tine#841); the name given there
// is a device setting, shown in Settings, and the comment and the reply written
// with the composer both carry `tine.author:: <name>`.
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { APP_ID } from "./lib/app-identity.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { marginFirstLineProblems } from "./lib/margin-first-line.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { seedJournalMargin, checkJournalMargin } from "./lib/e2e-journal-margin.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4518);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4519);
const TMP = process.env.E2E_MARGIN_TMP || "/tmp/tine-margin-comment-e2e";
const OUT = process.env.E2E_ARTIFACT_DIR || TMP;
const GRAPH = `${TMP}/graph`;
const PAGE_NAME = "Margin Test";
const PAGE_FILE = `${GRAPH}/pages/${PAGE_NAME}.md`;
const PHRASE = "a phrase worth disputing";
// No doubled letters: WebDriver key actions drop a repeated key ("ee" types "e").
const REPLY = "I doubt this claim";
const REPLY_2 = "A reply from me";
const MY_NAME = "Ada Test";
const SETTINGS = `${TMP}/xdg/data/${APP_ID}/tine-settings.json`;
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
const proof = { testedCommit, delta: "working tree", app: APP, steps: [], screenshots: [] };

if (process.env.E2E_MARGIN_TMP) {
  if (fs.existsSync(TMP)) throw new Error("E2E_MARGIN_TMP must be a fresh fixture directory");
} else fs.rmSync(TMP, { recursive: true, force: true });
fs.mkdirSync(OUT, { recursive: true });
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache", "state"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(`${TMP}/home`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
// A graph under git is the one that asks for a name (tine#841); autocommit stays off.
execFileSync("git", ["-C", GRAPH, "init", "-q", "-b", "main"]);
fs.writeFileSync(PAGE_FILE, `- An agent wrote ${PHRASE} in this paragraph.\n  tine.author:: claude\n- A second block stays as it is.\n- Authored note\n  tine.author:: Ada Marks  \t\n- A following sentinel\n- > A quoted passage\n  tine.author:: Quote Agent  \t\n`);
const journalFixture = seedJournalMargin(GRAPH, PAGE_NAME);
const account = installAccountHome(TMP, OUT);

const env = {
  ...process.env,
  TINE_GRAPH: GRAPH,
  HOME: `${TMP}/home`, XDG_STATE_HOME: `${TMP}/xdg/state`,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
  TINE_E2E_TOUCH_GESTURES: "ios",
};
const log = fs.openSync(`${OUT}/tauri-driver.log`, "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

// Settings -> Editor -> "Your name" shows the name the first-comment dialog saved.
async function settingsNameField(browser) {
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForExist({ timeout: 5_000 });
  await browser.$("//button[contains(concat(' ', normalize-space(@class), ' '), ' settings-nav-item ') and normalize-space(.)='Editor']").click();
  const input = await browser.$('input[aria-label="Your name"]');
  await input.waitForExist({ timeout: 5_000 });
  const value = await input.getValue();
  await browser.keys(["Escape"]);
  await browser.$(".settings-modal").waitForExist({ timeout: 5_000, reverse: true });
  return value;
}
const deviceSettings = () => (fs.existsSync(SETTINGS) ? JSON.parse(fs.readFileSync(SETTINGS, "utf8")) : {});

let browser;
async function shot(name) {
  await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
  execFileSync("import", ["-window", "root", `${OUT}/${name}.png`]);
  proof.screenshots.push(`${name}.png`);
}
async function blockWith(text) {
  const id = await browser.execute(text => [...document.querySelectorAll(".page-blocks .ls-block")]
    .find(row => row.querySelector(":scope > .block-main .block-content")?.textContent.includes(text))?.dataset.blockId, text);
  if (!id) throw new Error(`No block containing ${text}`);
  return browser.$(`[data-block-id="${id}"]`);
}
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await ensureMainWindow(browser);
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  await assertAppLease(account);
  console.log("app bundle:", await browser.execute(() => document.querySelector('link[rel="modulepreload"][href*="app-"]')?.getAttribute("href")));
  // A window too narrow for the margin column: comments open in a live popover.
  await browser.setWindowSize(1000, 820);
  await openPageByName(browser, PAGE_NAME);

  // GH #894/#895: a real edit keeps hidden author bytes, author chrome and the
  // following row's position. Opening a legacy quote never rewrites it.
  const initialBytes = fs.readFileSync(PAGE_FILE, "utf8");
  const quote = await blockWith("A quoted passage");
  if ((await quote.$(".md-quote").getText()).includes("tine.author")) throw new Error("Author metadata rendered inside the quote");
  if (await quote.$(".author-chip").getText() !== "Quote Agent") throw new Error("Quote lost its author mark");
  await shot("00-author-and-quote");
  if (fs.readFileSync(PAGE_FILE, "utf8") !== initialBytes) throw new Error("Viewing the quote rewrote its bytes");
  const authored = await blockWith("Authored note");
  const following = await blockWith("A following sentinel");
  const before = (await following.getLocation()).y;
  await authored.$(".block-content").click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  if (await browser.$("textarea.block-editor").getValue() !== "Authored note") throw new Error("Ordinary author metadata is visible in the editor");
  if (!await authored.$(".author-chip").isDisplayed()) throw new Error("Author chip disappeared during editing");
  const after = (await following.getLocation()).y;
  if (Math.abs(after - before) > 1) throw new Error(`Opening the authored block moved the following row: ${before} -> ${after}`);
  proof.steps.push({ outcome: "author hidden; mark and chip visible; following row stable", before, after });
  await shot("01-hidden-author-editor");
  await browser.execute(() => { const ta = document.querySelector("textarea.block-editor"); ta.setSelectionRange(ta.value.length, ta.value.length); });
  await browser.keys(" revised");
  await browser.keys(["Escape"]);
  await waitForFileText(PAGE_FILE, text => text.includes("Authored note revised\n  tine.author:: Ada Marks  \t\n"), { timeoutMs: 20_000 });
  await quote.$(".block-content").click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  if (await browser.$("textarea.block-editor").getValue() !== "> A quoted passage") throw new Error("Quote editor exposes its author property");
  await browser.execute(() => { const ta = document.querySelector("textarea.block-editor"); const at = ta.value.indexOf("quoted"); ta.setSelectionRange(at, at + 6); });
  await browser.keys("revised"); await browser.keys(["Escape"]);
  await waitForFileText(PAGE_FILE, text => text.includes("> A revised passage") && text.includes("tine.author:: Quote Agent  \t"), { timeoutMs: 20_000 });
  await browser.refresh(); await ensureMainWindow(browser); await openPageByName(browser, PAGE_NAME);
  const reopenedQuote = await blockWith("A revised passage");
  if ((await reopenedQuote.$(".md-quote").getText()).includes("tine.author") || await reopenedQuote.$(".author-chip").getText() !== "Quote Agent") throw new Error("Saved quote lost its text/author separation after reload");
  proof.steps.push("quote author never rendered as prose; retained author bytes survive an edit and reload");
  const reopened = await blockWith("Authored note revised");
  await reopened.$(".block-content").click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  await browser.execute(() => { const ta = document.querySelector("textarea.block-editor"); ta.setSelectionRange(ta.value.length, ta.value.length); });
  await browser.keys(" pending");
  await reopened.$(".author-chip").click();
  await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".ctx-item")].some(b => b.textContent.trim() === "Remove author")), { timeout: 5000 });
  await shot("02-remove-author-menu");
  await browser.execute(() => [...document.querySelectorAll(".ctx-item")].find(b => b.textContent.trim() === "Remove author").click());
  await waitForFileText(PAGE_FILE, text => text.includes("Authored note revised pending") && !text.includes("tine.author:: Ada Marks"), { timeoutMs: 20_000 });
  await shot("03-author-removed");
  await browser.keys(["Control", "z"]);
  await waitForFileText(PAGE_FILE, text => text.includes("Authored note revised pending\n  tine.author:: Ada Marks  \t"), { timeoutMs: 20_000 });
  proof.steps.push("chip menu removes only author, keeps pending draft; undo restores exact author bytes");

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

  // tine#841: a git graph with no name asks once, before the comment is written. It says
  // the name is a Tine setting of this device, used in every graph, and writes nothing yet.
  const prompt = await browser.$('[role="dialog"].comment-name-prompt');
  await prompt.waitForExist({ timeout: 5_000, timeoutMsg: "the first comment in a git graph did not ask for a name" });
  const promptText = await prompt.getText();
  if (!promptText.includes("How should your comments be signed?") || /git|github/i.test(promptText)) throw new Error(`Name prompt wording: ${promptText}`);
  await shot("04-comment-name-question");
  if (!/Tine setting on this device, used in all your graphs, not a setting of this graph/.test(promptText)) throw new Error(`the name dialog does not say it is a device setting: ${JSON.stringify(promptText)}`);
  if (fs.readFileSync(PAGE_FILE, "utf8").includes("tine.quote::")) throw new Error("the comment was written before the name was answered");
  await browser.keys(MY_NAME);
  await browser.keys(["Enter"]);
  await prompt.waitForExist({ timeout: 5_000, reverse: true });
  await browser.waitUntil(() => deviceSettings().comment_author_name === MY_NAME, { timeout: 5_000, timeoutMsg: "the name was not saved to the device settings" });
  if (deviceSettings().comment_name_prompt_declined) throw new Error("saving a name recorded a decline");

  // The caret moves into the new comment's empty body line.
  await browser.waitUntil(() => browser.execute(() => {
    const ta = document.querySelector(".comment-thread-popup textarea.block-editor");
    return !!ta && ta.closest(".comment-in-thread") && ta.value === "" && ta.selectionStart === 0 && document.activeElement === ta;
  }), { timeout: 5_000, timeoutMsg: "Ctrl+R did not open an empty body editor on the new comment" });
  if ((await browser.execute(() => window.__marginE2E)) !== "alive") throw new Error("Ctrl+R reloaded the webview");

  await browser.keys(REPLY);
  await browser.keys(["Escape"]);
  await browser.$(".comment-card").moveTo();
  await browser.$(".comment-reply-draft").click();
  await browser.keys(REPLY_2);
  await browser.keys(["Control", "Enter"]);

  // On disk: the reply is a child of the agent block and carries tine.quote::.
  const saved = await waitForFileText(PAGE_FILE, (text) => text.includes(REPLY_2) && text.includes(`tine.quote:: ${PHRASE}`), { timeoutMs: 20_000 });
  const lines = saved.split("\n");
  const replyLine = lines.findIndex((line) => line.trimStart() === `- ${REPLY}`);
  if (replyLine < 0) throw new Error(`no reply bullet in ${JSON.stringify(saved)}`);
  const indent = (line) => line.length - line.trimStart().length;
  if (indent(lines[replyLine]) === 0) throw new Error(`reply is not a child bullet: ${JSON.stringify(saved)}`);
  if (lines[replyLine + 1]?.trim() !== `tine.quote:: ${PHRASE}`) throw new Error(`tine.quote:: does not follow the reply: ${JSON.stringify(saved)}`);
  // The comment's quote/context, author and immutable creation date.
  const props = [];
  for (let i = replyLine + 1; i < lines.length && !lines[i].trimStart().startsWith("- "); i++) props.push(lines[i].trim());
  if (!props.some((line) => line.startsWith("tine.quote-prefix:: "))) throw new Error(`the comment carries no tine.quote-prefix:: (MC2): ${JSON.stringify(saved)}`);
  // Your name: the comment carries it, and so does the reply from the composer.
  if (props[props.length - 2] !== `tine.author:: ${MY_NAME}` || !/^tine.date:: \d{4}-\d\d-\d\dT\d\d:\d\dZ$/.test(props[props.length - 1])) throw new Error(`the comment does not carry its author and creation date: ${JSON.stringify(saved)}`);
  const secondReply = lines.findIndex((line) => line.trimStart() === `- ${REPLY_2}`);
  if (secondReply < 0 || indent(lines[secondReply]) <= indent(lines[replyLine]) || lines[secondReply + 1]?.trim() !== `tine.author:: ${MY_NAME}`) {
    throw new Error(`the reply does not nest under the comment with tine.author:: ${MY_NAME}: ${JSON.stringify(saved)}`);
  }
  const agentLine = lines.findIndex((line) => line.startsWith(`- An agent wrote ${PHRASE}`));
  const secondLine = lines.findIndex((line) => line.startsWith("- A second block"));
  if (!(agentLine < replyLine && replyLine < secondLine)) throw new Error(`reply is not under the agent block: ${JSON.stringify(saved)}`);
  if (!saved.includes(`- An agent wrote ${PHRASE} in this paragraph.\n  tine.author:: claude\n`)) {
    throw new Error(`the commented block changed on disk: ${JSON.stringify(saved)}`);
  }

  // Rendered: a comment card with the quote, and the phrase marked in the parent.
  await browser.waitUntil(() => browser.execute((phrase) =>
    document.querySelector(".comment-thread-popup .ls-block.comment-card .comment-quote-text")?.textContent === phrase
      && document.querySelector(".page-blocks .comment-quote-anchor")?.textContent === phrase, PHRASE), {
    timeout: 5_000, timeoutMsg: "the comment did not render as a card with its quote and a marked passage",
  });
  // Your own name renders as "you", not as an agent.
  const chips = await browser.execute(() => [...document.querySelectorAll(".comment-thread-popup .ls-block.comment-in-thread .author-chip")]
    .map((el) => ({ text: el.textContent, agent: el.classList.contains("agent") })));
  if (!chips.length || chips.some((chip) => chip.text !== "you" || chip.agent)) throw new Error(`own comments are not shown as you: ${JSON.stringify(chips)}`);
  const beforeDraft = fs.statSync(PAGE_FILE).mtimeMs;
  await browser.execute(() => document.querySelector(".comment-card").dispatchEvent(new MouseEvent("mouseenter")));
  // Focus via the native pointer after hovering the live thread.
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
  await browser.keys(["Control", "Enter"]);
  const drafted = await waitForFileText(PAGE_FILE, text => text.includes("A draft reply"), { timeoutMs: 20_000 });
  const draftLines = drafted.split("\n");
  const draftAt = draftLines.findIndex(line => line.trim() === "- A draft reply");
  if (draftAt < 0 || draftLines[draftAt + 1]?.trim() !== `tine.author:: ${MY_NAME}`) throw new Error("draft submission lost the configured author");
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
  await waitForFileText(PAGE_FILE, text => text.includes("A quoted child") && text.includes("tine.quote:: I"), { timeoutMs: 20_000 });
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
  await waitForFileText(PAGE_FILE, text => text.includes("A draft worth thought") && text.includes("A nested draft remark") && text.includes("tine.quote:: draft"), { timeoutMs: 20_000 });
  if (await browser.execute(() => window.__marginE2E) !== "alive") throw new Error("Ctrl+R in a nonempty draft reloaded the app");
  const problems = await browser.execute(marginFirstLineProblems);
  if (problems.length) throw new Error(problems.join("; "));
  // The name given in the dialog is the one device setting Settings shows, and later comments did not ask again.
  if (await settingsNameField(browser) !== MY_NAME) throw new Error("Settings > Your name does not show the name given in the dialog");
  proof.steps.push("first comment asks once using signing language without git/GitHub; name persists in device Settings");
  await shot("05-completed-margin-journey");
  await checkJournalMargin(browser, journalFixture, shot, proof);
  fs.writeFileSync(`${OUT}/proof.json`, JSON.stringify(proof, null, 2));
  console.log("PASS: signing prompt, author editing, comment/reply actions, per-day journal margins, synthetic iOS tap editing with exact retained metadata, width transitions and reload");
} catch (error) {
  const failure = { testedCommit, delta: "working tree", journey: "margin-comment, author editing and journal/iOS metadata", expected: "hidden retained metadata, quote separation, removable chip, signing prompt, per-day journal margins and durable comment edits", observed: String(error), classification: "ambiguous; inspect log and screenshot", screenshot: `${OUT}/failure.png` };
  console.error(JSON.stringify(failure));
  fs.writeFileSync(`${OUT}/failure.json`, JSON.stringify(failure, null, 2));
  // External capture avoids WebKitGTK's occasionally hanging screenshot endpoint.
  const { execFileSync } = await import("node:child_process");
  try { execFileSync("import", ["-window", "root", `${OUT}/failure.png`]); } catch {}
  throw error;
} finally {
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  fs.closeSync(log);
}
