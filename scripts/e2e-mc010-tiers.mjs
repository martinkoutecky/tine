// D-20261107-MC3 §§6–7 / D-20261108-MC4, MC7, MC8: routed-page proof in both editors.
// GH #891 finding 1: a native typing budget beside a commented block with many siblings,
// and (review round 2) in a folded block above many hidden children; (round 3)
// opening one of 64 passages mounts that passage's cards only, with its latency recorded.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import crypto from "node:crypto";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const root = path.resolve(import.meta.dirname, "..");
const modes = process.env.MC010_EDITOR ? [process.env.MC010_EDITOR] : ["textarea", "cm6"];
assert.ok(modes.every(mode => mode === "textarea" || mode === "cm6"), "MC010_EDITOR must be textarea or cm6");
const artifacts = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/og-mc010-tiers");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
console.log(`Artifacts: ${out}`);
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
let ids = { parent: "01000000-0000-4000-8000-000000000001", one: "01000000-0000-4000-8000-000000000002",
  two: "01000000-0000-4000-8000-000000000003", folded: "01000000-0000-4000-8000-000000000004" };
const seed = `- First passage then second passage.
  id:: ${ids.parent}
  - First thread
    id:: ${ids.one}
    tine.quote:: First
    - An existing reply
  - Second thread
    id:: ${ids.two}
    tine.quote:: second
- Collapsed parent
  id:: ${ids.folded}
  collapsed:: true
  - Own folded thread
    tine.quote:: Collapsed
`;
for (const mode of ["textarea", "cm6"]) fs.writeFileSync(path.join(graph, `pages/Comments ${mode}.md`), mode === "cm6" ? seed.replaceAll("01000000", "02000000") : seed);
// 64 ordinary siblings of one comment root; the typed sibling has its own id.
const SIBLINGS = 64, TYPED = 32, TYPING_P95_MS = 16;
const siblingSeed = prefix => `- A commented passage among many siblings
  - Comment on the passage
    tine.quote:: commented passage
${Array.from({ length: SIBLINGS }, (_, i) => `  - Sibling ${i}\n${i === TYPED ? `    id:: ${prefix}000000-0000-4000-8000-000000000032\n` : ""}`).join("")}`;
for (const [mode, prefix] of [["textarea", "03"], ["cm6", "04"]]) fs.writeFileSync(path.join(graph, `pages/Siblings ${mode}.md`), siblingSeed(prefix));
// Review round 2 (PC-3): typing in a folded block whose 64 hidden children hold one thread ("1 below").
const foldedSeed = prefix => `- A folded block with many hidden children
  id:: ${prefix}000000-0000-4000-8000-000000000001
  collapsed:: true
${Array.from({ length: SIBLINGS }, (_, i) => `  - Hidden ${i}\n${i === 0 ? "    - Hidden thread\n      tine.quote:: Hidden\n" : ""}`).join("")}`;
for (const [mode, prefix] of [["textarea", "05"], ["cm6", "06"]]) fs.writeFileSync(path.join(graph, `pages/Folded ${mode}.md`), foldedSeed(prefix));
// Review round 3 (PC-3/PC-4): one block with 64 passages, one thread each.
// Opening one passage mounts that passage's cards only.
const PASSAGES = 64;
const passageSeed = prefix => `- ${Array.from({ length: PASSAGES }, (_, i) => `p${i}q`).join(" ")}
${Array.from({ length: PASSAGES }, (_, i) => `  - Thread on p${i}q\n${i === 1 ? `    id:: ${prefix}000000-0000-4000-8000-000000000101\n` : ""}    tine.quote:: p${i}q\n`).join("")}`;
for (const [mode, prefix] of [["textarea", "07"], ["cm6", "08"]]) fs.writeFileSync(path.join(graph, `pages/Passages ${mode}.md`), passageSeed(prefix));
const fixture = accountHomeFixture(out, out, { ...process.env, TINE_GRAPH: graph, HOME: path.join(out, "home"),
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"), XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11",
  TINE_E2E_TOUCH_GESTURES: "ios" });
const app = process.env.TINE_APP || path.join(root, "target/release/tine");
async function freePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
const port = Number(process.env.E2E_DRIVER_PORT) || await freePort();
const nativePort = Number(process.env.E2E_NATIVE_PORT) || await freePort();
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(nativePort), "--native-driver", "/usr/bin/WebKitWebDriver"],
  { env: fixture.env, stdio: ["ignore", log, log], detached: true });
const base = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const proof = { base, delta: "working tree", app, modes, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"), outcomes: [], typing: [], opening: [], focusProbes: [], controlProbes: [], menuProbes: [], screenshots: [] };
let browser, stage = "startup", editorSelector, runtimeIds, siblingId;
const row = id => `.ls-block[data-block-ref="${id}"]`;
async function shot(name) {
  await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
  execFileSync("import", ["-window", "root", path.join(out, name + ".png")], { timeout: 10000 });
  proof.screenshots.push(name + ".png");
}
async function keys(...value) { await browser.keys(value); await browser.releaseActions(); }
async function value() {
  return browser.execute(selector => {
    const el = document.querySelector(selector);
    return el instanceof HTMLTextAreaElement ? el.value : el ? [...el.querySelectorAll(".cm-line")].map(line => line.textContent).join("\n") : null;
  }, editorSelector);
}
async function type(text) {
  for (const char of text) {
    const before = await value(); await browser.keys(char);
    await browser.waitUntil(async () => await value() !== before, { timeout: 5000 });
  }
}

async function activate(selector, scaled = false) {
  if (!scaled) return browser.$(selector).click();
  await browser.execute(selector => document.querySelector(selector).focus(), selector); await keys("Enter");
}
async function closePopup(scaled = false) { await activate(".comment-popup-close", scaled); await browser.$(".comment-thread-popup").waitForExist({ reverse: true }); }
async function pause(ms) { await browser.executeAsync((ms, done) => setTimeout(done, ms), ms); }
const highlight = id => `.page-blocks ${row(ids.parent)} .comment-quote-anchor[data-comment-id="${id}"]`;
async function shownThread(text) {
  await browser.waitUntil(() => browser.execute(text => [...document.querySelectorAll(".comment-thread-popup [data-block-ref]")].some(el => !el.closest("[hidden]") && el.textContent.includes(text)), text));
}
// D-20261108-MC7: a real pointer move onto the words; the hover preview's own
// dwell must elapse first (default 1 s), so nothing is open shortly after.
async function hoverOpen(selector, text) {
  await browser.$(selector).moveTo();
  await pause(400);
  assert.equal(await browser.$(".comment-thread-popup").isExisting(), false, "Hover opens only after the preview delay");
  await browser.$(".comment-thread-popup").waitForDisplayed({ timeout: 5000 });
  if (text) await shownThread(text);
}
// A mouse click on highlighted words places the caret in the commented block and opens nothing.
async function clickPlacesCaret(id, file, expected) {
  await browser.$(highlight(id)).click();
  await browser.waitUntil(() => browser.execute((selector, parent) => document.querySelector(selector)?.closest(".ls-block")?.dataset.blockRef === parent, editorSelector, ids.parent),
    { timeout: 5000, timeoutMsg: "A click on a highlight must edit the commented block" });
  await pause(1600);
  assert.equal(await browser.$(".comment-thread-popup").isExisting(), false, "A click on a highlight opens no thread");
  await shot(stage.replaceAll(": ", "-").replaceAll(" ", "-").toLowerCase());
  await keys("Escape"); await browser.$(editorSelector).waitForExist({ reverse: true });
  assert.equal(fs.readFileSync(file, "utf8"), expected, "Placing the caret must not change the file");
}
async function datedReply(file, text) {
  const pattern = new RegExp(`    - ${text}\\n      tine.author:: Ada Test\\n      tine.date:: (\\d{4}-\\d\\d-\\d\\dT\\d\\d:\\d\\dZ)\\n`);
  const saved = await waitForFileText(file, value => pattern.test(value));
  return `    - ${text}\n      tine.author:: Ada Test\n      tine.date:: ${saved.match(pattern)[1]}\n`;
}
// GH #891 finding 1: keystroke-to-paint latency while typing in one ordinary sibling
// (round 2: or in a folded block, `target`).
async function siblingTyping(mode, tier, count, target = siblingId, measurement = "sibling typing") {
  await browser.$(`${row(target)} > .block-main .block-content`).click();
  await browser.$(editorSelector).waitForExist();
  await keys("End");
  await browser.execute(() => {
    const p = window.__siblingTyping = { paint: [] };
    p.key = event => { if (event.key !== "x") return; const at = performance.now(); requestAnimationFrame(() => setTimeout(() => p.paint.push(performance.now() - at), 0)); };
    document.addEventListener("keydown", p.key, true);
  });
  await type("x".repeat(count));
  await browser.executeAsync(done => requestAnimationFrame(() => setTimeout(done, 0)));
  const paint = await browser.execute(() => { const p = window.__siblingTyping; document.removeEventListener("keydown", p.key, true); return p.paint; });
  await keys("Escape");
  const sorted = [...paint].sort((a, b) => a - b), p95 = sorted[Math.ceil(sorted.length * 0.95) - 1];
  const result = { mode, tier, case: measurement, siblings: SIBLINGS, samples: paint.length, p50: sorted[Math.floor(sorted.length / 2)], p95, load: fs.readFileSync("/proc/loadavg", "utf8").trim() };
  proof.typing.push(result); console.log(JSON.stringify({ measurement, ...result }));
  assert.equal(paint.length, count);
}
async function commentsDisplay(choice) {
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-search-input").setValue("Comments");
  await browser.$('[data-setting-target="settings-comments-display"]').click();
  const select = await browser.$('[data-setting-id="settings-comments-display"] select');
  await select.waitForDisplayed(); await select.selectByAttribute("value", choice);
  assert.equal(await select.getValue(), choice);
  await shot(stage.replaceAll(": ", "-").replaceAll(" ", "-").toLowerCase() + "-" + choice);
  await keys("Escape");
}
async function readThreadTargets() {
  await browser.$('.page-blocks ' + row(ids.parent) + ' .comment-quote-anchor').waitForDisplayed();
  const targets = await browser.execute(parent => {
    const marks = [...document.querySelectorAll('.page-blocks .ls-block[data-block-ref="' + parent + '"] .comment-quote-anchor')];
    return { one: marks.find(mark => mark.textContent === "First")?.dataset.commentId,
      two: marks.find(mark => mark.textContent === "second")?.dataset.commentId };
  }, ids.parent);
  assert.ok(targets.one && targets.two, "Passage highlights identify both thread targets");
  return targets;
}
async function enterPhone() {
  await browser.setWindowSize(700, 1000);
  let steps = 0;
  while (steps++ < 10 && await browser.execute(() => innerWidth > 420)) {
    const before = await browser.execute(() => innerWidth); await keys("Control", "=");
    await browser.waitUntil(() => browser.execute(before => innerWidth < before, before));
  }
  assert.ok(await browser.execute(() => innerWidth <= 420));
  if (await browser.execute(() => !!document.querySelector("[data-mobile-drawer-scrim]"))) await keys("Escape");
}
async function layoutState() {
  return browser.execute(parent => ({ viewport: innerWidth, pane: document.querySelector(".main-content").clientWidth,
    inlineCards: document.querySelectorAll(".page-blocks .comment-card").length, margin: !!document.querySelector(".margin-column"),
    badges: document.querySelectorAll("button.margin-marker").length,
    expandedBadges: document.querySelectorAll('.ls-block[data-block-ref="' + parent + '"] > .block-main button.margin-marker').length,
    railMarkers: document.querySelectorAll(".comment-rail-marker").length,
    popover: !!document.querySelector(".comment-thread-popup"), sheet: !!document.querySelector(".comment-bottom-sheet") }), ids.parent);
}
async function reply(text, scaled = false) {
  const draft = ".comment-thread-popup [data-reply-root='" + runtimeIds.one + "']";
  if (scaled) await browser.execute(selector => document.querySelector(selector).focus(), draft);
  else await browser.$(draft).click();
  await browser.keys(text);
  const send = ".comment-thread-popup .comment-composer.expanded button.comment-reply";
  await browser.execute(selector => document.querySelector(selector).scrollIntoView({ block: "nearest" }), send);
  assert.ok(await browser.execute(selector => {
    const button = document.querySelector(selector), rect = button.getBoundingClientRect();
    return button.contains(document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2));
  }, send), "Send must remain visible and unobstructed");
  if (scaled) await shot(stage.split(":")[0] + "-phone-send");
  await activate(send, scaled);
}
async function resolutionJourney(file, expected, mode, tier, scaled = false) {
  stage = `${mode}: ${tier} Resolve and Reopen`;
  const parent = row(ids.parent) + " > .block-main";
  if (await browser.$(".comment-thread-popup").isExisting()) await closePopup(scaled);
  for (const id of [ids.one, ids.two]) {
    // Phones tap the count-free chip; medium panes hover the thread's words.
    if (scaled) await activate(parent + " .margin-marker", scaled);
    else await hoverOpen(highlight(id === ids.one ? runtimeIds.one : runtimeIds.two));
    await browser.$(".comment-thread-popup").waitForDisplayed();
    await activate(`.comment-thread-popup ${row(id)} > .comment-header .comment-resolve`, scaled);
    if (await browser.$(".comment-thread-popup").isExisting()) await closePopup(scaled);
  }
  const closed = expected.replaceAll("    tine.quote:: First\n", "    tine.quote:: First\n    tine.resolved:: true\n")
    .replaceAll("    tine.quote:: second\n", "    tine.quote:: second\n    tine.resolved:: true\n");
  const saved = await waitForFileText(file, text => (text.match(/tine.resolved:: true/g) || []).length === 2);
  assert.equal(saved, closed, "Resolve must preserve exact Markdown except the two root flags");
  assert.equal(await browser.$$(parent + " .margin-marker," + parent + " .comment-rail-marker," + parent + " .comment-quote-anchor").then(items => items.length), 0);
  if (await browser.$(".main-content .comment-resolved-toggle").getAttribute("aria-pressed") !== "true")
    await activate(".main-content .comment-resolved-toggle", scaled);
  assert.equal(await browser.$$(parent + " .margin-marker," + parent + " .comment-rail-marker," + parent + " .comment-quote-anchor").then(items => items.length), 0, "Reveal must not restore open entry points");
  for (const id of [ids.one, ids.two]) {
    await activate(parent + " .comment-resolved-entry", scaled);
    await browser.$(`.comment-thread-popup ${row(id)}.comment-resolved`).waitForDisplayed();
    assert.equal(await browser.$(`.comment-thread-popup ${row(id)} .comment-resolve`).getText(), "Reopen");
    if (id === ids.one) {
      assert.ok(await browser.$(`.comment-thread-popup ${row(id)} time`).isExisting(), "Dated replies must show their date in the compact card");
      await shot(`${mode}-${tier}-resolved`);
    }
    await activate(`.comment-thread-popup ${row(id)} > .comment-header .comment-resolve`, scaled);
    await closePopup(scaled);
  }
  await waitForFileText(file, text => text === expected);
  assert.equal(fs.readFileSync(file, "utf8"), expected, "Reopen must restore the exact original Markdown");
  await browser.$(".main-content .comment-resolved-toggle").waitForExist({ reverse: true });
  if (scaled) await activate(parent + " .margin-marker", scaled); else await hoverOpen(highlight(runtimeIds.one));
  await browser.$(".comment-thread-popup").waitForDisplayed();
  proof.outcomes.push({ mode, tier, resolve: true, reveal: true, explicitReopen: true, compactDate: true, exactMarkdown: true });
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser); await browser.$(".app-container").waitForExist({ timeout: 30000 }); await assertAppLease(fixture);
  await browser.$('button[title^="Settings"]').click(); await browser.$(".settings-search-input").setValue("Your name"); await browser.$(".settings-search-result").click();
  await browser.$('input[aria-label="Your name"]').setValue("Ada Test"); await keys("Enter"); await keys("Escape");
  for (const mode of modes) {
    if (mode === "cm6") ids = Object.fromEntries(Object.entries(ids).map(([key, id]) => [key, id.replace("01000000", "02000000")]));
    editorSelector = mode === "cm6" ? '[data-block-editor="cm6"] .cm-content' : "textarea.block-editor";
    const file = path.join(graph, `pages/Comments ${mode}.md`);
    const expectedSeed = mode === "cm6" ? seed.replaceAll("01000000", "02000000") : seed;
    await browser.setWindowSize(1700, 1000);
    stage = `${mode}: editor setup`;
    await browser.$('button[title^="Settings"]').click(); await browser.$(".settings-search-input").setValue("CodeMirror"); await browser.$(".settings-search-result").click();
    const toggle = await browser.$('[data-setting-label="Experimental: CodeMirror editor"] [role="switch"]');
    await toggle.waitForDisplayed(); if (await toggle.getAttribute("aria-checked") !== String(mode === "cm6")) await toggle.click();
    await keys("Escape");
    stage = `${mode}: sibling typing budget`;
    siblingId = `${mode === "cm6" ? "04" : "03"}000000-0000-4000-8000-000000000032`;
    const siblingsFile = path.join(graph, `pages/Siblings ${mode}.md`);
    await openPageByName(browser, `Siblings ${mode}`);
    await browser.$(".margin-column .comment-card").waitForDisplayed();
    await siblingTyping(mode, "wide", 12);
    await browser.setWindowSize(850, 1000);
    await browser.waitUntil(() => browser.execute(() => !document.querySelector(".margin-column")));
    await siblingTyping(mode, "medium", 12);
    const typed = siblingSeed(mode === "cm6" ? "04" : "03").replace(`  - Sibling ${TYPED}\n`, `  - Sibling ${TYPED}${"x".repeat(24)}\n`);
    await waitForFileText(siblingsFile, text => text === typed);
    assert.equal(fs.readFileSync(siblingsFile, "utf8"), typed, "Typing beside many siblings saves exact Markdown");
    stage = `${mode}: folded parent typing budget`;
    const foldedPrefix = mode === "cm6" ? "06" : "05", foldedId = `${foldedPrefix}000000-0000-4000-8000-000000000001`;
    const foldedFile = path.join(graph, `pages/Folded ${mode}.md`);
    await openPageByName(browser, `Folded ${mode}`);
    const below = await browser.$(`${row(foldedId)} > .block-main .comment-below-chip`);
    await below.waitForDisplayed(); assert.equal(await below.getText(), "1 below", "The folded block counts its hidden thread");
    await siblingTyping(mode, "medium", 12, foldedId, "folded parent typing");
    assert.equal(await below.getText(), "1 below", "Typing in the folded block keeps its count");
    const foldedTyped = foldedSeed(foldedPrefix).replace("- A folded block with many hidden children\n", `- A folded block with many hidden children${"x".repeat(12)}\n`);
    await waitForFileText(foldedFile, text => text === foldedTyped);
    assert.equal(fs.readFileSync(foldedFile, "utf8"), foldedTyped, "Typing in a folded block saves exact Markdown");
    stage = `${mode}: one passage among ${PASSAGES} opens alone`;
    const passagePrefix = mode === "cm6" ? "08" : "07", passageFile = path.join(graph, `pages/Passages ${mode}.md`);
    await openPageByName(browser, `Passages ${mode}`);
    // Highlights carry the runtime block id; pick passage p1q by its text.
    await browser.$(".page-blocks .comment-quote-anchor").waitForExist();
    const passageId = await browser.execute(() => [...document.querySelectorAll(".page-blocks .comment-quote-anchor")].find(mark => mark.textContent === "p1q")?.dataset.commentId);
    assert.ok(passageId, "Passage p1q's highlight identifies its thread");
    const passageMark = `.page-blocks .comment-quote-anchor[data-comment-id="${passageId}"]`;
    // Keyboard entry (Enter on the focused highlight) skips the hover dwell, so
    // the latency is the opening's own: keydown to the frame after the popup.
    await browser.execute(selector => {
      const p = window.__passageOpen = {};
      p.key = event => {
        if (event.key !== "Enter") return;
        const at = performance.now();
        const painted = () => requestAnimationFrame(() => setTimeout(() => { p.ms = performance.now() - at; }, 0));
        const observer = new MutationObserver(() => { if (document.querySelector(".comment-thread-popup")) { observer.disconnect(); painted(); } });
        observer.observe(document.body, { childList: true, subtree: true });
      };
      document.addEventListener("keydown", p.key, true);
      document.querySelector(selector).focus();
    }, passageMark);
    await keys("Enter");
    await browser.$(".comment-thread-popup").waitForDisplayed();
    await browser.waitUntil(() => browser.execute(() => window.__passageOpen.ms !== undefined));
    const opening = await browser.execute(() => {
      const p = window.__passageOpen; document.removeEventListener("keydown", p.key, true);
      const popup = document.querySelector(".comment-thread-popup");
      return { ms: p.ms, groups: popup.querySelectorAll(".comment-group").length, cards: popup.querySelectorAll("[data-block-ref]").length, text: popup.textContent };
    });
    const openingResult = { mode, tier: "medium", passages: PASSAGES, ms: opening.ms, groups: opening.groups, cards: opening.cards, load: fs.readFileSync("/proc/loadavg", "utf8").trim() };
    proof.opening.push(openingResult); console.log(JSON.stringify({ measurement: "open one passage", ...openingResult }));
    assert.ok(opening.text.includes("Thread on p1q"), "The opened passage shows its thread");
    assert.equal(opening.groups, 1, "Opening one passage mounts that passage's group only");
    assert.equal(opening.cards, 1, "Opening one passage mounts that passage's cards only");
    await closePopup();
    assert.equal(fs.readFileSync(passageFile, "utf8"), passageSeed(passagePrefix), "Opening a passage changes nothing on disk");
    await browser.setWindowSize(1700, 1000);
    await openPageByName(browser, `Comments ${mode}`);
    stage = `${mode}: wide`;
    await browser.$(".margin-column .comment-card").waitForDisplayed();
    runtimeIds = await readThreadTargets();
    if (process.env.MC010_LAYOUT_PROBE === "1") {
      stage = `${mode}: baseline layout probe`;
      const observations = [{ tier: "wide", ...await layoutState() }];
      await shot(`${mode}-before-wide`); await browser.setWindowSize(850, 1000);
      await browser.waitUntil(() => browser.execute(() => document.querySelector(".main-content").clientWidth < 960));
      await browser.$(".page-blocks .comment-quote-anchor").moveTo(); await pause(1500);
      const medium = await layoutState();
      medium.highlightEditsParentInstead = await browser.execute(id => document.querySelector("textarea.block-editor,[data-block-editor=cm6]")?.closest(".ls-block")?.dataset.blockRef === id, ids.parent);
      observations.push({ tier: "medium", ...medium });
      for (let i = 0; i < 3 && await browser.$(editorSelector).isExisting(); i++) await keys("Escape");
      await shot(`${mode}-before-medium`); await enterPhone();
      if (await browser.$(".page-blocks button.margin-marker").isExisting()) await activate(".page-blocks button.margin-marker", true);
      observations.push({ tier: "phone", ...await layoutState() }); await shot(`${mode}-before-phone`);
      assert.equal(fs.readFileSync(file, "utf8"), expectedSeed, "Baseline browsing must preserve exact Markdown");
      await keys("Control", "0"); proof.outcomes.push({ mode, observations }); continue;
    }
    assert.equal(await browser.$$(row(ids.parent) + " > .block-main button.margin-marker").then(items => items.length), 0);
    assert.equal(fs.readFileSync(file, "utf8"), expectedSeed);
    await shot(`${mode}-01-wide`);
    stage = `${mode}: wide In a popup setting`;
    await commentsDisplay("popup");
    await browser.$(highlight(runtimeIds.one)).waitForDisplayed();
    assert.equal(await browser.execute(() => document.querySelectorAll(".margin-column,.page-blocks .comment-card,.margin-marker,.comment-rail-marker").length), 0,
      "In a popup draws no margin, cards, markers or counts");
    await browser.refresh(); await browser.$(".app-container").waitForExist({ timeout: 30000 });
    await openPageByName(browser, `Comments ${mode}`);
    await browser.$('.page-blocks ' + row(ids.parent)).waitForDisplayed();
    runtimeIds = await readThreadTargets();
    assert.equal(await browser.execute(() => document.querySelectorAll(".margin-column,.margin-marker").length), 0, "In a popup survives reload");
    stage = `${mode}: wide In a popup hover`;
    await hoverOpen(highlight(runtimeIds.one), "First thread");
    assert.equal(await browser.$(".comment-bottom-sheet").isExisting(), false);
    await shot(`${mode}-wide-popup-hover`);
    await reply("Popup reply");
    let tail = "    - An existing reply\n", added = await datedReply(file, "Popup reply");
    let expected = expectedSeed.replace(tail, tail + added); tail = added;
    await waitForFileText(file, text => text === expected);
    assert.equal(fs.readFileSync(file, "utf8"), expected, "A hovered popover reply saves exact Markdown");
    assert.ok(await browser.execute(() => document.querySelector(".comment-thread-popup")?.textContent.includes("Popup reply")), "The hovered popover stays live after replying");
    await shot(`${mode}-wide-popup-replied`); await closePopup();
    stage = `${mode}: wide In a popup leave closes`;
    await hoverOpen(highlight(runtimeIds.two), "Second thread");
    await browser.$(".main-content h1.page-title").moveTo(); await browser.$(".comment-thread-popup").waitForExist({ reverse: true, timeout: 3000 });
    stage = `${mode}: wide In a popup click places caret`;
    await clickPlacesCaret(runtimeIds.two, file, expected);
    await commentsDisplay("margin");
    await browser.$(".margin-column .comment-card").waitForDisplayed(); await shot(`${mode}-wide-margin-restored`);
    stage = `${mode}: medium hover`;
    // Below margin fit even after phone layout closes the left sidebar.
    await browser.setWindowSize(850, 1000);
    await browser.waitUntil(() => browser.execute(() => !document.querySelector(".margin-column")));
    await browser.$(highlight(runtimeIds.one)).waitForDisplayed();
    assert.equal(await browser.execute(() => document.querySelectorAll(".page-blocks .comment-card,.margin-column,.comment-rail-marker,.margin-marker").length), 0,
      "Medium panes draw no inline cards, rail markers or counted chips");
    await hoverOpen(highlight(runtimeIds.one), "First thread"); await reply("Medium reply");
    added = await datedReply(file, "Medium reply"); expected = expected.replace(tail, tail + added); tail = added;
    await waitForFileText(file, text => text === expected);
    await resolutionJourney(file, expected, mode, "medium");
    await shot(`${mode}-02-medium-popover`); await closePopup();
    stage = `${mode}: medium highlight`;
    await hoverOpen(highlight(runtimeIds.two), "Second thread");
    await shot(`${mode}-03-medium-highlight`); await closePopup();
    stage = `${mode}: medium click places caret`;
    await clickPlacesCaret(runtimeIds.one, file, expected);
    stage = `${mode}: folded own highlight`;
    await hoverOpen(row(ids.folded) + " > .block-main .comment-quote-anchor");
    assert.ok(await browser.execute(() => document.querySelector(".comment-thread-popup").textContent.includes("Own folded thread")));
    await closePopup();
    stage = `${mode}: phone`;
    await commentsDisplay("popup");
    await enterPhone();
    await activate('.page-blocks [data-comment-id="' + runtimeIds.one + '"]', true);
    await browser.$(".comment-bottom-sheet").waitForDisplayed(); await reply("Phone reply", true);
    added = await datedReply(file, "Phone reply"); expected = expected.replace(tail, tail + added); tail = added;
    await waitForFileText(file, text => text === expected);
    await resolutionJourney(file, expected, mode, "phone", true);
    const geometry = await browser.execute(() => {
      const sheet = document.querySelector(".comment-bottom-sheet").getBoundingClientRect();
      const source = document.querySelector(".page-blocks .comment-quote-anchor").getBoundingClientRect();
      return { width: innerWidth, sheetTop: sheet.top, sheetBottom: sheet.bottom, height: innerHeight, sourceBottom: source.bottom };
    });
    assert.ok(geometry.sourceBottom <= geometry.sheetTop, "Source text must remain visible above the sheet");
    await shot(`${mode}-04-phone-sheet`);
    await activate(".comment-next-thread", true);
    assert.ok(await browser.execute(() => [...document.querySelectorAll(".comment-thread-popup .comment-group")].filter(el => !el.closest("[hidden]")).some(el => el.textContent.includes("Second thread"))));
    await shot(`${mode}-05-phone-next`);
    const syntheticSwipe = await browser.execute(() => {
      const target = document.querySelector(".comment-popup-toolbar");
      for (const [type, x] of [["touchstart", 40], ["touchend", 190]]) {
        const event = new Event(type, { bubbles: true, cancelable: true });
        const point = { clientX: x, clientY: 30 };
        Object.defineProperties(event, { touches: { value: type === "touchend" ? [] : [point] }, changedTouches: { value: [point] } }); target.dispatchEvent(event);
      }
      return [...document.querySelectorAll(".comment-thread-popup .comment-group")].filter(el => !el.closest("[hidden]")).some(el => el.textContent.includes("First thread"));
    });
    assert.ok(syntheticSwipe, "A rightward swipe must return to the previous passage");
    await closePopup(true);
    assert.equal(await browser.$(row(ids.parent) + " > .block-main .margin-marker").getText(), "", "The phone chip shows no count");
    await activate(row(ids.parent) + " > .block-main .margin-marker", true);
    await browser.$(".comment-bottom-sheet").waitForDisplayed(); await closePopup(true);
    await keys("Control", "0"); await browser.waitUntil(() => browser.execute(() => innerWidth > 420));
    await commentsDisplay("margin");
    assert.equal(fs.readFileSync(file, "utf8"), expected, "Display changes and thread browsing preserve exact Markdown");
    proof.outcomes.push({ mode, exactMarkdown: true, wide: true, popupHoverReply: true, popupLeaveCloses: true, clickPlacesCaret: true, displaySurvivesReload: true, marginRestored: true, mediumHover: true, mediumHighlight: true, noRailOrCount: true, phonePopup: true, phoneHighlight: true, phoneChipWithoutCount: true, foldedOwnHighlight: true, syntheticSwipe, geometry });
  }
  fixture.verify();
  stage = "sibling typing budget";
  const overBudget = proof.typing.filter(result => !(result.p95 < TYPING_P95_MS));
  assert.deepEqual(overBudget, [], `Typing p95 beside ${SIBLINGS} siblings or above ${SIBLINGS} hidden children must stay under ${TYPING_P95_MS} ms`);
  if (process.env.MC010_LAYOUT_PROBE === "1") {
    stage = "baseline width assertions";
    const deviations = proof.outcomes.flatMap(({ mode, observations }) => observations.flatMap(state => {
      const missing = state.tier === "wide" ? state.expandedBadges > 0
        : state.tier === "medium" ? state.inlineCards > 0 || state.railMarkers > 0 || !state.popover
        : state.inlineCards > 0 || !state.sheet;
      return missing ? [{ mode, ...state }] : [];
    }));
    assert.deepEqual(deviations, [], "Width-only live thread entry is missing at these widths");
  }
  fs.writeFileSync(path.join(out, "receipt.json"), JSON.stringify({ outcome: "PASS", ...proof }, null, 2)); console.log(`PASS ${out}`);
} catch (error) {
  try { proof.dom = await browser.execute(() => ({ width: innerWidth, panes: [...document.querySelectorAll(".main-content")].map(el => el.clientWidth),
    markers: [...document.querySelectorAll(".comment-quote-anchor,.margin-marker")].map(el => ({ id: el.dataset.commentId, rect: el.getBoundingClientRect().toJSON(),
      chain: [...(function* () { for (let at = el; at && at !== document.body; at = at.parentElement) yield at; })()].map(at => ({ cls: at.className, display: getComputedStyle(at).display, visibility: getComputedStyle(at).visibility, rect: at.getBoundingClientRect().toJSON() })) })) })); } catch { /* retain original error */ }
  try { await shot("failure"); } catch { /* preserve original error */ }
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage, expected: "Viewer display choice and pane tiers open live threads and preserve exact saved Markdown", observed: error.stack, classification: error.code === "ERR_ASSERTION" ? "product" : "ambiguous harness/infrastructure" }, null, 2)); throw error;
} finally {
  try { await browser?.deleteSession(); } catch { /* session may have failed */ }
  try { process.kill(-driver.pid, "SIGTERM"); } catch { /* driver may have exited */ }
  fs.closeSync(log); fixture.close?.();
}
