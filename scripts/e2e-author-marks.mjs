// GH #892/#893/#896/#999: real WebKitGTK layout, sibling run tint cards, both editors,
// independent Settings/palette/rebound-key visibility (hidden: no tint, line or chip in either look),
// the author's name on hover over the chip and the thread line only, and guarded removal.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { installAccountHome, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const root = path.resolve(import.meta.dirname, "..");
const app = path.resolve(process.env.TINE_APP || path.join(root, "target/release/tine"));
const out = path.resolve(process.env.E2E_ARTIFACT_DIR || "target/artifacts/og-mc010-author-marks/native");
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, "fixture-"));
const graph = path.join(run, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(run, dir), { recursive: true });
const pageName = "Author marks";
const file = path.join(graph, "pages", `${pageName}.md`);
const initial = "- Run alpha\n  tine.author:: Ada\n- Run beta\n  tine.author:: Ada\n- Run gamma\n  tine.author:: Ada\n- Owner gap\n- Parent outline\n  tine.author:: Ada\n\t- Child first\n\t  tine.author:: Ada\n\t- Child next\n\t  tine.author:: Ada\n- After nesting\n  tine.author:: Ada\n- With comment\n  tine.author:: Ada\n\t- A comment message\n\t  tine.quote:: With comment\n\t  tine.author:: Bo\n- After thread\n  tine.author:: Ada\n- {{embed [[Author source]]}}\n  tine.author:: Ada\n- After embed\n  tine.author:: Ada\n- > A quoted sentinel\n  tine.author:: Quote Agent  \t\n- Closed parent\n  tine.author:: Bo\n  collapsed:: true\n\t- Hidden child\n";
const extraFixture = "- Numbered block\n  tine.author:: Ada\n  logseq.order-list-type:: number\n- {{embed ((12345678-1234-4000-8000-123456789abc))}}\n  tine.author:: Bo\n";
fs.writeFileSync(file, initial + extraFixture);
const source = "- Source first\n  tine.author:: Ada\n  id:: 12345678-1234-4000-8000-123456789abc\n- Source next\n  tine.author:: Ada\n";
fs.writeFileSync(path.join(graph, "pages/Author source.md"), source);
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const fixture = installAccountHome(run, out);
const env = { ...fixture.env, TINE_GRAPH: graph, HOME: path.join(run, "home"),
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_CACHE_HOME: path.join(run, "cache"), XDG_STATE_HOME: path.join(run, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4652);
const nativePort = Number(process.env.E2E_NATIVE_PORT || 4653);
const td = process.env.TAURI_DRIVER || (process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, "bin/tauri-driver") : "tauri-driver");
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(td, ["--port", String(port), "--native-port", String(nativePort), "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"],
  { env, stdio: ["ignore", log, log], detached: true });
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const proof = { testedCommit, delta: process.env.E2E_AUTHOR_BASELINE ? "base production" : "working tree", app,
  binarySha256: createHash("sha256").update(fs.readFileSync(app)).digest("hex"), steps: [], screenshots: [] };
let browser, stage = "launch";
const keys = async (...values) => { await browser.keys(values); await browser.releaseActions(); };
async function shot(name) {
  await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
  execFileSync("import", ["-window", "root", path.join(out, `${name}.png`)]);
  proof.screenshots.push(`${name}.png`);
}
async function blockId(text) {
  let id;
  await browser.waitUntil(async () => {
    id = await browser.execute(text => [...document.querySelectorAll(".page-blocks .ls-block:not(.ref-block)")].find(row =>
      row.querySelector(":scope > .block-main > .block-content-wrapper > .block-content:not(.macro-host)")?.textContent.trim().includes(text))?.dataset.blockId, text);
    return !!id;
  }, { timeout: 10000, timeoutMsg: `Missing block ${text}` });
  assert.ok(id, `Missing block ${text}`); return id;
}
const rowSelector = id => `.page-blocks [data-block-id="${id}"]`;
async function snapshot(id) {
  return browser.execute(id => {
    const row = [...document.querySelectorAll(".page-blocks .ls-block")].find(row => row.dataset.blockId === id);
    const wrapper = row.querySelector(":scope > .block-main > .block-content-wrapper");
    const content = wrapper.querySelector(":scope > .block-content");
    const bullet = row.querySelector(":scope > .block-main > .block-controls .bullet-container");
    const wr = wrapper.getBoundingClientRect(), br = bullet?.getBoundingClientRect();
    const background = getComputedStyle(wrapper).backgroundColor;
    const tinted = row.classList.contains("author-tinted") && background !== "rgba(0, 0, 0, 0)" && background !== "transparent";
    // While the block is being edited the editor replaces .block-content: there is no rendered text to measure.
    let textX = null;
    if (content) {
      const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
      let text; while ((text = walker.nextNode())) if (text.textContent.trim() && !text.parentElement.closest(".author-chip, .ref-count, .margin-marker")) break;
      if (text) { const range = document.createRange(); range.setStart(text, 0); range.setEnd(text, 1); textX = range.getBoundingClientRect().left; }
    }
    return { textX, wrapperX: wr.left,
      bulletRight: br?.right, tint: tinted ? { left: wr.left, right: wr.right, top: wr.top, bottom: wr.bottom } : null,
      gutterLines: row.querySelectorAll(".author-mark").length, title: wrapper.getAttribute("title"),
      // The chip the reader sees: a results list (an embed) renders a joined hit's chip but hides it (`display: none`).
      chip: (c => c && c.getClientRects().length > 0 ? c.textContent : null)(wrapper.querySelector(".author-chip")) };
  }, id);
}
/** The tooltip the pointer gets: the `title` of the hovered element or of its nearest ancestor. */
async function tooltipOver(element) {
  await element.moveTo();
  return browser.execute(() => [...document.querySelectorAll(":hover")].pop()?.closest("[title]")?.getAttribute("title") ?? null);
}
/** Author chips drawn on outline rows (a comment's or reply's own header is not an author mark). */
const outlineChips = () => browser.execute(() => [...document.querySelectorAll(".page-blocks .ls-block:not(.comment-in-thread) > .block-main .author-chip")]
  .filter(chip => chip.getClientRects().length > 0 && getComputedStyle(chip).visibility !== "hidden").map(chip => chip.textContent.trim()));
/** Settings > Appearance > Advanced > Fine-tuning > Author marks. */
async function authorLook(value) {
  await settings(); await browser.$('.settings-nav-item=Appearance').click();
  const look = await browser.$('select[aria-label="Author marks"]');
  if (!(await look.isExisting())) await browser.$(".settings-advanced-toggle").click();
  await look.waitForExist({ timeout: 5000 }); await look.scrollIntoView(); await look.selectByAttribute("value", value);
  // Escape would first fold Advanced away: close Settings with its own button.
  await browser.$(".settings-close").click(); await browser.$(".settings-modal").waitForExist({ timeout: 5000, reverse: true });
  assert.equal(await browser.execute(() => document.documentElement.classList.contains("tine-author-marks-thread")), value === "thread", `Author marks = ${value} did not apply`);
}
async function palette(label) {
  await keys("Control", "Shift", "p");
  const search = await browser.$(".switcher-input");
  await search.waitForExist({ timeout: 5000 }); await search.setValue(label);
  await keys("Enter");
  await browser.$(".switcher").waitForExist({ timeout: 5000, reverse: true });
}
async function settings() {
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForExist({ timeout: 5000 });
}
try {
  await sleep(1500);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 60000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser);
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  await browser.setWindowSize(1550, 1000); await openPageByName(browser, pageName);
  const ids = {};
  for (const text of ["Run alpha", "Run beta", "Run gamma", "Parent outline", "Child first", "Child next", "After nesting", "With comment", "After thread", "After embed", "Source first", "Source next"])
    ids[text] = await blockId(text);
  stage = "tint geometry and one chip per run";
  const alpha = await snapshot(ids["Run alpha"]), beta = await snapshot(ids["Run beta"]), gamma = await snapshot(ids["Run gamma"]);
  proof.steps.push({ stage, alpha, beta, gamma });
  await shot("01-reading-runs");
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  assert.ok(alpha.tint, `No author tint: text x=${alpha.textX}, wrapper x=${alpha.wrapperX}`);
  assert.equal(alpha.gutterLines, 0, "The old gutter line element is still drawn");
  assert.equal(alpha.chip, "Ada"); assert.equal(beta.chip, null); assert.equal(gamma.chip, null);
  // Text keeps the bullet-to-letter offset of an unauthored block (6px dot, 4px number).
  assert.ok(Math.abs(alpha.textX - alpha.bulletRight - 6) <= 0.75, "Authored text moved from the plain bullet-to-text offset");
  const closed = await snapshot(await blockId("Closed parent"));
  assert.ok(closed.tint && Math.abs(closed.textX - closed.bulletRight - 6) <= 0.75, "Folded authored block lost its tint or moved its text");
  proof.steps.push({ outcome: "folded authored block is tinted and its text has the plain offset", closed });
  const number = await snapshot(await blockId("Numbered block"));
  proof.steps.push({ stage: "numbered bullet geometry", number });
  assert.ok(number.tint && Math.abs(number.textX - number.bulletRight - 4) <= 0.75, "Numbered authored block lost its tint or moved its text");
  await browser.$(".block-embed-host .ls-block.authored").waitForExist({ timeout: 10000 });
  assert.equal((await browser.$$(".block-embed-host.author-tinted")).length, 0, "Bulletless embed host tinted itself");
  // The chip clips its own box (overflow: hidden, for the icon knob), so WebDriver's getText atom reads it as empty:
  // assert the outcome with the chip's text content, that the driver finds it displayed, and that it has a visible box.
  const embedChip = await browser.$(".block-embed-host .ls-block > .block-main .author-chip");
  assert.equal((await embedChip.getProperty("textContent")).trim(), "Ada", "Block embed inherited its host's author instead of showing the source author");
  assert.ok(await embedChip.isDisplayed(), "The block embed's source author chip is not visible");
  assert.ok((await embedChip.getSize()).width > 0, "The block embed's source author chip has no box");
  assert.ok(Math.abs(alpha.tint.bottom - beta.tint.top) <= 0.5 && Math.abs(beta.tint.bottom - gamma.tint.top) <= 0.5, "Same-author sibling tint has a gap");
  for (const text of ["Parent outline", "Child first", "After nesting", "After thread", "After embed", "Source first"])
    assert.equal((await snapshot(ids[text])).chip, "Ada", `${text} must start a separate run`);
  for (const text of ["Child next", "With comment", "Source next"])
    assert.equal((await snapshot(ids[text])).chip, null, `${text} must continue its sibling run`);
  // GH #999: the block's text box shows no tooltip; the name is on the chip (and, in the thread look, the line).
  for (const text of ["Run alpha", "Run beta", "Run gamma"]) assert.equal((await snapshot(ids[text])).title, null, `${text} shows a tooltip over its text`);
  assert.equal(await tooltipOver(await browser.$(`${rowSelector(ids["Run beta"])} > .block-main > .block-content-wrapper > .block-content`)), null, "Hovering a block's text shows a tooltip");
  assert.match(await tooltipOver(await browser.$(`${rowSelector(ids["Run alpha"])} .author-chip`)) ?? "", /Ada/, "Hovering the chip does not name its author");
  await shot("02-name-on-the-chip");
  assert.equal(fs.readFileSync(file, "utf8"), initial + extraFixture, "Reading/disclosing marks changed the graph");

  for (const editor of ["textarea", "cm6"]) {
    stage = `${editor}: edit continuation with retained author and tint`;
    await settings();
    await browser.$(".settings-search-input").setValue("CodeMirror");
    await browser.$(".settings-search-result").click();
    const cm = await browser.$('[data-setting-id="settings-cm6-label"] [role="switch"]');
    if (await cm.getAttribute("aria-checked") !== String(editor === "textarea")) await cm.click();
    await keys("Escape");
    await browser.$(`${rowSelector(ids["Run gamma"])} .block-content`).click();
    const input = await browser.$(editor === "cm6" ? '[data-block-editor="cm6"] .cm-content' : "textarea.block-editor");
    await input.waitForExist({ timeout: 10000 });
    assert.ok((await snapshot(ids["Run gamma"])).tint, `${editor}: the tint went away while editing`);
    assert.ok(!(editor === "cm6" ? await input.getText() : await input.getValue()).includes("tine.author"));
    await keys("Control", "End"); await browser.keys(` ${editor}`);
    await shot(`03-edit-${editor}`); await keys("Escape");
    await waitForFileText(file, text => text.includes(`Run gamma${editor === "cm6" ? " textarea cm6" : " textarea"}\n  tine.author:: Ada`));
    proof.steps.push(stage);
  }
  stage = "independent Settings switches preserve text coordinates";
  const before = await snapshot(ids["Run alpha"]);
  await settings(); await browser.$('.settings-nav-item=Appearance').click();
  const marksSwitch = await browser.$('[data-setting-id="settings-author-marks"] [role="switch"]');
  const commentsSwitch = await browser.$('[data-setting-id="settings-ui-comment-margin"] [role="switch"]');
  assert.equal(await marksSwitch.getAttribute("aria-checked"), "true");
  assert.equal(await commentsSwitch.getAttribute("aria-checked"), "true");
  const grouped = await browser.execute(() => {
    const a = document.querySelector('[data-setting-id="settings-author-marks"]');
    return a.closest('[role="group"]')?.contains(document.querySelector('[data-setting-id="settings-ui-comment-margin"]')) ?? false;
  });
  assert.equal(grouped, true); await marksSwitch.click();
  assert.equal(await commentsSwitch.getAttribute("aria-checked"), "true");
  await shot("04-settings-independent-switches"); await keys("Escape");
  const hidden = await snapshot(ids["Run alpha"]);
  // GH #896: hidden means no tint and no chip on any block, while the comment keeps its card.
  assert.equal(hidden.tint, null); assert.equal(hidden.textX, before.textX); assert.equal(hidden.chip, null);
  assert.deepEqual(await outlineChips(), [], "Author chips stay with author marks hidden");
  assert.equal(await browser.$(".comment-card").isDisplayed(), true);
  await shot("05-tint-hidden-text-stable");
  await browser.refresh(); await ensureMainWindow(browser); await openPageByName(browser, pageName);
  assert.equal((await snapshot(await blockId("Run alpha"))).tint, null, "Visibility preference was lost after reload");
  stage = "palette and user-bound shortcut";
  await palette("Show/Hide author marks");
  await browser.$(".author-tinted").waitForExist({ timeout: 5000 });
  await settings(); await browser.$('.settings-nav-item=Keyboard shortcuts').click();
  const shortcut = '[data-shortcut-id="ui/toggle-author-marks"]';
  assert.equal(await browser.$(`${shortcut} .help-keycap-button`).getAttribute("class").then(value => value.includes("disabled")), true, "Author command unexpectedly has a default key");
  await browser.$(`${shortcut} button[title="Add keybinding"]`).click(); await keys("F8");
  await keys("Escape"); await keys("F8");
  await browser.$(".author-tinted").waitForExist({ timeout: 5000, reverse: true });
  await keys("F8"); await browser.$(".author-tinted").waitForExist({ timeout: 5000 });
  proof.steps.push({ outcome: "independent persisted switch, palette and rebound F8; text coordinate unchanged", before, hidden });
  stage = "thread look: the name on hover over the chip and the line only; hidden takes every mark";
  await authorLook("thread");
  const childNext = rowSelector(await blockId("Child next"));
  const threadHover = {
    text: await tooltipOver(await browser.$(`${childNext} > .block-main > .block-content-wrapper > .block-content`)),
    chip: await tooltipOver(await browser.$(`${rowSelector(await blockId("Parent outline"))} .author-chip`)),
    line: await tooltipOver(await browser.$(`${childNext} > .block-main > .author-line`)),
  };
  const shownThreadChips = await outlineChips();
  await shot("05b-thread-shown");
  await keys("F8"); await browser.$(".author-tinted").waitForExist({ timeout: 5000, reverse: true });
  const hiddenThreadChips = await outlineChips();
  // Moving the pointer also retires the tooltip the line opened (WebKitGTK keeps it until the pointer moves).
  const hiddenHover = await tooltipOver(await browser.$(`${childNext} > .block-main > .block-content-wrapper > .block-content`));
  proof.steps.push({ stage, threadHover, shownThreadChips, hiddenThreadChips, hiddenHover });
  await shot("05c-thread-hidden");
  assert.equal(threadHover.text, null, "Thread look: hovering a block's text shows a tooltip");
  assert.match(threadHover.chip ?? "", /Ada/, "Thread look: hovering the chip does not name its author");
  assert.match(threadHover.line ?? "", /Ada/, "Thread look: hovering the line does not name its author");
  assert.deepEqual(hiddenThreadChips, [], "Thread look: chips are drawn with author marks hidden");
  assert.equal(hiddenHover, null, "Thread look: a tooltip shows with author marks hidden");
  assert.equal((await browser.$$(".page-blocks .author-line")).length, 0, "Thread look: the line's hover target stays with author marks hidden");
  assert.equal(await browser.$(".comment-card").isDisplayed(), true);
  await keys("F8"); await browser.$(".author-tinted").waitForExist({ timeout: 5000 });
  await authorLook("tint");
  stage = "first chip removes author through the guarded save and undo restores it";
  const first = rowSelector(await blockId("Run alpha"));
  await browser.$(`${first} .author-chip`).click();
  await browser.$('.ctx-item=Remove author').click();
  await waitForFileText(file, text => text.startsWith("- Run alpha\n- Run beta\n"));
  assert.equal((await snapshot(await blockId("Run beta"))).chip, "Ada");
  await keys("Control", "z");
  await waitForFileText(file, text => text.startsWith("- Run alpha\n  tine.author:: Ada\n"));
  assert.equal((await snapshot(await blockId("Run beta"))).chip, null);
  assert.equal(fs.readFileSync(path.join(graph, "pages/Author source.md"), "utf8"), source);
  assert.ok(fs.readFileSync(file, "utf8").includes("tine.author:: Quote Agent  \t"));
  await shot("06-completed"); proof.steps.push(stage);
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  console.log("PASS: tinted continuous author runs, context boundaries, names on the chip and line only, both editors, independent persisted visibility hiding every mark, thread look, palette, rebound key, remove/undo");
} catch (error) {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2));
  const failure = { testedCommit, journey: "author-marks", stage, expected: "tinted sibling runs with independent visibility and retained authorship in both editors",
    observed: String(error), classification: process.env.E2E_AUTHOR_BASELINE ? "product" : "ambiguous", screenshot: "failure.png" };
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify(failure, null, 2));
  try { await shot("failure"); } catch { /* Driver may have failed to launch. */ }
  console.error(failure); throw error;
} finally {
  try { await browser?.deleteSession(); } catch { /* Preserve the original failure. */ }
  try { process.kill(-driver.pid, "SIGKILL"); } catch { /* Only this journey's driver. */ }
  fs.closeSync(log);
}
