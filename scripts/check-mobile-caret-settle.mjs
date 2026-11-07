// GH #783: real textarea layout and toolbar, driven through Android-style
// viewport resize steps followed by a delayed WebView scroll. No graph or IME
// is substituted for a real-device claim: this proves the event-order contract.
// `TINE_CARET_ENGINE=cm6` runs the same journeys with the CodeMirror editor (cm6CaretReveal on a real EditorView).
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { createServer } from "vite";
import solid from "vite-plugin-solid";

const root = fileURLToPath(new URL("../", import.meta.url));
const out = path.resolve(process.argv[2] || "target/artifacts/fix-beta-caret-783/browser");
fs.mkdirSync(out, { recursive: true });
const reference = process.env.TINE_CARET_BASE_REF;
const cm6 = process.env.TINE_CARET_ENGINE === "cm6";
const source = String.raw`
  import { render } from "solid-js/web";
  import { MobileKeyboardToolbar, NATIVE_IME_EVENT } from "/src/components/MobileKeyboardToolbar.tsx";
  import { registerFocusedEditorCommandBridge } from "/src/editorCommandBridge.ts";
  import { caretViewRow } from "/src/editor/caretReveal.ts";
  import { EditorView } from "@codemirror/view";
  import { StateEffect } from "@codemirror/state";
  import { cm6CaretReveal } from "/src/editor/cm6CaretReveal.ts";
  import { applyAndroidInterfaceZoom } from "/src/zoom.ts";
  import "/src/styles/theme.css";
  import "/src/styles/app.css";
  const main = document.querySelector("main");
  const CM6 = ${cm6};
  let cmView;
  let editor = document.querySelector("textarea");
  if (CM6) {
    cmView = new EditorView({ parent: document.querySelector("#cm6-host"), doc: "", extensions: [EditorView.lineWrapping, EditorView.contentAttributes.of({ "data-cm6-editor": "fixture" })] });
    editor = cmView.contentDOM;
    const reveal = cm6CaretReveal(cmView);
    cmView.dispatch({ effects: StateEffect.appendConfig.of(reveal.extension) });
  }
  render(() => <MobileKeyboardToolbar />, document.querySelector("#dock"));
  main.scrollTop = 650;
  window.fixture = {
    focus(tall = false) {
      const text = tall ? "Edited first line\n".repeat(30) : "Edited heading";
      if (CM6) {
        cmView.dispatch({ changes: { from: 0, to: cmView.state.doc.length, insert: text }, selection: { anchor: 3 } });
        cmView.focus();
      } else {
        editor.style.height = tall ? "720px" : "40px";
        editor.value = text;
        editor.focus({ preventScroll: true });
        editor.setSelectionRange(3, 3);
      }
      registerFocusedEditorCommandBridge({ blockId: "fixture", dispatch: () => true, blur: () => editor.blur() });
    },
    zoom(scale) { applyAndroidInterfaceZoom(scale); },
    // A native scroll that leaves the line's top 4px under the painted pins (GH #783 follow-up).
    parkUnderPins() {
      const g = window.fixture.measure();
      main.scrollTop += (g.lineTop - (g.pinsBottom - 4)) / g.zoom;
    },
    ime(up) { window.dispatchEvent(new CustomEvent(NATIVE_IME_EVENT, { detail: up })); },
    scroll(delta, user = false) {
      if (user) main.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      main.scrollTop += delta;
    },
    measure() {
      const mainBox = main.getBoundingClientRect();
      const zoom0 = parseFloat(getComputedStyle(document.documentElement).zoom) || 1;
      // CM6: the caret row is the view's own measurement; the bounds are the harness's (pins + margin, scroller bottom - margin).
      const row = CM6 ? { ...cmView.coordsAtPos(3), visibleTop: NaN, visibleBottom: NaN } : caretViewRow(editor, main);
      const dock = document.querySelector("[data-mobile-keyboard-toolbar]");
      const view = main.getBoundingClientRect();
      // GH #783 beta.4: the pinned parents drawn over the top of the page, measured as painted.
      const pinsBottom = Math.max(view.top, ...[...document.querySelectorAll(".sticky-ancestor button:not([hidden])")].map(pin => pin.getBoundingClientRect().bottom));
      // The caret is on the textarea's first line (offset 3): its painted line box, measured without caretViewRow
      // (4px padding, 24px line height, both scaled by the root's CSS zoom).
      const zoom = parseFloat(getComputedStyle(document.documentElement).zoom) || 1;
      const lineTop = CM6 ? editor.querySelector(".cm-line").getBoundingClientRect().top : editor.getBoundingClientRect().top + (4 - editor.scrollTop) * zoom;
      if (CM6) row.visibleTop = Math.max(mainBox.top, pinsBottom) + 8 * zoom0;
      return { ...row, zoom, lineTop, lineBottom: lineTop + 24 * zoom, scrollerTop: view.top, scrollerBottom: view.bottom, pinsBottom, toolbarTop: dock.getBoundingClientRect().top, scrollTop: main.scrollTop };
    },
  };
`;
const html = `<!doctype html><meta name="viewport" content="width=device-width, initial-scale=1">
<style>
  :root { --system-inset-bottom: 0px; --system-inset-left: 0px; --system-inset-right: 0px; }
  body { margin: 0; }
  main.main-content { position: fixed; inset: 64px 0 0; padding: 0; display: block; overflow: auto; overflow-anchor: none; }
  .spacer-before { height: 450px; }
  .spacer-after { height: 1600px; }
  textarea.block-editor { display: block; width: 95%; margin: 0; padding: 4px; border: 0; font: 18px/24px sans-serif; }
  #cm6-host { width: 95%; }
  #cm6-host .cm-editor { outline: none; }
  #cm6-host .cm-content { padding: 4px; font: 18px/24px sans-serif; }
  #cm6-host .cm-line { padding: 0; }
</style>
<main class="main-content"><div class="ls-block" data-sticky-parent><div class="sticky-ancestor" style="top: 0"><button type="button">Parent A</button></div><div class="block-main">Parent A</div><div class="spacer-before"></div><div class="ls-block" data-sticky-parent><div class="sticky-ancestor" style="top: 30px"><button type="button">Parent B</button></div><div class="block-main">Parent B</div><div class="spacer-before"></div><div class="ls-block">${cm6 ? '<div id="cm6-host"></div>' : '<textarea class="block-editor"></textarea>'}</div><div class="spacer-after"></div></div></div></main>
<div id="dock"></div><script type="module" src="/__caret_settle.tsx"></script>`;
const server = await createServer({
  root,
  configFile: false,
  cacheDir: path.join(out, ".vite"),
  define: { __BUILD_TIME__: JSON.stringify(""), __GIT_COMMIT__: JSON.stringify(""), __TINE_COMMUNITY_REGISTRY__: "false" },
  plugins: [{
    name: "caret-settle-fixture",
    enforce: "pre",
    configureServer(dev) {
      dev.middlewares.use((req, res, next) => {
        if (req.url !== "/") return next();
        res.setHeader("content-type", "text/html");
        res.end(html);
      });
    },
    resolveId(id) { if (id === "/__caret_settle.tsx") return "virtual:caret-settle.tsx"; },
    load(id) {
      if (id === "virtual:caret-settle.tsx") return source;
      if (reference) {
        for (const file of ["src/components/MobileKeyboardToolbar.tsx", "src/editor/caretReveal.ts", "src/editor/cm6CaretReveal.ts"]) {
          if (id === path.join(root, file)) return execFileSync("git", ["show", reference + ":" + file], { cwd: root, encoding: "utf8" });
        }
      }
    },
  }, solid()],
  server: { host: "127.0.0.1", port: 0 },
});
await server.listen();
const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
const results = [];
const failures = [];
const frames = page => page.evaluate(async () => {
  for (let i = 0; i < 8; i++) await new Promise(resolve => requestAnimationFrame(resolve));
});
function visible(g) {
  // caretViewRow must see the line and the pins where they are painted, at any interface zoom (GH #783 follow-up).
  // CM6 reports the glyph box, which sits inside the 24px line box; the textarea mirror reports the line box itself.
  assert(cm6 ? g.top >= g.lineTop - 1 && g.bottom <= g.lineBottom + 1 : Math.abs(g.top - g.lineTop) <= 1 && Math.abs(g.bottom - g.lineBottom) <= 1,
    "Caret row measured off its painted line: " + JSON.stringify(g));
  assert(Math.abs(g.visibleTop - (g.pinsBottom + 8 * g.zoom)) <= 1, "Visible top must be the painted pins' bottom plus the margin: " + JSON.stringify(g));
  assert(g.lineTop >= g.pinsBottom - 1 && g.lineBottom <= Math.min(g.scrollerBottom, g.toolbarTop) + 1,
    "Edited line must be visible below the pinned parents and above the keyboard toolbar: " + JSON.stringify(g));
  // GH #783 comment 14: leave three body lines below the caret for children/following blocks.
  if (g.editorHeight < 100) assert(g.childTop >= g.lineBottom && g.childBottom < g.toolbarTop,
    "A short parent's first child must remain visible above the keyboard toolbar: " + JSON.stringify(g));
  // At large zoom the pins can leave less than three lines; the caret must stay below them.
  const target = Math.max(g.visibleTop + g.lineBottom - g.lineTop, g.toolbarTop - 3 * g.bodyLine * g.zoom);
  assert(Math.abs(g.lineBottom - target) <= 2,
    "Edited line brought back must leave three zoomed body lines, bounded by the pinned parents: " + JSON.stringify(g));
}
try {
  for (const journey of ["keyboard-open", "two-line-keyboard-open", "zoom-keyboard-open", "late-scroll", "late-scroll-below", "tall-late-scroll", "late-focus", "user-scroll", "keyboard-close", "zoom-late-scroll", "zoom150-late-scroll", "zoom-under-pins", "zoom150-under-pins"]) {
    const context = await browser.newContext({ viewport: { width: 412, height: 840 }, hasTouch: true });
    const page = await context.newPage();
    page.on("pageerror", error => failures.push(journey + ": " + error.message));
    try {
      await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/`);
      await page.waitForFunction(() => !!window.fixture);
      if (journey.startsWith("zoom")) await page.evaluate(scale => window.fixture.zoom(scale), journey.startsWith("zoom150") ? 1.5 : 1.25);
      if (journey !== "late-focus") await page.evaluate(journey => window.fixture.focus(journey === "tall-late-scroll", journey === "two-line-keyboard-open"), journey);
      await page.evaluate(() => window.fixture.ime(true));
      for (const height of [690, 550, 430]) {
        await page.setViewportSize({ width: 412, height });
        await frames(page);
      }
      if (journey === "late-focus") {
        await page.evaluate(() => { window.fixture.scroll(500); window.fixture.focus(); });
      } else if (journey.endsWith("under-pins")) {
        await page.evaluate(() => window.fixture.parkUnderPins());
      } else if (!journey.endsWith("keyboard-open")) {
        // Native focus scroll arrives AFTER beta.2's two-frame reveal stopped.
        await page.evaluate(delta => window.fixture.scroll(delta), journey === "late-scroll-below" ? -300 : 500);
      }
      await frames(page);
      const settled = await page.evaluate(() => window.fixture.measure());
      await page.screenshot({ path: path.join(out, journey + ".png") });
      visible(settled);
      await frames(page);
      assert.equal((await page.evaluate(() => window.fixture.measure())).scrollTop, settled.scrollTop, "Visible caret should not keep moving");
      if (journey === "user-scroll" || journey === "keyboard-close") {
        if (journey === "keyboard-close") {
          await page.evaluate(() => window.fixture.ime(false));
          await page.setViewportSize({ width: 412, height: 840 });
          await frames(page);
        }
        const before = await page.evaluate(() => window.fixture.measure());
        await page.evaluate(user => window.fixture.scroll(150, user), journey === "user-scroll");
        await frames(page);
        assert.equal((await page.evaluate(() => window.fixture.measure())).scrollTop, before.scrollTop + 150, "Reader's scroll must win");
      }
      results.push({ journey, verdict: "pass", settled });
    } catch (error) {
      failures.push(journey + ": " + error.message);
      results.push({ journey, verdict: "fail", error: error.message });
      await page.screenshot({ path: path.join(out, journey + "-failure.png") });
    } finally { await context.close(); }
  }
} finally {
  fs.writeFileSync(path.join(out, "results.json"), JSON.stringify({ reference: reference || "worktree", results, failures }, null, 2) + "\n");
  await browser.close();
  await server.close();
}
console.log(JSON.stringify({ journeys: results.length, failures }, null, 2));
if (failures.length) process.exitCode = 1;
