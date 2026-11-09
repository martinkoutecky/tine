// og-wasm-enter-trap: in one app session, edit three page embeds' source pages through the embed
// (Enter at the end of the last embedded block, type, Tab) and check each edit lands once, as a
// child of that block, in the source file and the embed. Repeats E2E_SESSIONS fresh sessions and
// records how often a parser trap ("lsdoc-wasm trap") or a misplaced edit occurs: the 2026-11-07
// trap in outline_apply_json happened on the third page of such a session and never reproduced.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = path.join(root, process.env.E2E_OUT || "target/artifacts/og-wasm-enter-trap/native");
fs.mkdirSync(out, { recursive: true });
const sessions = Number(process.env.E2E_SESSIONS || 1);
const pages = Number(process.env.E2E_PAGES || 3);
const port = Number(process.env.E2E_DRIVER_PORT || 4974);
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();

/** The focused editor, whichever engine: the block it belongs to, its text and its selection. Every step
 *  below waits for the state it stands for, never for a fixed time. */
const editorState = (browser) => browser.execute(() => {
  const el = document.activeElement;
  const row = el?.closest?.(".ls-block");
  const where = { id: row?.dataset.blockId ?? null, inEmbed: !!row?.closest(".embed-block") };
  if (el instanceof HTMLTextAreaElement) return { ...where, text: el.value, start: el.selectionStart, end: el.selectionEnd };
  const view = el?.cmTile?.root?.view;
  if (view) { const s = view.state.selection.main; return { ...where, text: view.state.doc.toString(), start: s.from, end: s.to }; }
  return { ...where, text: null, start: null, end: null };
});
// A state that never arrives is the product failing the journey (an Enter that made no block), not the harness.
const expected = async (what, wait) => { try { return await wait(); } catch (error) { assert.fail(`never reached: ${what} (${String(error.message).slice(0, 200)})`); } };
const untilEditor = (browser, what, predicate) => expected(what, () => browser.waitUntil(async () => predicate(await editorState(browser)),
  { timeout: 15000, interval: 50 }));
const untilNoEditor = (browser, what) => expected(what, () => browser.waitUntil(() => browser.execute(() => !document.querySelector(".block-editor, [data-block-editor]")),
  { timeout: 15000, interval: 50 }));

/** A source page of ordinary, varied blocks: properties, ids, tasks, links, CJK and emoji text. */
function sourcePage(n) {
  const rows = [`tags:: Example\n`];
  for (let i = 0; i < 24; i++) {
    const id = `0f0e0000-0000-4000-8000-${String(n * 100 + i).padStart(12, "0")}`;
    const text = [`Plain note ${n}.${i}`, `TODO task ${n}.${i} [[Linked ${i % 4}]]`, `中文 ${n}.${i} 😀 **bold** \`code\``,
      `Quoted ((${id})) ref ${i}`, `Heading ${i}\n  heading:: true`][i % 5];
    rows.push(`- ${text}\n${i % 3 === 0 ? `  id:: ${id}\n` : ""}${i % 4 === 1 ? `  - child of ${n}.${i}\n` : ""}`);
  }
  rows.push(`- Last block of source ${n}\n`);
  return rows.join("");
}

async function session(index) {
  const run = fs.mkdtempSync(path.join(out, `session-${index}-`));
  const graph = path.join(run, "graph");
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
  for (let n = 1; n <= pages; n++) {
    fs.writeFileSync(path.join(graph, `pages/Source ${n}.md`), sourcePage(n));
    fs.writeFileSync(path.join(graph, `pages/Host ${n}.md`), `- Intro ${n}\n- {{embed [[Source ${n}]]}}\n- Host tail ${n}\n`);
  }
  fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
  for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(run, dir));
  const fixture = accountHomeFixture(run, run);
  const env = { ...fixture.env, HOME: path.join(run, "home"), TINE_GRAPH: graph,
    XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
    XDG_CACHE_HOME: path.join(run, "cache"), XDG_STATE_HOME: path.join(run, "state"),
    WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
  const log = fs.openSync(path.join(run, "driver.log"), "w");
  const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1),
    "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
  let browser, stage = "startup";
  const result = { session: index, run, edits: [] };
  try {
    browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
      capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
        "tauri:options": { application: process.env.TINE_APP || path.join(root, "target/release/tine") } } });
    await ensureMainWindow(browser);
    await browser.setWindowSize(1200, 1000);
    await browser.$(".app-container").waitForExist({ timeout: 30000 });
    await assertAppLease(fixture);
    for (let n = 1; n <= pages; n++) {
      stage = `page ${n}: open host`;
      await openPageByName(browser, `Host ${n}`);
      const embed = '.main-content .embed-block';
      await browser.$(`${embed} .ls-block:not(.ref-block)`).waitForExist({ timeout: 30000 });
      const last = await browser.execute((sel, label) => [...document.querySelectorAll(`${sel} .ls-block`)]
        .find(row => row.querySelector(":scope > .block-main .block-content")?.textContent.trim() === label)?.dataset.blockId,
      embed, `Last block of source ${n}`);
      assert(last, `page ${n}: the embed shows its source's last block`);
      stage = `page ${n}: edit the last embedded block`;
      const content = await browser.$(`${embed} [data-block-id="${last}"] > .block-main .block-content`);
      await content.scrollIntoView({ block: "center" });
      await content.click();
      await browser.$(`${embed} [data-block-id="${last}"] > .block-main .block-editor, ${embed} [data-block-id="${last}"] > .block-main [data-block-editor]`).waitForExist({ timeout: 10000 });
      await untilEditor(browser, "the clicked block editing and focused", st => st.id === last && st.text !== null);
      const sentinel = `entered s${index} p${n}`;
      const file = path.join(graph, `pages/Source ${n}.md`);
      const lineAfterLast = (text) => { const lines = text.split("\n"); const at = lines.indexOf(`- Last block of source ${n}`); return at >= 0 ? lines[at + 1] ?? "" : null; };
      stage = `page ${n}: caret to the end`;
      await browser.keys(["Control", "End"]);
      await untilEditor(browser, "the caret at the end of the last block", st => st.id === last && st.start === st.end && st.end === st.text.length);
      stage = `page ${n}: Enter`;
      await browser.keys("Enter");
      // A new, empty block is editing and has focus, inside the embed and not the block Enter was pressed in.
      await untilEditor(browser, "a new empty block editing after Enter", st => st.id !== null && st.id !== last && st.inEmbed && st.text === "");
      const fresh = (await editorState(browser)).id;
      stage = `page ${n}: type`;
      await browser.keys(sentinel);
      await untilEditor(browser, "the typed text in the new block", st => st.id === fresh && st.text === sentinel);
      // Committed: the file shows it as the next sibling of the last block (saves do not wait for the editor to close).
      await expected("the typed block saved as the next sibling of the last block",
        () => waitForFileText(file, text => lineAfterLast(text) === `- ${sentinel}`, { timeoutMs: 15_000 }));
      stage = `page ${n}: Tab`;
      await browser.keys("Tab");
      // The same block keeps the caret, and the file shows it as a child of the last block.
      await untilEditor(browser, "the new block still editing after Tab", st => st.id === fresh && st.text === sentinel);
      stage = `page ${n}: Escape`;
      await browser.keys("Escape");
      await untilNoEditor(browser, "no editor open after Escape");
      let saved = null;
      try {
        saved = await waitForFileText(file, text => /^[\t ]+- /.test(lineAfterLast(text) ?? "") && lineAfterLast(text).trim() === `- ${sentinel}`, { timeoutMs: 15_000 });
      } catch { saved = fs.readFileSync(file, "utf8"); }
      const view = await browser.execute((sel, s) => ({
        copies: [...document.querySelectorAll(`${sel} .block-content`)].filter(el => el.textContent.trim() === s).length,
        trap: [...document.querySelectorAll(".toast, [role=alert], [role=status]")].map(el => el.textContent).filter(t => /lsdoc-wasm trap/.test(t)),
      }), embed, sentinel);
      const lines = saved.split("\n");
      const at = lines.indexOf(`- Last block of source ${n}`);
      const edit = { page: n, sentinel, childOfLast: at >= 0 && /^[\t ]+- /.test(lines[at + 1] ?? "") && lines[at + 1].trim() === `- ${sentinel}`,
        lastUnchanged: at >= 0, embedCopies: view.copies, trapToasts: view.trap };
      result.edits.push(edit);
      fs.writeFileSync(path.join(run, `source-${n}.md`), saved);
      execFileSync("import", ["-window", "root", path.join(run, `page-${n}.png`)], { timeout: 10000 });
    }
    result.outcome = result.edits.every(e => e.childOfLast && e.lastUnchanged && e.embedCopies === 1 && !e.trapToasts.length) ? "pass" : "fail";
  } catch (error) {
    result.outcome = "error"; result.stage = stage; result.observed = String(error);
    result.classification = error.code === "ERR_ASSERTION" ? "product" : "harness";
    if (browser) try { fs.writeFileSync(path.join(run, "failure-document.html"), await browser.execute(() => document.documentElement.outerHTML)); } catch {}
  } finally {
    if (browser) await browser.deleteSession().catch(() => {});
    try { process.kill(-driver.pid, "SIGTERM"); } catch {}
    fixture.close?.(); fs.closeSync(log);
  }
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ testedCommit, ...result }, null, 2));
  return result;
}

const results = [];
for (let i = 1; i <= sessions; i++) {
  const r = await session(i);
  results.push(r);
  console.log(`session ${i}/${sessions}: ${r.outcome}${r.observed ? ` (${r.stage}: ${r.observed})` : ""}`);
}
const traps = results.filter(r => r.edits.some(e => e.trapToasts.length)).length;
const summary = { testedCommit, sessions, pagesPerSession: pages, pass: results.filter(r => r.outcome === "pass").length,
  fail: results.filter(r => r.outcome === "fail").length, error: results.filter(r => r.outcome === "error").length, sessionsWithTrap: traps,
  failures: results.filter(r => r.outcome !== "pass").map(r => ({ session: r.session, run: r.run, stage: r.stage, observed: r.observed, edits: r.edits })) };
fs.writeFileSync(path.join(out, `summary-${Date.now()}.json`), JSON.stringify(summary, null, 2));
console.log(JSON.stringify({ ...summary, failures: summary.failures.length }));
if (summary.pass !== sessions) process.exitCode = 1;
