// D-20261107-MC3 §§6–7: native routed-page proof in both editor engines.
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
const proof = { base, delta: "working tree", app, modes, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"), outcomes: [], focusProbes: [], controlProbes: [], menuProbes: [], screenshots: [] };
let browser, stage = "startup", editorSelector, runtimeIds;
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
    await activate(parent + " .margin-marker", scaled);
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
  await activate(parent + " .margin-marker", scaled);
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
    await keys("Escape"); await openPageByName(browser, `Comments ${mode}`);
    stage = `${mode}: wide`;
    await browser.$(".margin-column .comment-card").waitForDisplayed();
    runtimeIds = await browser.execute(ids => Object.fromEntries(Object.entries(ids).map(([key, id]) => [key,
      document.querySelector('.ls-block[data-block-ref="' + id + '"]')?.dataset.blockId])), ids);
    if (process.env.MC010_LAYOUT_PROBE === "1") {
      stage = `${mode}: baseline layout probe`;
      const observations = [{ tier: "wide", ...await layoutState() }];
      await shot(`${mode}-before-wide`); await browser.setWindowSize(850, 1000);
      await browser.waitUntil(() => browser.execute(() => document.querySelector(".main-content").clientWidth < 960));
      await browser.$(".page-blocks .comment-quote-anchor").click();
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
    stage = `${mode}: medium marker`;
    // Below margin fit even after phone layout closes the left sidebar.
    await browser.setWindowSize(850, 1000);
    await browser.$('[data-comment-marker="' + runtimeIds.one + '"]').waitForDisplayed();
    assert.equal(await browser.execute(() => document.querySelectorAll(".page-blocks .comment-card,.margin-column").length), 0);
    await activate('[data-comment-marker="' + runtimeIds.one + '"]');
    await browser.$(".comment-thread-popup").waitForDisplayed(); await reply("Medium reply");
    const mediumSaved = await waitForFileText(file, text => text.includes("Medium reply"));
    const mediumDate = mediumSaved.match(/tine.date:: (\d{4}-\d\d-\d\dT\d\d:\d\dZ)/)?.[1];
    assert.ok(mediumDate, "Medium reply must carry a UTC minute creation date");
    let expected = expectedSeed.replace("    - An existing reply\n", `    - An existing reply\n    - Medium reply\n      tine.author:: Ada Test\n      tine.date:: ${mediumDate}\n`);
    await waitForFileText(file, text => text === expected);
    await resolutionJourney(file, expected, mode, "medium");
    await shot(`${mode}-02-medium-popover`); await closePopup();
    stage = `${mode}: medium highlight`;
    await activate('.page-blocks [data-comment-id="' + runtimeIds.two + '"]');
    await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".comment-thread-popup [data-block-ref]")].some(el => !el.closest("[hidden]") && el.textContent.includes("Second thread"))));
    await shot(`${mode}-03-medium-highlight`); await closePopup();
    stage = `${mode}: folded own chip`;
    await activate(row(ids.folded) + " > .block-main .margin-marker");
    await browser.$(".comment-thread-popup").waitForDisplayed();
    assert.ok(await browser.execute(() => document.querySelector(".comment-thread-popup").textContent.includes("Own folded thread")));
    await closePopup();
    stage = `${mode}: phone`;
    await enterPhone();
    await activate('.page-blocks [data-comment-id="' + runtimeIds.one + '"]', true);
    await browser.$(".comment-bottom-sheet").waitForDisplayed(); await reply("Phone reply", true);
    const phoneSaved = await waitForFileText(file, text => text.includes("Phone reply"));
    const phoneDate = phoneSaved.match(/- Phone reply\n      tine.author:: Ada Test\n      tine.date:: (\d{4}-\d\d-\d\dT\d\d:\d\dZ)/)?.[1];
    assert.ok(phoneDate, "Phone reply must carry a UTC minute creation date");
    expected = expected.replace(`      tine.date:: ${mediumDate}\n`, `      tine.date:: ${mediumDate}\n    - Phone reply\n      tine.author:: Ada Test\n      tine.date:: ${phoneDate}\n`);
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
    await activate(row(ids.parent) + " > .block-main .margin-marker", true);
    await browser.$(".comment-bottom-sheet").waitForDisplayed(); await closePopup(true);
    await keys("Control", "0"); await browser.waitUntil(() => browser.execute(() => innerWidth > 420));
    proof.outcomes.push({ mode, exactMarkdown: true, wide: true, mediumMarker: true, mediumHighlight: true, phoneHighlight: true, phoneChip: true, foldedOwnChip: true, syntheticSwipe, geometry });
  }
  fixture.verify();
  if (process.env.MC010_LAYOUT_PROBE === "1") {
    stage = "baseline width assertions";
    const deviations = proof.outcomes.flatMap(({ mode, observations }) => observations.flatMap(state => {
      const missing = state.tier === "wide" ? state.expandedBadges > 0
        : state.tier === "medium" ? state.inlineCards > 0 || state.railMarkers === 0 || !state.popover
        : state.inlineCards > 0 || !state.sheet;
      return missing ? [{ mode, ...state }] : [];
    }));
    assert.deepEqual(deviations, [], "Width-only live thread entry is missing at these widths");
  }
  fs.writeFileSync(path.join(out, "receipt.json"), JSON.stringify({ outcome: "PASS", ...proof }, null, 2)); console.log(`PASS ${out}`);
} catch (error) {
  try { proof.dom = await browser.execute(() => ({ width: innerWidth, panes: [...document.querySelectorAll(".main-content")].map(el => el.clientWidth),
    markers: [...document.querySelectorAll(".comment-rail-marker")].map(el => ({ id: el.dataset.commentMarker, rect: el.getBoundingClientRect().toJSON(),
      chain: [...(function* () { for (let at = el; at && at !== document.body; at = at.parentElement) yield at; })()].map(at => ({ cls: at.className, display: getComputedStyle(at).display, visibility: getComputedStyle(at).visibility, rect: at.getBoundingClientRect().toJSON() })) })) })); } catch { /* retain original error */ }
  try { await shot("failure"); } catch { /* preserve original error */ }
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage, expected: "Width-only live threads and exact saved replies", observed: error.stack, classification: error.code === "ERR_ASSERTION" ? "product" : "ambiguous harness/infrastructure" }, null, 2)); throw error;
} finally {
  try { await browser?.deleteSession(); } catch { /* session may have failed */ }
  try { process.kill(-driver.pid, "SIGTERM"); } catch { /* driver may have exited */ }
  fs.closeSync(log); fixture.close?.();
}
