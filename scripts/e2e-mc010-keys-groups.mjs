// D-20261107-MC3 §§4,5,7,9: native routed-page proof in both editor engines.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import crypto from "node:crypto";
import assert from "node:assert/strict";
import { remote } from "webdriverio";
import { chooseEditor } from "./lib/e2e-editor-choice.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { waitForFileText } from "./e2e-file-poll.mjs";

const root = path.resolve(import.meta.dirname, "..");
const modes = process.env.MC010_EDITOR ? [process.env.MC010_EDITOR] : ["textarea", "cm6"];
assert.ok(modes.every(mode => mode === "textarea" || mode === "cm6"), "MC010_EDITOR must be textarea or cm6");
const artifacts = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/og-mc010-keys-groups");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
let ids = { parent: "01000000-0000-4000-8000-000000000001", one: "01000000-0000-4000-8000-000000000002",
  two: "01000000-0000-4000-8000-000000000003", folded: "01000000-0000-4000-8000-000000000004" };
const seed = `- Words to discuss today.
  id:: ${ids.parent}
  - First thread
    id:: ${ids.one}
    tine.quote:: discuss
    - An existing reply
  - Second thread
    id:: ${ids.two}
    tine.quote:: discuss
- Collapsible parent
  id:: ${ids.folded}
  collapsed:: true
  - Hidden body
    - Hidden thread
      tine.quote:: body
      - Reply never counted
        - Hand nested reply
          tine.quote:: Reply
    - Another hidden thread
      tine.quote:: Hidden
- Body Enter sentinel
`;
for (const mode of ["textarea", "cm6"]) fs.writeFileSync(path.join(graph, `pages/Comments ${mode}.md`), mode === "cm6" ? seed.replaceAll("01000000", "02000000") : seed);
for (const count of [1, 60]) for (const format of ["md", "org"]) fs.writeFileSync(path.join(graph, `pages/Cost ${count} ${format}.${format}`),
  (format === "md" ? "- Measure passage\n  Another visible line.\n  - Cost thread\n    tine.quote:: visible\n" : "* Measure passage\nAnother visible line.\n** Cost thread\n:PROPERTIES:\n:tine.quote: visible\n:END:\n")
  + Array.from({ length: count - 1 }, (_, i) => `${format === "md" ? "-" : "*"} Cost filler ${i + 1}\n`).join(""));
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
let browser, stage = "startup", editorSelector;
const row = id => `.main-content .ls-block[data-block-ref="${id}"]`;
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
async function menuToggle(label, scaled = false) {
  await browser.execute(() => {
    window.__mcMenuClick = null;
    document.addEventListener("click", e => { window.__mcMenuClick = { x: e.clientX, y: e.clientY, target: e.target?.outerHTML?.slice(0, 400) }; }, { once: true, capture: true });
  });
  // Under interface zoom, this WebKitWebDriver delivered its pointer click
  // outside the measured button. Keep the real visible control and activate it
  // with native Enter after focusing it, avoiding the driver's coordinates.
  if (scaled) {
    await browser.execute(() => document.querySelector(".main-content [data-page-actions-trigger]").focus());
    await keys("Enter");
  } else await browser.$(".main-content [data-page-actions-trigger]").click();
  proof.controlProbes.push(await browser.execute(() => {
    const trigger = document.querySelector(".main-content [data-page-actions-trigger]");
    return { focused: document.activeElement === trigger, focusVisible: trigger.matches(":focus-visible"), opacity: getComputedStyle(trigger).opacity,
      active: document.activeElement?.outerHTML?.slice(0, 300), width: innerWidth };
  }));
  await browser.$(".main-content [data-page-actions-trigger]").waitForDisplayed();
  proof.menuProbes.push(await browser.execute(() => {
    const trigger = document.querySelector(".main-content [data-page-actions-trigger]"), rect = trigger.getBoundingClientRect();
    return { width: innerWidth, height: innerHeight, ratio: devicePixelRatio, click: window.__mcMenuClick,
      trigger: { x: rect.x, y: rect.y, width: rect.width, height: rect.height, expanded: trigger.getAttribute("aria-expanded") },
      menu: !!document.querySelector(".ctx-menu"), action: !!document.querySelector('[data-page-action-id="comment-visibility"]') };
  }));
  const action = await browser.$('[data-page-action-id="comment-visibility"]');
  await action.waitForDisplayed(); assert.equal(await action.getText(), label);
  if (scaled) {
    if (label === "Hide comments") await shot(`${stage.split(":")[0]}-03-phone-menu`);
    await browser.execute(() => document.querySelector('[data-page-action-id="comment-visibility"]').focus());
    await keys("Enter");
  } else await action.click();
}
async function resolutionJourney(file, mode) {
  const original = fs.readFileSync(file, "utf8");
  const resolved = original.replaceAll("    tine.quote:: discuss\n", "    tine.quote:: discuss\n    tine.resolved:: true\n");
  stage = `${mode}: resolve entire grouped threads`;
  assert.equal(await browser.execute(() => document.querySelectorAll(".comment-date").length), 0, "Legacy comments have no fabricated date");
  for (const id of [ids.one, ids.two]) {
    await browser.$(`${row(id)} > .comment-header .comment-resolve`).click();
    await browser.$(row(id)).waitForExist({ reverse: true });
  }
  const closed = await waitForFileText(file, text => (text.match(/tine.resolved:: true/g) || []).length === 2);
  assert.equal(closed, resolved, "Resolve must change only the root's one property line in the exact saved Markdown");
  assert.equal(await browser.execute(() => document.querySelectorAll(".comment-quote-anchor").length), 0, "Resolved passages retain no highlight");
  const reveal = await browser.$(".main-content .comment-resolved-toggle");
  assert.equal(await reveal.getText(), "2 resolved"); await reveal.click();
  await browser.$(`${row(ids.two)}.comment-resolved`).waitForExist();
  assert.equal(await browser.$(`${row(ids.one)} > .comment-header .comment-resolve`).getText(), "Reopen");
  assert.equal(await browser.execute(() => document.querySelectorAll(".comment-quote-anchor").length), 0, "Revealing must not restore highlights");
  await shot(`${mode}-05-resolved-revealed`);
  stage = `${mode}: reply reopens a revealed thread`;
  await browser.$(row(ids.two)).moveTo();
  const draft = await browser.$(`${row(ids.two)} > .comment-composer textarea`);
  await draft.click(); await browser.keys("Reopen by reply");
  await browser.$(`${row(ids.two)} > .comment-composer .comment-reply`).click();
  const replied = await waitForFileText(file, text => text.includes("Reopen by reply") && (text.match(/tine.resolved:: true/g) || []).length === 1);
  const date = replied.match(/tine.date:: (\d{4}-\d\d-\d\dT\d\d:\d\dZ)/)?.[1];
  assert.ok(date, "The new reply must carry a UTC minute creation date");
  const expectedReply = original.replace("    tine.quote:: discuss\n", "    tine.quote:: discuss\n    tine.resolved:: true\n")
    .replace(`    id:: ${ids.two}\n    tine.quote:: discuss\n`, `    id:: ${ids.two}\n    tine.quote:: discuss\n    - Reopen by reply\n      tine.author:: Ada Test\n      tine.date:: ${date}\n`);
  assert.equal(replied, expectedReply, "Reply must reopen only its own root and save the exact new message and metadata");
  assert.equal(await reveal.getText(), "1 resolved");
  assert.equal(await browser.$(`${row(ids.two)} > .comment-header .comment-resolve`).getText(), "Resolve");
  await browser.$(`${row(ids.two)} time[datetime="${date}"]`).waitForExist();
  stage = `${mode}: explicit Reopen`;
  await browser.$(`${row(ids.one)} > .comment-header .comment-resolve`).click();
  await reveal.waitForExist({ reverse: true });
  await waitForFileText(file, text => !text.includes("tine.resolved"));
  await shot(`${mode}-06-reopened-with-date`);
  proof.outcomes.push({ mode, resolve: true, reveal: true, reopenByReply: true, explicitReopen: true, exactSavedMarkdown: true, replyDate: date });
}
async function measureUnitCosts() {
  proof.unitCosts = [];
  const hashes = () => new Map(fs.readdirSync(graph, { recursive: true }).flatMap(name => {
    const file = path.join(graph, name);
    return fs.statSync(file).isFile() ? [[name, crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex")]] : [];
  }));
  const changes = before => {
    const after = hashes();
    return { changed: [...after.keys()].filter(name => before.get(name) !== after.get(name)), added: [...after.keys()].filter(name => !before.has(name)) };
  };
  await browser.setWindowSize(1700, 1000);
  for (const count of [1, 60]) for (const format of ["md", "org"]) {
    stage = `unit cost: ${count} ${format} body blocks`;
    const file = path.join(graph, `pages/Cost ${count} ${format}.${format}`);
    const before = fs.readFileSync(file, "utf8");
    await openPageByName(browser, `Cost ${count} ${format}`);
    const beforeResolve = hashes();
    const action = await browser.$(".main-content .comment-resolve"); await action.waitForDisplayed(); await action.click();
    const resolved = await waitForFileText(file, text => text.includes("tine.resolved"));
    const resolveFiles = changes(beforeResolve);
    assert.deepEqual(resolveFiles, { changed: [path.relative(graph, file)], added: [] });
    assert.equal(resolved, format === "md" ? before.replace("    tine.quote:: visible\n", "    tine.quote:: visible\n    tine.resolved:: true\n")
      : before.replace(":tine.quote: visible\n", ":tine.quote: visible\n:tine.resolved: true\n"));
    await browser.$(".main-content .comment-resolved-toggle").click();
    await browser.$(".main-content .comment-card.comment-resolved").waitForExist();
    await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
    const placement = await browser.execute(() => {
      const thread = document.querySelector(".main-content .margin-thread"), content = document.querySelector(".main-content .page-blocks .block-content");
      const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
      for (let node; (node = walker.nextNode());) {
        const at = node.textContent.indexOf("visible");
        if (at < 0) continue;
        const range = document.createRange(); range.setStart(node, at); range.setEnd(node, at + 7);
        const passage = range.getBoundingClientRect();
        return { card: thread.getBoundingClientRect().top, passage: passage.top, lineHeight: parseFloat(getComputedStyle(content).lineHeight), marks: content.querySelectorAll(".comment-quote-anchor").length };
      }
      return null;
    });
    (proof.unitCostPlacements ??= []).push({ count, format, ...placement });
    assert.ok(placement && Math.abs(placement.card - placement.passage) <= placement.lineHeight, "Revealed resolved card must remain at its later-line passage");
    assert.equal(placement.marks, 0);
    await shot(`cost-${count}-${format}-resolved-placement`);
    const draft = await browser.$(".main-content .comment-reply-draft"); await draft.waitForExist();
    await browser.execute(() => document.querySelector(".main-content .comment-reply-draft").focus());
    const beforeReply = hashes();
    await browser.keys("Cost reply"); await keys("Control", "Enter");
    const replied = await waitForFileText(file, text => text.includes("Cost reply") && !text.includes("tine.resolved"));
    const replyFiles = changes(beforeReply);
    assert.deepEqual(replyFiles, { changed: [path.relative(graph, file)], added: [] });
    const dateLine = replied.match(format === "md" ? /^ *tine.date:: \d{4}-\d\d-\d\dT\d\d:\d\dZ\n/m : /^:tine.date: \d{4}-\d\d-\d\dT\d\d:\d\dZ\n/m)?.[0];
    assert.ok(dateLine);
    proof.unitCosts.push({ format, bodyBlocks: count, existingThreadRoots: 1, beforeFileBytes: Buffer.byteLength(before),
      resolvedFileBytes: Buffer.byteLength(resolved), resolveDeltaBytes: Buffer.byteLength(resolved) - Buffer.byteLength(before),
      replyFileBytes: Buffer.byteLength(replied), creationDateBytes: Buffer.byteLength(dateLine),
      affectedGraphFiles: resolveFiles.changed.length, addedGraphFiles: resolveFiles.added.length,
      replyAffectedGraphFiles: replyFiles.changed.length, replyAddedGraphFiles: replyFiles.added.length, addedTransportBytes: 0 });
  }
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true, "tauri:options": { application: app } } });
  await ensureMainWindow(browser); await browser.$(".app-container").waitForExist({ timeout: 30000 }); await assertAppLease(fixture);
  // The scratch graph is inside this checkout's git work tree. Choose a name
  // through Settings so the unrelated first-comment prompt cannot intercept Send.
  stage = "choose comment name";
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-search-input").setValue("Your name"); await browser.$(".settings-search-result").click();
  await browser.$('input[aria-label="Your name"]').setValue("Ada Test"); await keys("Enter"); await keys("Escape");
  for (const mode of modes) {
    if (mode === "cm6") ids = Object.fromEntries(Object.entries(ids).map(([key, id]) => [key, id.replace("01000000", "02000000")]));
    editorSelector = mode === "cm6" ? '[data-block-editor="cm6"] .cm-content' : "textarea.block-editor";
    const file = path.join(graph, `pages/Comments ${mode}.md`);
    await browser.setWindowSize(1700, 1000);
    stage = `${mode}: choose editor`;
    await chooseEditor(browser, mode);
    await openPageByName(browser, `Comments ${mode}`);
    stage = `${mode}: grouped card and collapsed count`;
    await browser.$(".margin-column .comment-group").waitForExist({ timeout: 10000 });
    const initial = await browser.execute(ids => {
      const first = document.querySelector('.main-content [data-block-ref="' + ids.one + '"]');
      const group = first.closest(".comment-group");
      return { quotes: group.querySelectorAll(".comment-quote").length,
        roots: group.querySelectorAll(".comment-card").length, composers: group.querySelectorAll(".comment-reply-draft").length,
        below: document.querySelector(".comment-below-chip")?.textContent };
    }, ids);
    assert.deepEqual(initial, { quotes: 1, roots: 2, composers: 2, below: "2 below" });
    assert.equal(fs.readFileSync(file, "utf8"), mode === "cm6" ? seed.replaceAll("01000000", "02000000") : seed, "Reading groups must preserve file bytes");
    await shot(`${mode}-01-group-and-below`);
    await resolutionJourney(file, mode);
    stage = `${mode}: editing Enter stays in comment`;
    await browser.$(`${row(ids.one)} > .block-main .block-content`).click();
    await browser.$(editorSelector).waitForDisplayed(); await keys("Control", "End");
    await keys("Enter"); await keys("Shift", "Enter"); await type("More lines");
    assert.equal(await value(), "First thread\n\nMore lines"); await keys("Control", "Enter");
    await browser.$(editorSelector).waitForExist({ reverse: true });
    await waitForFileText(file, text => text.includes("First thread\n    \n    More lines") || /First thread\n\s*\n\s*More lines/.test(text));
    assert.equal((fs.readFileSync(file, "utf8").match(/^\s*tine.quote::/gm) || []).length, 5, "Enter created a thread or split metadata");
    stage = `${mode}: reply draft newline then Send`;
    await browser.$(row(ids.two)).moveTo();
    const draft = await browser.$(`${row(ids.two)} > .comment-composer textarea`);
    await draft.click(); await browser.keys("Draft first"); await keys("Enter"); await browser.keys("Draft second");
    assert.ok(!fs.readFileSync(file, "utf8").includes("Draft first"), "Plain Enter submitted the reply");
    const send = await browser.$(`${row(ids.two)} .comment-composer button.comment-reply`);
    assert.equal(await send.getText(), "Send"); await send.click();
    await waitForFileText(file, text => /- Draft first\n\s*Draft second/.test(text));
    stage = `${mode}: commenting on same passage focuses composer`;
    await browser.$(`${row(ids.parent)} > .block-main .block-content`).click(); await browser.$(editorSelector).waitForDisplayed();
    await keys("Control", "Home"); for (let i = 0; i < 9; i++) await keys("ArrowRight");
    for (let i = 0; i < 7; i++) await keys("Shift", "ArrowRight");
    const before = fs.readFileSync(file, "utf8");
    if (process.env.MC010_FOCUS_PROBE === "1") proof.focusProbes.push(await browser.execute(selector => {
      window.__mcFocusTrace = [];
      const snapshot = el => ({ tag: el?.tagName, cls: el?.className, reply: el?.dataset?.replyRoot, editor: el?.dataset?.cm6Editor });
      for (const el of document.querySelectorAll(".comment-reply-draft")) {
        const focus = el.focus;
        el.focus = function (...args) {
          window.__mcFocusTrace.push({ call: snapshot(this), display: getComputedStyle(this.parentElement).display, expanded: this.parentElement.className });
          const result = focus.apply(this, args);
          window.__mcFocusTrace.push({ after: snapshot(document.activeElement) });
          return result;
        };
      }
      for (const type of ["focusin", "focusout"]) document.addEventListener(type, event => { if (window.__mcFocusTrace.length < 40) window.__mcFocusTrace.push({ type, target: snapshot(event.target), related: snapshot(event.relatedTarget) }); }, true);
      return { mode: selector, active: snapshot(document.activeElement), selection: getSelection()?.toString(), editor: !!document.querySelector(selector) };
    }, editorSelector));
    await keys("Control", "r");
    await browser.waitUntil(() => browser.execute(() => document.activeElement?.matches("textarea.comment-reply-draft")), { timeout: 5000 });
    assert.equal(fs.readFileSync(file, "utf8"), before, "Commenting again wrote another thread");
    stage = `${mode}: explicit New thread`;
    await browser.$(`${row(ids.two)}`).moveTo();
    await browser.$(".margin-column .comment-new-thread").click(); await browser.$(editorSelector).waitForDisplayed();
    await type("Explicit thread"); await keys("Escape");
    const afterNewThread = await waitForFileText(file, text => text.includes("Explicit thread"));
    const newRoot = afterNewThread.slice(afterNewThread.indexOf("Explicit thread"), afterNewThread.indexOf("Collapsible parent"));
    assert.ok(newRoot.includes("tine.quote-prefix::") && newRoot.includes("tine.quote-suffix::"), "New roots must write context even when the old root omitted it");
    assert.match(newRoot, /tine.date:: \d{4}-\d\d-\d\dT\d\d:\d\dZ/, "New comments carry a creation date");
    assert.equal((fs.readFileSync(file, "utf8").match(/^\s*tine.quote:: discuss$/gm) || []).length, 3);
    await shot(`${mode}-02-explicit-thread`);
    stage = `${mode}: subtree expand`;
    await browser.$(".comment-below-chip").click();
    await browser.$(".comment-below-chip").waitForExist({ reverse: true });
    await browser.waitUntil(() => browser.execute(() => [...document.querySelectorAll(".margin-column .block-content")].some(el => el.textContent.includes("Hidden thread"))));
    await waitForFileText(file, text => !text.includes("collapsed:: true"));
    stage = `${mode}: narrow visibility control`;
    await browser.setWindowSize(700, 900);
    await menuToggle("Hide comments");
    assert.equal(await browser.execute(() => document.querySelectorAll(".comment-group,.comment-quote-anchor,.comment-below-chip,.margin-marker").length), 0);
    await menuToggle("Show comments");
    // Desktop's minimum window size is wider than a phone. Use the existing
    // interface zoom controls to reach the phone CSS layout in the real app.
    let zoomSteps = 0;
    while (zoomSteps < 10 && await browser.execute(() => innerWidth > 420)) {
      const beforeWidth = await browser.execute(() => innerWidth);
      await keys("Control", "="); zoomSteps++;
      await browser.waitUntil(() => browser.execute(before => innerWidth < before, beforeWidth));
    }
    assert.ok(await browser.execute(() => innerWidth <= 420), "Need a phone-width CSS viewport for the visible page-menu control");
    const phoneCssWidth = await browser.execute(() => innerWidth);
    if (await browser.execute(() => !!document.querySelector("[data-mobile-drawer-scrim]"))) {
      await keys("Escape");
      await browser.waitUntil(() => browser.execute(() => !document.querySelector("[data-mobile-drawer-scrim]")));
    }
    await browser.$(".page-blocks .margin-marker").waitForExist();
    await menuToggle("Hide comments", true);
    assert.equal(await browser.execute(() => document.querySelectorAll(".comment-group,.comment-quote-anchor,.comment-below-chip,.margin-marker").length), 0);
    await menuToggle("Show comments", true); await browser.$(".page-blocks .margin-marker").waitForExist();
    await shot(`${mode}-04-narrow-comments`);
    await keys("Control", "0");
    await browser.waitUntil(() => browser.execute(() => innerWidth > 420));
    await browser.executeAsync(done => requestAnimationFrame(() => requestAnimationFrame(done)));
    stage = `${mode}: fresh Comment shortcut`;
    const body = await browser.$("//div[contains(concat(' ', normalize-space(@class), ' '), ' block-content ') and contains(., 'Body Enter sentinel')]");
    await body.click(); await browser.$(editorSelector).waitForDisplayed(); await keys("Control", "Home");
    for (let i = 0; i < 4; i++) await keys("Shift", "ArrowRight");
    await keys("Control", "r");
    await browser.waitUntil(() => browser.execute(selector => {
      const el = document.querySelector(selector);
      return !!el?.closest(".comment-card");
    }, editorSelector));
    await type("Fresh comment"); await keys("Escape");
    await waitForFileText(file, text => text.includes("Fresh comment") && text.includes("tine.quote:: Body"));
    stage = `${mode}: body Enter still splits`;
    await body.click(); await browser.$(editorSelector).waitForDisplayed(); await keys("Control", "End"); await keys("Enter");
    await type("New body block"); await keys("Escape");
    await waitForFileText(file, text => text.includes("\n- New body block"));
    proof.outcomes.push({ mode, groupedQuote: true, multilineEditor: true, multilineReply: true, explicitNewThread: true, freshCommentShortcut: true, subtreeExpand: true, narrowMenu: true, phoneCssWidth, bodyEnter: true });
  }
  await measureUnitCosts();
  fixture.verify(); fs.writeFileSync(path.join(out, "receipt.json"), JSON.stringify({ outcome: "PASS", ...proof }, null, 2)); console.log(`PASS ${out}`);
} catch (error) {
  if (process.env.MC010_FOCUS_PROBE === "1") {
    try { proof.focusProbes.push(await browser.execute(() => ({ events: window.__mcFocusTrace, active: document.activeElement?.outerHTML?.slice(0, 400) }))); } catch { /* retain original failure */ }
  }
  try { await shot("failure"); } catch { /* retain the original failure if capture fails */ }
  fs.writeFileSync(path.join(out, "failure.json"), JSON.stringify({ ...proof, stage, expected: "Whole-thread Resolve/reveal/Reopen and reply-to-reopen; exact Markdown and immutable dates; revealed closed cards beside later-line passages without highlights; multiline comments, groups, subtree expand and visibility", observed: error.stack,
    classification: error.code === "ERR_ASSERTION" ? "product" : "ambiguous harness/infrastructure" }, null, 2)); throw error;
} finally {
  try { await browser?.deleteSession(); } catch { /* session may have failed during startup */ }
  try { process.kill(-driver.pid, "SIGTERM"); } catch { /* driver may already have exited */ }
  fs.closeSync(log); fixture.close?.();
}
