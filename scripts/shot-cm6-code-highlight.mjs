// npm run build && node scripts/shot-cm6-code-highlight.mjs [outDir]
// Uses the browser-only regression graph; no native graph or settings files.
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

const out = process.argv[2] ?? "artifacts/cm6-code-highlight";
const port = Number(process.env.CM6_HIGHLIGHT_PORT || 5237);
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "preview", "--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
const results = [];
const colours = element => Object.fromEntries(["keyword", "string", "comment"].map(token => {
  const span = element.querySelector(`.hljs-${token}`);
  return [token, span ? getComputedStyle(span).color : null];
}));
try {
  mkdirSync(out, { recursive: true });
  for (let i = 0; i < 80; i++) {
    try { if ((await fetch(`http://localhost:${port}/?regressions`)).ok) break; } catch {}
    await sleep(250);
  }
  browser = await chromium.launch({ headless: true });
  for (const theme of ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width: 1200, height: 900 } });
    const errors = [];
    page.on("pageerror", error => errors.push(String(error)));
    await page.addInitScript(() => {
      window.highlightMetrics = { workers: [], applications: [], inputs: [], longTasks: [] };
      const NativeWorker = window.Worker;
      window.Worker = class extends NativeWorker {
        constructor(...args) {
          super(...args);
          if (!String(args[0]).includes("cm6CodeHighlight")) return;
          const starts = new Map();
          const post = this.postMessage.bind(this);
          this.postMessage = request => {
            starts.set(request.revision, performance.now());
            return post(request);
          };
          this.addEventListener("message", event => {
            const start = starts.get(event.data.revision);
            if (start !== undefined) window.highlightMetrics.workers.push(performance.now() - start);
          });
        }
        set onmessage(listener) {
          super.onmessage = listener ? event => {
            const start = performance.now();
            try { listener.call(this, event); }
            finally { window.highlightMetrics.applications.push(performance.now() - start); }
          } : null;
        }
      };
      let start;
      document.addEventListener("beforeinput", event => {
        if (event.target.closest?.(".cm-content")) start = performance.now();
      }, true);
      document.addEventListener("input", event => {
        if (start === undefined || !event.target.closest?.(".cm-content")) return;
        const began = start; start = undefined;
        // Includes synchronous input handlers and the queued DOM observer flush.
        queueMicrotask(() => window.highlightMetrics.inputs.push(performance.now() - began));
      });
      new PerformanceObserver(list => window.highlightMetrics.longTasks.push(...list.getEntries().map(e => ({ start: e.startTime, duration: e.duration })))).observe({ type: "longtask", buffered: true });
    });
    await page.goto(`http://localhost:${port}/?regressions`);
    await page.locator(".ls-block").first().waitFor();
    await page.evaluate(theme => document.documentElement.setAttribute("data-theme", theme), theme);
    await page.keyboard.press("Control+,");
    await page.locator(".settings-search-input").fill("CodeMirror");
    await page.locator(".settings-search-result").first().click();
    await page.locator(".settings-field").filter({ hasText: "Experimental: CodeMirror editor" }).getByRole("switch").click();
    await page.keyboard.press("Escape");
    await page.keyboard.press("Control+k");
    await page.locator(".switcher-input").fill("CM6 code highlight regression");
    await page.locator(".switcher-row").first().click();

    for (const [language, text] of [["js", "const greeting"], ["python", "def greet"]]) {
      const code = page.locator("pre.code-block").filter({ hasText: text }).first();
      await code.locator(".hljs-keyword").first().waitFor();
      const before = await code.evaluate(colours);
      const id = await code.locator("xpath=ancestor::*[@data-block-id][1]").getAttribute("data-block-id");
      const row = page.locator(`[data-block-id="${id}"]`);
      await code.locator(".hljs-string").first().click();
      const editor = row.locator(".cm-content");
      await editor.locator(".hljs-keyword").first().waitFor();
      assert.deepEqual(await editor.evaluate(colours), before, `${theme}/${language}: same view/editor token colours`);
      assert(!/```/.test(await editor.innerText()));
      await page.keyboard.type("hello");
      await editor.locator(".hljs-string").filter({ hasText: "hello" }).first().waitFor();
      assert.equal(await row.locator("textarea").count(), 0);
      await page.screenshot({ path: `${out}/${theme}-${language}-editing.png` });
      await page.keyboard.press("Escape");
    }

    const mixed = page.locator("pre.code-block").filter({ hasText: "const answer" }).first();
    await mixed.locator(".hljs-keyword").first().click();
    const editor = page.locator(".cm-content");
    await editor.locator(".hljs-keyword").first().waitFor();
    assert(!/```/.test(await editor.innerText()));
    await page.keyboard.press("Home");
    await page.keyboard.press("ArrowLeft");
    assert((await editor.innerText()).includes("```js"), "mixed boundary exposes source in CM6");
    assert.equal(await page.locator("textarea.block-editor").count(), 0);
    await page.keyboard.press("Escape");

    const large = page.locator("pre.code-block").filter({ hasText: "// Step 99" }).first();
    await large.locator(".hljs-keyword").first().click();
    await page.locator(".cm-content .hljs-keyword").first().waitFor();
    await page.keyboard.press("Control+Home");
    await sleep(300);
    await page.evaluate(() => { window.highlightMetrics.inputs = []; window.highlightMetrics.workers = []; window.highlightMetrics.applications = []; window.highlightMetrics.longTasks = []; });
    // A comment preserves the 300-line fence and avoids completion/auto-pair edits.
    await page.keyboard.type("// responsive syntax highlighting ", { delay: 100 });
    await sleep(300);
    results.push({ theme, lines: 300, ...await page.evaluate(() => window.highlightMetrics) });
    assert.equal(await page.locator(".cm-content").count(), 1);
    assert.equal(await page.locator("textarea.block-editor").count(), 0);
    assert.deepEqual(errors, []);
    await page.close();
  }
  writeFileSync(`${out}/metrics.json`, JSON.stringify(results, null, 2) + "\n");
  for (const result of results) {
    const stats = values => {
      const sorted = [...values].sort((a, b) => a - b);
      return { count: sorted.length, median: sorted[Math.floor(sorted.length / 2)], p95: sorted[Math.floor(sorted.length * .95)], max: sorted.at(-1) };
    };
    console.log(JSON.stringify({ theme: result.theme, lines: result.lines, inputMs: stats(result.inputs), workerRoundtripMs: stats(result.workers), decorationApplicationMs: stats(result.applications), longTasks: result.longTasks }));
  }
} finally {
  await browser?.close();
  server.kill();
}
