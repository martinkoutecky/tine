// GH #783: real textarea layout and toolbar, driven through Android-style
// viewport resize steps followed by a delayed WebView scroll. No graph or IME
// is substituted for a real-device claim: this proves the event-order contract.
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
const source = String.raw`
  import { render } from "solid-js/web";
  import { MobileKeyboardToolbar, NATIVE_IME_EVENT } from "/src/components/MobileKeyboardToolbar.tsx";
  import { registerFocusedEditorCommandBridge } from "/src/editorCommandBridge.ts";
  import { caretViewRow } from "/src/editor/caretReveal.ts";
  import "/src/styles/theme.css";
  import "/src/styles/app.css";
  const main = document.querySelector("main");
  const editor = document.querySelector("textarea");
  render(() => <MobileKeyboardToolbar />, document.querySelector("#dock"));
  main.scrollTop = 650;
  window.fixture = {
    focus(tall = false) {
      editor.style.height = tall ? "720px" : "40px";
      editor.value = tall ? "Edited first line\n".repeat(30) : "Edited heading";
      editor.focus({ preventScroll: true });
      editor.setSelectionRange(3, 3);
      registerFocusedEditorCommandBridge({ blockId: "fixture", dispatch: () => true, blur: () => editor.blur() });
    },
    ime(up) { window.dispatchEvent(new CustomEvent(NATIVE_IME_EVENT, { detail: up })); },
    scroll(delta, user = false) {
      if (user) main.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      main.scrollTop += delta;
    },
    measure() {
      const row = caretViewRow(editor, main);
      const dock = document.querySelector("[data-mobile-keyboard-toolbar]");
      const view = main.getBoundingClientRect();
      return { ...row, scrollerTop: view.top, scrollerBottom: view.bottom, toolbarTop: dock.getBoundingClientRect().top, scrollTop: main.scrollTop };
    },
  };
`;
const html = `<!doctype html><meta name="viewport" content="width=device-width, initial-scale=1">
<style>
  :root { --system-inset-bottom: 0px; --system-inset-left: 0px; --system-inset-right: 0px; }
  body { margin: 0; }
  main.main-content { position: fixed; inset: 64px 0 0; padding: 0; display: block; overflow: auto; overflow-anchor: none; }
  .spacer-before { height: 900px; }
  .spacer-after { height: 1600px; }
  textarea.block-editor { display: block; width: 95%; margin: 0; padding: 4px; border: 0; font: 18px/24px sans-serif; }
</style>
<main class="main-content"><div class="spacer-before"></div><textarea class="block-editor"></textarea><div class="spacer-after"></div></main>
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
        for (const file of ["src/components/MobileKeyboardToolbar.tsx", "src/editor/caretReveal.ts"]) {
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
  assert(g.top >= g.scrollerTop - 1 && g.bottom <= Math.min(g.scrollerBottom, g.toolbarTop) + 1,
    "Edited line must be visible above the keyboard toolbar: " + JSON.stringify(g));
}
try {
  for (const journey of ["late-scroll", "late-scroll-below", "tall-late-scroll", "late-focus", "user-scroll", "keyboard-close"]) {
    const context = await browser.newContext({ viewport: { width: 412, height: 840 }, hasTouch: true });
    const page = await context.newPage();
    page.on("pageerror", error => failures.push(journey + ": " + error.message));
    try {
      await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/`);
      await page.waitForFunction(() => !!window.fixture);
      if (journey !== "late-focus") await page.evaluate(tall => window.fixture.focus(tall), journey === "tall-late-scroll");
      await page.evaluate(() => window.fixture.ime(true));
      for (const height of [690, 550, 430]) {
        await page.setViewportSize({ width: 412, height });
        await frames(page);
      }
      if (journey === "late-focus") {
        await page.evaluate(() => { window.fixture.scroll(500); window.fixture.focus(); });
      } else {
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
