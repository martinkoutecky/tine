// GH #868: reference disclosure and editor lifetimes must preserve transcluded outlines.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { remote } from "webdriverio";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";

const root = path.resolve(import.meta.dirname, "..");
const out = path.join(root, "target/artifacts/fix-beta-embed-868");
fs.mkdirSync(out, { recursive: true });
const run = fs.mkdtempSync(path.join(out, `${process.env.E2E_LABEL || "native"}-`));
const graph = path.join(run, "graph");
const corpus = process.env.E2E_CORPUS;
const surface = process.env.E2E_SURFACE || "main";
const surfaceSelector = surface === "sidebar" ? ".right-sidebar" : surface === "references" ? ".main-content .linked-references" : ".main-content";
const host = process.env.E2E_HOST || "Embed host";
const source = process.env.E2E_SOURCE || "Outline source";
const parent = process.env.E2E_PARENT || (surface === "references" ? "Surveys Linked target" : "Surveys");
const later = process.env.E2E_LATER || "86800000-0000-4000-8000-000000000002";
const kind = process.env.EMBED_KIND || "page";
const sourceRoot = "86800000-0000-4000-8000-000000000001";
const testedCommit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
if (corpus) {
  fs.cpSync(corpus, graph, { recursive: true });
  const writable = dir => {
    fs.chmodSync(dir, 0o700);
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      if (entry.isDirectory()) writable(file);
      else if (entry.isFile()) fs.chmodSync(file, 0o600);
    }
  };
  writable(graph);
} else {
  for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
  fs.writeFileSync(path.join(graph, "pages/Outline source.md"),
    `tags:: Example\n\n- Survey trial\n  id:: ${sourceRoot}\n  - Trial child\n- Review together\n  - TODO Prepare questions\n  - LATER Review answers\n    id:: ${later}\n- Open questions\n  collapsed:: true\n  - Hidden question\n- Definitions\n  collapsed:: true\n  - Hidden definition\n- Archive\n  collapsed:: true\n  - Old notes\n`);
  if (kind === "block") {
    const file = path.join(graph, "pages/Outline source.md");
    const body = fs.readFileSync(file, "utf8").split("\n\n")[1].replace(`  id:: ${sourceRoot}\n`, "");
    fs.writeFileSync(file, `- Shared outline\n  id:: ${sourceRoot}\n${body.trimEnd().split("\n").map(line => `  ${line}`).join("\n")}\n`);
  }
  const macro = kind === "block" ? `{{embed ((${sourceRoot}))}}` : "{{embed [[Outline source]]}}";
  fs.writeFileSync(path.join(graph, "pages/Embed host.md"), `- Surveys${surface === "references" ? " [[Linked target]]" : ""}\n  collapsed:: true\n  - ${macro}\n- Host sibling\n`);
  fs.writeFileSync(path.join(graph, "pages/Launcher.md"), "- [[Embed host]]\n");
  fs.writeFileSync(path.join(graph, "pages/Linked target.md"), "- Reference target\n");
  fs.writeFileSync(path.join(graph, "pages/Referrer.md"), `- See ((${later}))\n`);
  fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{:tine/guide-announced? true}\n");
}
if (process.env.E2E_NO_CSS === "1") {
  const css = path.join(graph, "logseq/custom.css");
  if (fs.existsSync(css)) fs.renameSync(css, `${css}.disabled`);
}
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(run, dir));
const fixture = accountHomeFixture(run, run);
const env = { ...fixture.env, HOME: path.join(run, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(run, "data"), XDG_CONFIG_HOME: path.join(run, "config"),
  XDG_CACHE_HOME: path.join(run, "cache"), XDG_STATE_HOME: path.join(run, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" };
const port = Number(process.env.E2E_DRIVER_PORT || 4918);
const log = fs.openSync(path.join(run, "driver.log"), "w");
const driver = spawn(process.env.TAURI_DRIVER || "tauri-driver", ["--port", String(port), "--native-port", String(port + 1),
  "--native-driver", process.env.WEBKIT_DRIVER || "/usr/bin/WebKitWebDriver"], { env, stdio: ["ignore", log, log], detached: true });
let browser, stage = "startup", original;
const evidence = [];
let scope = ".main-content .embed-block";
const pause = () => browser.pause(Number(process.env.E2E_CLICK_DELAY || 300));
async function capture(label) {
  const data = await browser.execute(selector => {
    const embed = document.querySelector(selector);
    if (!embed) return null;
    // Reference panels can render further blocks inside a source row. Their
    // rows are independent occurrences and do not belong to this outline.
    const rows = [...embed.querySelectorAll(".ls-block")].filter(row => !row.closest(".block-references")
      && row.closest(".embed-block") === embed);
    return { html: embed.outerHTML, rows: rows.map(row => {
      const ancestors = []; let p = row.parentElement?.closest(".ls-block");
      while (p && embed.contains(p)) { ancestors.push(p.dataset.blockId); p = p.parentElement?.closest(".ls-block"); }
      const main = row.querySelector(":scope > .block-main");
      return { id: row.dataset.blockId, ref: row.dataset.blockRef, depth: ancestors.length,
        collapsed: row.classList.contains("collapsed"), readOnly: row.classList.contains("ref-block"),
        text: main?.querySelector(".block-content")?.textContent ?? main?.querySelector("textarea")?.value,
        disclosure: !!main?.querySelector(".collapse-toggle.has-children") };
    }) };
  }, scope);
  assert(data, `embed missing at ${label}`);
  await browser.execute(selector => document.querySelector(selector)?.scrollIntoView({ block: "start" }), scope);
  await browser.pause(100);
  fs.writeFileSync(path.join(run, `${label}.html`), data.html);
  fs.writeFileSync(path.join(run, `${label}.json`), JSON.stringify(data.rows, null, 2));
  execFileSync("import", ["-window", "root", path.join(run, `${label}.png`)], { timeout: 10000 });
  evidence.push({ label, rows: data.rows });
  return data.rows;
}
const shape = rows => rows.map(({ id, depth, collapsed, readOnly, disclosure }) => ({ id, depth, collapsed, readOnly, disclosure }));
async function sourceOutline() {
  const result = await browser.executeAsync((args, done) => {
    window.__TAURI_INTERNALS__.invoke("load_graph", { path: args.graph }).then(binding => {
      if (binding.kind !== "already_current") throw new Error(`unexpected binding: ${binding.kind}`);
      return window.__TAURI_INTERNALS__.invoke("get_page", { name: args.source, kind: "page", bindingGeneration: binding.binding_generation });
    }).then(page => done({ page }), error => done({ error: String(error) }));
  }, { graph, source });
  assert(!result.error, result.error); assert(result.page, "source page is available");
  const rows = [];
  const visit = (block, depth) => {
    rows.push({ id: block.id, depth, collapsed: block.collapsed });
    if (!block.collapsed) block.children.forEach(child => visit(child, depth + 1));
  };
  result.page.blocks.forEach(block => visit(block, 0));
  return { rows, file: path.join(graph, result.page.id) };
}
async function clickContent(id) {
  const selector = `${scope} [data-block-id="${id}"] > .block-main .block-content`;
  const content = await browser.$(selector);
  await content.scrollIntoView({ block: "center" });
  // An interior text point avoids task controls, links and the reference badge.
  await browser.execute(sel => {
    const content = document.querySelector(sel);
    const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
    let text;
    while ((text = walker.nextNode())) {
      if (!text.textContent.trim() || text.parentElement.closest("a,.block-marker,.block-task-checkbox,.page-ref")) continue;
      const range = document.createRange(); range.selectNodeContents(text);
      const box = range.getBoundingClientRect();
      if (box.width && box.height) return { x: Math.round(box.left + Math.min(20, box.width / 2)), y: Math.round(box.top + box.height / 2) };
    }
    const box = content.getBoundingClientRect(); return { x: Math.round(box.left + 20), y: Math.round(box.top + box.height / 2) };
  }, selector).then(async point => {
    await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", duration: 0, origin: "viewport", ...point }, { type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 },
    ] }]);
    await browser.releaseActions();
  });
  await browser.$(`${scope} [data-block-id="${id}"] > .block-main textarea.block-editor`).waitForExist({ timeout: 10000 });
}
try {
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 2, connectionRetryTimeout: 30000,
    capabilities: { browserName: "wry", "wdio:enforceWebDriverClassic": true,
      "tauri:options": { application: process.env.TINE_APP || path.join(root, "target/release/tine") } } });
  await ensureMainWindow(browser);
  await browser.setWindowSize(Number(process.env.E2E_WIDTH || 1200), Number(process.env.E2E_HEIGHT || 1000));
  await browser.$(".app-container").waitForExist({ timeout: 30000 });
  await assertAppLease(fixture);
  if (surface === "sidebar") {
    await openPageByName(browser, "Launcher");
    const link = await browser.$(".main-content .page-ref");
    await browser.performActions([{ type: "key", id: "keyboard", actions: [{ type: "keyDown", value: "\uE008" }] }]);
    await link.click(); await browser.releaseActions();
    await browser.$(".right-sidebar .rs-item-body").waitForExist({ timeout: 20000 });
  } else if (surface === "references") {
    await openPageByName(browser, "Linked target");
    await browser.$(".linked-references .references-header").waitForExist({ timeout: 20000 });
    await browser.waitUntil(() => browser.execute(() => Number(document.querySelector(".linked-references .references-count")?.textContent.trim()) > 0), { timeout: 20000 });
    if (!(await browser.$(".linked-references .reference-group").isExisting())) await browser.$(".linked-references .references-header").click();
  } else await openPageByName(browser, host);
  stage = "expand host";
  let parentId;
  await browser.waitUntil(async () => (parentId = await browser.execute(args => {
    const row = [...document.querySelectorAll(`${args.selector} .ls-block`)].find(row =>
      row.querySelector(":scope > .block-main .block-content")?.textContent.trim().replace(/\[\[|\]\]/g, "") === args.label);
    if (!row) return false;
    row.scrollIntoView({ block: "center" });
    if (row.classList.contains("collapsed")) row.querySelector(":scope > .block-main .collapse-toggle").click();
    return row.dataset.blockId;
  }, { selector: surfaceSelector, label: parent })), { timeout: 20000 });
  scope = `${surfaceSelector} .ls-block[data-block-id="${parentId}"] > .block-children-container .embed-block`;
  await browser.$(`${scope} .ls-block:not(.ref-block)`).waitForExist({ timeout: 30000 });
  await pause();
  original = await capture("01-expanded");
  assert(original.every(row => !row.readOnly), "source outline must hydrate live");
  const expected = await sourceOutline();
  if (kind === "page") assert.deepEqual(original.map(({ id, depth, collapsed }) => ({ id, depth, collapsed })), expected.rows,
    "page embed must equal the source page's visible outline");
  const originalBytes = fs.readFileSync(expected.file, "utf8");
  stage = "open nested task references";
  const badge = await browser.$(`${scope} [data-block-ref="${later}"] > .block-main .block-refs-count`);
  await badge.waitForExist({ timeout: 20000 }); await badge.scrollIntoView({ block: "center" });
  if (process.env.E2E_REF_SIDEBAR === "1") {
    await browser.performActions([{ type: "key", id: "keyboard", actions: [{ type: "keyDown", value: "\uE008" }] }]);
    await badge.click(); await browser.releaseActions();
    await browser.$(`.right-sidebar .ls-block[data-block-ref="${later}"]`).waitForExist({ timeout: 20000 });
  } else {
    await badge.click();
    await browser.$(`${scope} .block-references .reference-group`).waitForExist({ timeout: 20000 });
  }
  await pause(); await capture("02-references");
  const todo = await browser.execute(selector => document.querySelector(`${selector} .marker-todo`)?.closest(".ls-block")?.dataset.blockId, scope);
  assert(todo, "TODO above LATER is present");
  stage = "enter TODO editor";
  await clickContent(todo); await pause(); await capture("03-editing-todo");
  stage = "click outside TODO editor";
  await browser.$("h1.page-title").click(); await pause();
  const after = await capture("04-left-todo");
  assert.deepEqual(shape(after), shape(original), "leaving TODO must preserve source outline order, depth and folds");
  for (const row of original.slice(0, 10)) {
    stage = `edit sibling ${row.id}`;
    await clickContent(row.id); await pause(); await browser.$("h1.page-title").click(); await pause();
    assert.deepEqual(shape(await capture(`05-sibling-${evidence.length}`)), shape(original), "sibling editor must preserve source outline");
  }
  assert.equal(fs.readFileSync(expected.file, "utf8"), originalBytes, "click-only editing leaves source bytes unchanged");
  if (process.env.E2E_STRUCTURAL === "1") {
    stage = "reparent an embedded sibling";
    const depth = kind === "block" ? 1 : 0;
    const candidate = original.filter(row => row.depth === depth && !row.collapsed)[1];
    assert(candidate, "second visible source sibling is present");
    await clickContent(candidate.id); await browser.keys(["Tab"]); await pause();
    const changed = await capture("06-indented-sibling");
    assert.deepEqual(changed.filter(row => row.id === candidate.id).map(row => row.depth), [depth + 1],
      "reparented source sibling must appear once at its new depth");
    await browser.keys(["Control", "z"]); await pause(); await browser.$("h1.page-title").click(); await pause();
    assert.deepEqual(shape(await capture("07-undone-sibling")), shape(original), "Undo restores source outline");
  }
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "pass", testedCommit, stage, corpus: !!corpus,
    kind, surface, width: process.env.E2E_WIDTH || 1200, cssDisabled: process.env.E2E_NO_CSS === "1", evidence }, null, 2));
  console.log(`PASS GH #868 ${run}`);
} catch (error) {
  if (browser) {
    try { await capture("failure"); } catch {}
    try { fs.writeFileSync(path.join(run, "failure-document.html"), await browser.execute(() => document.documentElement.outerHTML)); } catch {}
  }
  fs.writeFileSync(path.join(run, "result.json"), JSON.stringify({ outcome: "fail", testedCommit, journey: "GH #868 references then embedded edit",
    expected: "source outline order, depth and folds survive edit entry/exit", observed: String(error), stage,
    classification: error.code === "ERR_ASSERTION" ? "product" : "harness", evidence }, null, 2));
  console.error(`FAIL ${stage}: ${error}\n${run}`); process.exitCode = 1;
} finally {
  if (browser) await browser.deleteSession().catch(() => {});
  try { process.kill(-driver.pid, "SIGTERM"); } catch {}
  fixture.close?.(); fs.closeSync(log);
}
