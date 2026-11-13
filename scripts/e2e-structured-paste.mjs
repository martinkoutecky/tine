// Linux real-app proof for GH #58: dispatch the browser's real ClipboardEvent
// and DataTransfer HTML/plain flavors, then verify the persisted graph outline.
// Also og batch 7 (block Cut/Copy): keyboard Copy and Cut of a selected block
// through the native clipboard, the cut subtree leaving its source only once the
// clipboard holds it, and a keyboard paste reproducing the nested outline.
import { spawn } from "node:child_process";
import { remote } from "webdriverio";
import { setTimeout as sleep } from "node:timers/promises";
import fs from "node:fs";
import crypto from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { saveJourneyScreenshot } from "./lib/e2e-screenshot.mjs";
import { nativeDialogWindows } from "./lib/e2e-native-dialog.mjs";
import { execFileSync } from "node:child_process";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const testedBase = execFileSync("git", ["-C", ROOT, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const APP = process.env.TINE_APP || path.join(ROOT, "target/release/tine");
const TD = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin", "tauri-driver") : "tauri-driver");
const DRIVER_PORT = Number(process.env.E2E_DRIVER_PORT || 4474);
const NATIVE_PORT = Number(process.env.E2E_NATIVE_PORT || 4475);
const TMP = process.env.E2E_TMP_DIR || `/tmp/tine-structured-paste-e2e-${process.pid}`;
const ARTIFACTS = process.env.E2E_ARTIFACT_DIR || TMP;

const GRAPH = `${TMP}/graph`;
const PAGE = `${GRAPH}/pages/Paste.md`;
const CUT_SOURCE = `${GRAPH}/pages/Cut Source.md`;
const CUT_TARGET = `${GRAPH}/pages/Cut Target.md`;
const FOLDED_SOURCE = `${GRAPH}/pages/Folded Source.md`;

for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(`${GRAPH}/${dir}`, { recursive: true });
for (const dir of ["data", "config", "cache", "state"]) fs.mkdirSync(`${TMP}/xdg/${dir}`, { recursive: true });
fs.mkdirSync(`${TMP}/home`, { recursive: true });
fs.writeFileSync(`${GRAPH}/logseq/config.edn`, "{}\n");
fs.writeFileSync(PAGE, "- paste here\n");
const CUT_SOURCE_BYTES = "- keep me\n- cut parent\n  - cut child\n- copy me\n";
fs.writeFileSync(CUT_SOURCE, CUT_SOURCE_BYTES);
fs.writeFileSync(CUT_TARGET, "- target first\n");
const FOLDED_SOURCE_BYTES = "- folded parent\n  collapsed:: true\n  - folded child\n- folded sibling\n";
fs.writeFileSync(FOLDED_SOURCE, FOLDED_SOURCE_BYTES);
const UNIT_SOURCE = `${GRAPH}/pages/Selection Units.md`;
const UNIT_SOURCE_BYTES = "- A\n- B\n  - B1\n    - B1a\n- D\n  - D1\n- E\n";
fs.writeFileSync(UNIT_SOURCE, UNIT_SOURCE_BYTES);
const now = new Date();
const journal = `${now.getFullYear()}_${String(now.getMonth() + 1).padStart(2, "0")}_${String(now.getDate()).padStart(2, "0")}`;
fs.writeFileSync(`${GRAPH}/journals/${journal}.md`, "- open [[Paste]]\n");

const env = {
  ...process.env,
  HOME: `${TMP}/home`, TINE_SOURCE_REVISION: testedBase,
  TINE_GRAPH: GRAPH,
  XDG_DATA_HOME: `${TMP}/xdg/data`, XDG_CONFIG_HOME: `${TMP}/xdg/config`, XDG_CACHE_HOME: `${TMP}/xdg/cache`,
  XDG_STATE_HOME: `${TMP}/xdg/state`,
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
};
fs.mkdirSync(ARTIFACTS, { recursive: true });
const account = accountHomeFixture(TMP, ARTIFACTS, env);
const log = fs.openSync(path.join(ARTIFACTS, "structured-paste-tauri-driver.log"), "w");
const td = spawn(TD, ["--port", String(DRIVER_PORT), "--native-port", String(NATIVE_PORT), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], {
  env: account.env, stdio: ["ignore", log, log], detached: true,
});
await sleep(2500);

let browser;
let stage = "startup";
const proof = { base: testedBase,
  app: APP, appSha256: crypto.createHash("sha256").update(fs.readFileSync(APP)).digest("hex"), observations: [], outcome: "pending" };
try {
  browser = await remote({
    hostname: "127.0.0.1", port: DRIVER_PORT, path: "/", logLevel: "error", connectionRetryCount: 1, connectionRetryTimeout: 60_000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: APP } },
  });
  await ensureMainWindow(browser);
  await assertAppLease(account);
  await browser.$(".ls-block, .page-title").waitForExist({ timeout: 20_000 });
  stage = "structured paste";
  await openPageByName(browser, "Paste");
  const target = await browser.$(".ls-block .block-content");
  await target.click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  const result = await browser.execute(() => {
    const editor = document.querySelector("textarea.block-editor");
    if (!editor) return { ok: false, error: "no editor" };
    if (typeof DataTransfer !== "function" || typeof ClipboardEvent !== "function") {
      return { ok: false, error: "browser clipboard constructors unavailable" };
    }
    editor.value = "";
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "deleteContent", data: null }));
    const clipboard = new DataTransfer();
    clipboard.setData("text/plain", "Parent\nChild bold\nSibling");
    clipboard.setData("text/html", "<ul><li>Parent<ul><li>Child <strong>bold</strong></li></ul></li><li>Sibling</li></ul>");
    const event = new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: clipboard });
    editor.dispatchEvent(event);
    return { ok: event.defaultPrevented, types: [...clipboard.types] };
  });
  if (!result.ok || !result.types.includes("text/html") || !result.types.includes("text/plain")) {
    throw new Error(`structured clipboard was not handled: ${JSON.stringify(result)}`);
  }
  await sleep(600);
  await browser.keys(["Escape"]);
  await sleep(1800);
  const saved = fs.readFileSync(PAGE, "utf8");
  if (!/^- Parent\n\s+- Child \*\*bold\*\*\n- Sibling\s*$/m.test(saved)) {
    throw new Error(`wrong persisted structured paste:\n${saved}`);
  }
  console.log("PASS: real ClipboardEvent preserved nested HTML and persisted one atomic outline");

  // ---- og batch 7: keyboard Copy / Cut of selected blocks -----------------
  const nativeClipboard = async (command, text) => {
    const out = await browser.executeAsync((cmd, value, done) => {
      const timer = setTimeout(() => done({ ok: false, error: "no answer within 2s" }), 2000);
      globalThis.__TAURI_INTERNALS__.invoke(`plugin:clipboard-manager|${cmd}`, value === null ? {} : { text: value })
        .then((value) => { clearTimeout(timer); done({ ok: true, value }); },
          (error) => { clearTimeout(timer); done({ ok: false, error: String(error) }); });
    }, command, text ?? null);
    if (!out.ok) throw new Error(`native clipboard ${command} failed: ${out.error}`);
    return out.value;
  };
  const read = (file) => fs.readFileSync(file, "utf8");
  // Select one block the way a user does: edit it, then Escape to block selection.
  const selectBlock = async (text, visibleCount = 1) => {
    const content = await browser.$(`//div[contains(concat(' ', normalize-space(@class), ' '), ' block-content ') and normalize-space(.) = '${text}']`);
    await content.waitForExist({ timeout: 10_000, timeoutMsg: `block ${JSON.stringify(text)} is not rendered` });
    await content.click();
    await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
    await browser.keys(["Escape"]);
    await browser.waitUntil(() => browser.execute((wanted, count) => {
      const selected = [...document.querySelectorAll(".block-main.selected")];
      return selected.length === count && !document.querySelector("textarea.block-editor")
        && selected[0].querySelector(".block-content")?.textContent?.trim() === wanted;
    }, text, visibleCount), { timeout: 5000, interval: 100, timeoutMsg: `Escape did not select the subtree of ${JSON.stringify(text)}` });
  };
  // Poll the native clipboard. Each read is bounded on both sides, so a read
  // that never answers is reported as such instead of stalling the journey.
  const waitClipboard = async (predicate, what) => {
    const seen = [];
    const deadline = Date.now() + 10_000;
    for (;;) {
      const started = Date.now();
      let value;
      try {
        value = await Promise.race([
          nativeClipboard("read_text"),
          sleep(3000).then(() => { throw new Error("WebDriver round trip did not return within 3s"); }),
        ]);
      } catch (error) { value = `<${error.message}>`; }
      const entry = JSON.stringify(value);
      if (seen.at(-1) !== entry) seen.push(entry);
      if (Date.now() - started > 1000) seen.push(`slow read ${Date.now() - started}ms`);
      if (predicate(value)) return value;
      if (Date.now() > deadline) throw new Error(`${what}; native clipboard readings ${JSON.stringify(seen)}`);
      await sleep(100);
    }
  };

  stage = "keyboard Copy";
  await openPageByName(browser, "Cut Source");
  await browser.waitUntil(() => read(CUT_SOURCE) === CUT_SOURCE_BYTES, { timeout: 5000, timeoutMsg: "Cut Source changed on open" });

  // Copy: the clipboard receives the block; the source is untouched.
  await nativeClipboard("write_text", "structured-paste sentinel");
  await selectBlock("copy me");
  await browser.keys(["Control", "c"]);
  await browser.keys(["Control"]);
  await waitClipboard((text) => typeof text === "string" && /copy me/.test(text), "keyboard Copy did not reach the native clipboard");
  await sleep(500);
  if (read(CUT_SOURCE) !== CUT_SOURCE_BYTES) throw new Error(`Copy changed its source:\n${read(CUT_SOURCE)}`);
  await browser.keys(["Escape"]);

  // GH #941: first Shift+Down selects the current subtree; the next skips its
  // descendants and takes the next sibling subtree. Copy must include both.
  stage = "GH941 Shift+Down and subtree Copy";
  await openPageByName(browser, "Selection Units");
  await browser.$("//div[contains(@class, 'block-content') and normalize-space(.) = 'B']").click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  await browser.keys(["End"]);
  const selectedText = () => browser.execute(() => [...document.querySelectorAll(".main-content .block-main.selected")]
    .map(row => row.querySelector(".block-content")?.textContent.trim()));
  await browser.keys(["Shift", "ArrowDown"]); await browser.keys(["Shift"]);
  await browser.waitUntil(async () => JSON.stringify(await selectedText()) === JSON.stringify(["B", "B1", "B1a"]),
    { timeout: 5000, timeoutMsg: "First Shift+Down did not select B's complete subtree" });
  await browser.keys(["Shift", "ArrowDown"]); await browser.keys(["Shift"]);
  await browser.waitUntil(async () => JSON.stringify(await selectedText()) === JSON.stringify(["B", "B1", "B1a", "D", "D1"]),
    { timeout: 5000, timeoutMsg: "Second Shift+Down did not extend across D's complete subtree" });
  await nativeClipboard("write_text", "subtree sentinel");
  await browser.keys(["Control", "c"]); await browser.keys(["Control"]);
  const unitText = await waitClipboard(text => typeof text === "string" && text.startsWith("- B\n"), "Subtree Copy did not reach the clipboard");
  const unitLines = unitText.trimEnd().split("\n").map(line => line.match(/^(\s*)-\s+(.*)$/));
  const expectedText = ["B", "B1", "B1a", "D", "D1"];
  const indents = unitLines.map(line => line?.[1].replaceAll("\t", "  ").length ?? -1);
  if (JSON.stringify(unitLines.map(line => line?.[2])) !== JSON.stringify(expectedText)
    || !(indents[0] === indents[3] && indents[1] > indents[0] && indents[2] > indents[1] && indents[4] === indents[1])) {
    throw new Error(`Subtree Copy lost content or hierarchy: ${JSON.stringify(unitText)}`);
  }
  await waitForFileText(UNIT_SOURCE, text => text === UNIT_SOURCE_BYTES);
  console.log("PASS GH941: Shift+Down selects sibling subtrees and native Copy retains all descendants");
  proof.observations.push({ stage, selected: expectedText, clipboard: unitText, sourceUnchanged: true });
  stage = "GH941 selection screenshot";
  await saveJourneyScreenshot(browser, path.join(ARTIFACTS, "subtree-selection.png"));
  await browser.keys(["Escape"]);

  // GH #943: a collapsed selected parent also includes its hidden children.
  stage = "GH943 folded Copy";
  await openPageByName(browser, "Folded Source");
  await nativeClipboard("write_text", "structured-paste sentinel");
  await selectBlock("folded parent");
  await browser.keys(["Control", "c"]);
  await browser.keys(["Control"]);
  const foldedText = await waitClipboard((text) => typeof text === "string" && /folded parent/.test(text),
    "keyboard Copy of a collapsed block did not reach the native clipboard");
  if (!/folded parent[\s\S]*folded child/.test(foldedText)) throw new Error(`collapsed Copy lost its hidden child: ${JSON.stringify(foldedText)}`);
  if (read(FOLDED_SOURCE) !== FOLDED_SOURCE_BYTES) throw new Error(`Copy changed its source:\n${read(FOLDED_SOURCE)}`);
  await browser.keys(["Escape"]);
  await openPageByName(browser, "Cut Source");

  // Cut: the clipboard receives the parent AND its child, then (only then) the
  // subtree leaves the source while its siblings stay byte-identical.
  stage = "keyboard Cut and cross-page paste";
  await selectBlock("cut parent", 2);
  await browser.keys(["Control", "x"]);
  await browser.keys(["Control"]);
  // Both public text and Tine's private clipboard payload carry the subtree.
  const cutText = await waitClipboard((text) => typeof text === "string" && /cut parent/.test(text),
    "keyboard Cut did not put the selected block on the native clipboard");
  await browser.waitUntil(() => read(CUT_SOURCE) === "- keep me\n- copy me\n", {
    timeout: 10_000, timeoutMsg: `Cut did not remove exactly the cut subtree; source is ${JSON.stringify(read(CUT_SOURCE))}`,
  });

  // Paste into another page with the keyboard: the nested outline arrives.
  await openPageByName(browser, "Cut Target");
  const first = await browser.$(`//div[contains(concat(' ', normalize-space(@class), ' '), ' block-content ') and normalize-space(.) = 'target first']`);
  await first.waitForExist({ timeout: 10_000 });
  await first.click();
  await browser.$("textarea.block-editor").waitForExist({ timeout: 5000 });
  await browser.keys(["End"]);
  await browser.keys(["Enter"]);
  await browser.waitUntil(() => browser.execute(() => document.querySelector("textarea.block-editor")?.value === ""), {
    timeout: 5000, timeoutMsg: "Enter did not open an empty sibling editor",
  });
  await browser.keys(["Control", "v"]);
  await browser.keys(["Control"]);
  await sleep(600);
  await browser.keys(["Escape"]);
  await browser.waitUntil(() => /^- target first\n- cut parent\n\s+- cut child\n?$/.test(read(CUT_TARGET)), {
    timeout: 10_000, timeoutMsg: `keyboard paste did not reproduce the cut outline; clipboard ${JSON.stringify(cutText)}, target ${JSON.stringify(read(CUT_TARGET))}`,
  });
  if (read(CUT_SOURCE) !== "- keep me\n- copy me\n") throw new Error(`paste disturbed the cut source:\n${read(CUT_SOURCE)}`);
  console.log("PASS: keyboard Copy kept its source and carried a collapsed block's hidden child; keyboard Cut moved the nested subtree through the native clipboard");
  proof.outcome = "PASS";
  account.verify();
} catch (error) {
  proof.outcome = "FAIL";
  proof.failureCapsule = { testedCommit: proof.base, journey: stage,
    expected: "Keyboard selection and Copy retain whole subtrees; Copy leaves its routed source unchanged and Cut/paste preserves the moved outline",
    observed: String(error.stack || error), evidence: ["structured-paste-tauri-driver.log", "structured-paste-failure.png", "failure-dom.html"],
    classification: stage.endsWith("screenshot") ? "harness" : stage === "startup" ? "infrastructure" : "ambiguous (inspect DOM, screenshot and driver log)" };
  for (const id of nativeDialogWindows()) {
    let name = "";
    try { name = execFileSync("xdotool", ["getwindowname", id], { encoding: "utf8" }).trim(); } catch {}
    console.error(`native dialog open: ${id} ${JSON.stringify(name)}`);
  }
  try { if (browser) await saveJourneyScreenshot(browser, path.join(ARTIFACTS, "structured-paste-failure.png")); } catch {}
  try { fs.writeFileSync(path.join(ARTIFACTS, "failure-dom.html"), await browser.getPageSource()); } catch {}
  for (const file of [PAGE, CUT_SOURCE, CUT_TARGET, FOLDED_SOURCE]) {
    try { console.error(`${path.basename(file)}:`, JSON.stringify(fs.readFileSync(file, "utf8"))); } catch {}
  }
  try { console.error("toasts:", JSON.stringify(await browser.execute(() => [...document.querySelectorAll(".toast")].map((node) => node.textContent)))); } catch {}
  throw error;
} finally {
  if (proof.failureCapsule) proof.failureCapsule.evidence = proof.failureCapsule.evidence.filter(file => fs.existsSync(path.join(ARTIFACTS, file)));
  fs.writeFileSync(path.join(ARTIFACTS, "proof.json"), JSON.stringify(proof, null, 2) + "\n");
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-td.pid, "SIGKILL"); } catch {}
  account.close();
  fs.closeSync(log);
}
