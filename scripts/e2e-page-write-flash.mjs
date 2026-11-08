// GH #911: a large routed page remains mounted during real guarded CLI writes.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import net from "node:net";
import { spawn, execFile, execFileSync } from "node:child_process";
import { promisify } from "node:util";
import { setTimeout as sleep } from "node:timers/promises";
import { remote } from "webdriverio";
import { accountHomeFixture, assertAppLease } from "./lib/e2e-account-home.mjs";
import { ensureMainWindow } from "./lib/e2e-main-window.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";
import { tauriCapabilities, webdriverServerArgs } from "./e2e-capabilities.mjs";

const root = path.resolve(import.meta.dirname, "..");
const artifacts = process.env.E2E_ARTIFACT_DIR || path.join(root, "target/artifacts/fix-beta-page-flash-911");
fs.mkdirSync(artifacts, { recursive: true });
const out = fs.mkdtempSync(path.join(artifacts, "native-"));
const graph = path.join(out, "graph");
// An explicitly supplied approved corpus is copied before any app/CLI can open it.
const corpus = process.env.E2E_CORPUS;
if (corpus) {
  fs.cpSync(corpus, graph, { recursive: true, dereference: true });
  // Approved corpora may be read-only. Grant writes only to the scratch copy,
  // including its directories so guarded temp-file/rename saves can run.
  const writable = dir => {
    fs.chmodSync(dir, fs.statSync(dir).mode | 0o700);
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      if (entry.isDirectory()) writable(file);
      else fs.chmodSync(file, fs.statSync(file).mode | 0o600);
    }
  };
  writable(graph);
}
for (const dir of ["pages", "journals", "logseq"]) fs.mkdirSync(path.join(graph, dir), { recursive: true });
for (const dir of ["home", "data", "config", "cache", "state"]) fs.mkdirSync(path.join(out, dir));
if (!corpus) fs.writeFileSync(path.join(graph, "logseq/config.edn"), "{}\n");
const name = corpus ? "Tajubelanecohonipurizsigem" : "GH911 Large";
const file = path.join(graph, corpus ? `pages/shared/${name}.md` : `pages/${name}.md`);
const uuid = i => `91100000-0000-4000-8000-${String(i).padStart(12, "0")}`;
const body = corpus ? fs.readFileSync(file, "utf8") : Array.from({ length: 20 }, (_, i) =>
  `- Synthetic outline ${i}\n  collapsed:: true\n${Array.from({ length: 20 }, (_, j) =>
    `\t- Synthetic child ${j} ${"ordinary synthetic prose ".repeat(45)}\n`).join("")}`).join("");
// Known independent witnesses surround the unchanged real/synthetic large body.
fs.writeFileSync(file, `- GH911 visible witness\n  id:: ${uuid(0)}\n- GH911 caret witness\n  id:: ${uuid(1)}\n${body}\n- GH911 offscreen parent\n  id:: ${uuid(3)}\n  collapsed:: true\n\t- GH911 offscreen witness\n\t  id:: ${uuid(2)}\n`);
fs.writeFileSync(path.join(graph, "pages/GH911 Writer.md"), "- Independent writer\n");
const app = path.resolve(process.env.TINE_APP || path.join(root, "target/debug/tine"));
const fixture = accountHomeFixture(out, out, { ...process.env, HOME: path.join(out, "home"), TINE_GRAPH: graph,
  XDG_DATA_HOME: path.join(out, "data"), XDG_CONFIG_HOME: path.join(out, "config"),
  XDG_CACHE_HOME: path.join(out, "cache"), XDG_STATE_HOME: path.join(out, "state"),
  WEBKIT_DISABLE_DMABUF_RENDERER: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1", LIBGL_ALWAYS_SOFTWARE: "1", GDK_BACKEND: "x11" });
async function freePort() {
  const server = net.createServer(); await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port; await new Promise(resolve => server.close(resolve)); return port;
}
const port = await freePort(), nativePort = await freePort();
const log = fs.openSync(path.join(out, "driver.log"), "w");
const driver = spawn("tauri-driver", webdriverServerArgs(port, nativePort, "/usr/bin/WebKitWebDriver"),
  { env: fixture.env, stdio: ["ignore", log, log], detached: true });
const proof = { base: execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim(),
  app, sha256: crypto.createHash("sha256").update(fs.readFileSync(app)).digest("hex"), corpus: !!corpus,
  pageBytes: fs.statSync(file).size, observations: {}, writes: [] };
const run = promisify(execFile);
async function cli(args) {
  const started = performance.now();
  await run(app, [args[0], "--graph", graph, "--json", ...args.slice(1)], { env: fixture.env, timeout: 30000 });
  proof.writes.push({ args, elapsedMs: performance.now() - started });
}
let browser, stage = "startup";
const shot = name => execFileSync("import", ["-window", "root", path.join(out, name)], { env: fixture.env });
async function probe() {
  await browser.execute(() => {
    const prior = window.__pageFlash; prior?.observer.disconnect(); if (prior) prior.stop = true;
    const container = document.querySelector(".main-content .page-blocks");
    const containers = [...document.querySelectorAll(".main-content .page-blocks, .right-sidebar .rs-item-body")];
    const rows = containers.flatMap(body => [...body.querySelectorAll(".ls-block")]);
    const editor = container.querySelector("textarea");
    const scroller = document.querySelector(".main-content");
    const scrollPositions = [...document.querySelectorAll(".main-content, .right-sidebar")].map(el => ({ el, top: el.scrollTop }));
    const p = window.__pageFlash = { container, containers, rows, editor, scroller, scrollPositions, top: scroller.scrollTop,
      removed: 0, mutations: 0, mutationDetails: [], blankFrames: 0, replacedFrames: 0, frames: 0, stop: false };
    p.observer = new MutationObserver(records => {
      for (const record of records) {
        if (rows.some(row => row === record.target || row.contains(record.target))) {
          p.mutations++;
          if (p.mutationDetails.length < 20) p.mutationDetails.push({ type: record.type, tag: record.target.nodeName,
            class: record.target.parentElement?.className, attribute: record.attributeName,
            added: [...record.addedNodes].map(node => node.nodeName), removed: [...record.removedNodes].map(node => node.nodeName) });
        }
        for (const node of record.removedNodes)
          if (rows.some(row => node === row || node.contains(row))) p.removed++;
      }
    });
    p.observer.observe(document.querySelector(".app-container"), { childList: true, characterData: true, attributes: true, subtree: true });
    function frame() {
      if (p.stop) return;
      p.frames++;
      if (!document.querySelector(".main-content .page-blocks .ls-block")) p.blankFrames++;
      if (containers.some(body => !body.isConnected)) p.replacedFrames++;
      requestAnimationFrame(frame);
    }
    requestAnimationFrame(frame);
  });
}
const state = () => browser.execute(() => {
  const p = window.__pageFlash, editor = document.querySelector(".main-content textarea");
  return { rows: p.rows.length, retained: p.rows.filter(row => row.isConnected).length, containerRetained: p.container.isConnected,
    removed: p.removed, mutations: p.mutations, mutationDetails: p.mutationDetails, blankFrames: p.blankFrames, replacedFrames: p.replacedFrames, frames: p.frames,
    scrollDelta: p.scroller.scrollTop - p.top, editorRetained: !!p.editor && p.editor === editor,
    scrollDeltas: p.scrollPositions.map(({ el, top }) => el.scrollTop - top),
    selection: editor ? [editor.selectionStart, editor.selectionEnd] : null,
    focused: !!editor && document.activeElement === editor,
    surfaces: p.containers.map(body => ({ surface: body.closest("[data-pane-id]")?.getAttribute("data-pane-id") || "sidebar",
      retained: body.isConnected })) };
});
try {
  await sleep(300);
  browser = await remote({ hostname: "127.0.0.1", port, path: "/", logLevel: "error", connectionRetryCount: 0, connectionRetryTimeout: 30000,
    capabilities: tauriCapabilities(app) });
  await browser.waitUntil(async () => { await ensureMainWindow(browser); return browser.execute(() => !!document.querySelector(".app-container")); }, { timeout: 60000 });
  await assertAppLease(fixture);
  await openPageByName(browser, name);
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".main-content .page-blocks .ls-block")), { timeout: 30000 });
  // Establish the first full publication, before measuring the precise stream.
  await cli(["add", "--page", "GH911 Writer", "- Baseline publication"]); await sleep(1200);
  if (process.env.E2E_PANES === "1") {
    stage = "open the same page in sidebar and split pane";
    for (const key of ["\uE008", "\uE00A"]) {
      await browser.performActions([{ type: "key", id: "keyboard", actions: [{ type: "keyDown", value: key }] }]);
      try { await browser.$('[data-pane-id="main"] h1.page-title').click(); }
      finally { await browser.releaseActions(); }
    }
    await browser.waitUntil(() => browser.execute(() => document.querySelectorAll(".main-content .page-blocks").length === 2
      && !!document.querySelector(".right-sidebar .rs-item-body .ls-block")), { timeout: 20000 });
  }
  await browser.$(`.main-content [data-block-ref="${uuid(1)}"] .block-content`).click();
  await browser.waitUntil(() => browser.execute(() => !!document.querySelector(".main-content textarea")), { timeout: 10000 });
  await browser.execute(() => {
    const editor = document.querySelector(".main-content textarea"); editor.focus(); editor.setSelectionRange(3, 9);
    for (const scroller of document.querySelectorAll(".main-content, .right-sidebar")) scroller.scrollTop = 40;
  });
  await browser.performActions([{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
    { type: "pointerMove", duration: 0, x: 1100, y: 25 },
  ] }]);
  await browser.releaseActions();
  await sleep(600); // Let the pointer's previous page-link preview close.
  shot("before.png");
  for (const kind of ["unrelated", "offscreen", "visible"]) {
    stage = `${kind} CLI writes`; await probe();
    for (let i = 0; i < 3; i++) {
      const started = performance.now();
      if (kind === "unrelated") await cli(["add", "--page", "GH911 Writer", `- Background ${i}`]);
      else if (kind === "offscreen" && i === 1) await cli(["add", "--under", uuid(2), "- GH911 hidden addition"]);
      else await cli(["edit", uuid(kind === "offscreen" ? 2 : 0), `GH911 ${kind} update ${i}`]);
      if (kind === "visible") await browser.waitUntil(() => browser.execute(text =>
        document.querySelector(".main-content .page-blocks")?.textContent.includes(text), `GH911 visible update ${i}`), { timeout: 20000 });
      proof.writes.at(-1).visibleApplyMs = kind === "visible" ? performance.now() - started : null;
      await sleep(1000);
    }
    proof.observations[kind] = await state(); shot(`${kind}.png`);
    if (kind === "offscreen") {
      // Prove the hidden update really reached every surface, rather than
      // passing an identity check before a slow delivery. Opening/closing this
      // disclosure adds/removes new children, never the original witness rows.
      const toggle = () => browser.execute(id => {
        for (const row of document.querySelectorAll(`[data-block-ref="${id}"]`)) row.querySelector(".collapse-toggle")?.click();
      }, uuid(3));
      await toggle();
      await browser.waitUntil(() => browser.execute(() => {
        const containers = window.__pageFlash.containers;
        return containers.every(body => body.textContent.includes("GH911 offscreen update 2"));
      }), { timeout: 20000, timeoutMsg: "the hidden CLI edit did not reach every open surface" });
      await toggle();
      const acknowledged = await state();
      assert.equal(acknowledged.retained, acknowledged.rows, "late hidden update remounted existing rows");
      assert.equal(acknowledged.removed, 0, "late hidden update removed original rows");
      proof.hiddenAcknowledged = acknowledged;
    }
  }
  fixture.verify();
  for (const [kind, observation] of Object.entries(proof.observations)) {
    assert.equal(observation.containerRetained, true, `${kind} replaced the page container`);
    assert.ok(observation.surfaces.every(surface => surface.retained), `${kind} replaced a pane/sidebar container`);
    assert.equal(observation.retained, observation.rows, `${kind} remounted existing blocks`);
    assert.equal(observation.removed, 0, `${kind} removed existing blocks`);
    if (kind !== "visible") assert.equal(observation.mutations, 0, `${kind} mutated a rendered block`);
    assert.equal(observation.blankFrames, 0, `${kind} left an empty outline frame`);
    assert.equal(observation.scrollDelta, 0, `${kind} moved scroll position`);
    assert.ok(observation.scrollDeltas.every(delta => delta === 0), `${kind} moved a pane/sidebar scroll position`);
    assert.equal(observation.editorRetained, true, `${kind} remounted the unchanged editor`);
    assert.equal(observation.focused, true, `${kind} lost editor focus`);
    assert.deepEqual(observation.selection, [3, 9], `${kind} changed the editor selection`);
  }
  proof.outcome = "PASS";
} catch (error) {
  proof.outcome = "FAIL"; proof.stage = stage; proof.observed = String(error.stack);
  proof.expected = "A large page retains its container, existing rows, selection and scroll under all three CLI write kinds";
  proof.classification = error.code === "ERR_ASSERTION" ? "product" : "ambiguous; inspect screenshot and driver log";
  try { shot("failure.png"); } catch {}
  throw error;
} finally {
  fs.writeFileSync(path.join(out, "proof.json"), JSON.stringify(proof, null, 2)); console.log(`${proof.outcome}: ${out}`);
  try { await browser?.deleteSession(); } catch {}
  try { process.kill(-driver.pid, "SIGKILL"); } catch {}
  fixture.close(); fs.closeSync(log);
}
