// Real frontend + graph-asset cache in Chromium. The Vite-only fixture plugin
// leaves the shipped mock and production backend unchanged.
import { createServer } from "vite";
import { chromium } from "playwright";
import fs from "node:fs";
import zlib from "node:zlib";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { gh745Page, gh745Png } from "./lib/gh745-fixture.mjs";
import { openPageByName } from "./lib/e2e-navigation.mjs";

const out = process.env.GH745_OUT || "artifacts/gh745-outdent/chromium";
fs.mkdirSync(out, { recursive: true });
const fixture = gh745Page((process.env.GH745_IMAGES || Array.from({ length: 22 }, (_, i) => 2 + 4 * i).join(",")).split(",").map(Number));
const baseline = process.env.GH745_BASE === "1";
const originals = new Map(baseline ? ["src/components/BlockList.tsx", "src/render/inline.tsx", "src/assetCache.ts", "src/editor/caretReveal.ts"].map((file) =>
  [path.resolve(file), execFileSync("git", ["show", `HEAD:${file}`], { encoding: "utf8" })]) : []);
const server = await createServer({ cacheDir: path.resolve(out, ".vite"), server: { port: 31975, strictPort: false, watch: null,
  fs: { allow: [process.cwd(), path.dirname(fs.realpathSync("node_modules/vite"))] } }, plugins: [{
  name: "gh745-fixture", enforce: "pre",
  load(id) { if (originals.has(id)) return originals.get(id); },
  transform(code, id) {
    if (!id.endsWith("/src/mock.ts")) return;
    return code.replace("const all = [...PAGES, ...NAMED, ...extraPages];", `const all = [...PAGES, ...NAMED, ...extraPages, ${JSON.stringify(fixture)}];`)
      .replace("async readAsset(name: string, maxBytes: number): Promise<Uint8Array> {", "async readAsset(name: string, maxBytes: number): Promise<Uint8Array> {\n      if ((globalThis as any).__gh745AssetGate) await (globalThis as any).__gh745AssetGate;")
      .replace("const bytes = mockAssets[name] ??", `const bytes = (name === "gh745.png" ? new Uint8Array(${JSON.stringify([...gh745Png(zlib)])}) : mockAssets[name]) ??`);
  },
}] });
await server.listen();
const browser = await chromium.launch({ headless: true, args: ["--no-sandbox"] });
const results = [];
try {
  const variants = [
    ["outdent-indent", "mid", ["Shift+Tab", "Tab"]],
    ["enter", "mid", ["Enter"]], ["backspace", "start", ["Backspace"]],
    ["delete", "end", ["Delete"]], ["indent-outdent", "mid", ["Tab", "Shift+Tab"]],
    ["move-up", "mid", ["Alt+Shift+ArrowUp"]], ["move-down", "mid", ["Alt+Shift+ArrowDown"]],
    ["drag", "mid", ["drag"]],
    // GH #745 follow-up: every image above the caret is still loading when the edit happens (an asset-cache
    // miss) and lands after revealCaretSettled's frame cap. Chromium's scroll anchoring would hide the
    // drift, so it is switched off to behave like WebKit, which has none.
    ["late-image", "mid", ["Enter"]],
    // Render cost of ordinary typing on the same long page: key to next frame, per keystroke.
    ["typing", "end", ["type"]],
  ].filter(([name]) => !process.env.ONLY || process.env.ONLY.split(",").includes(name));
  for (const [name, caret, keys] of variants) {
    const late = name === "late-image";
    const page = await browser.newPage({ viewport: { width: 1536, height: 820 } });
    if (late) await page.addInitScript(() => {
      window.__gh745AssetGate = new Promise((resolve) => { window.__gh745Release = resolve; });
      new MutationObserver(() => { const sc = document.querySelector(".main-content"); if (sc) sc.style.overflowAnchor = "none"; })
        .observe(document, { childList: true, subtree: true });
    });
    page.on("pageerror", (error) => console.error(error.message));
    page.on("console", (message) => { if (message.type() === "error") console.error(message.text()); });
    await page.goto(server.resolvedUrls.local[0]);
    try { await page.waitForSelector(".page-title"); }
    catch (error) { await page.screenshot({ path: `${out}/startup-failure.png` }); fs.writeFileSync(`${out}/startup-failure.html`, await page.content()); throw error; }
    await page.waitForTimeout(2000);
    // Adapt the canonical navigation helper to Playwright; native journeys use
    // the same helper directly through WebDriver.
    await openPageByName({
      execute: (fn, ...args) => page.evaluate(({ code, args }) => (0, eval)(`(${code})`)(...args), { code: fn.toString(), args }),
      keys: (keys) => page.keyboard.press(Array.isArray(keys) ? keys.join("+") : keys),
      $: (selector) => ({ waitForExist: (opts) => page.locator(selector).waitFor(opts), setValue: (value) => page.locator(selector).fill(value) }),
      waitUntil: async (fn, { timeout, interval = 100, timeoutMsg }) => {
        const end = Date.now() + timeout;
        while (!await fn()) { if (Date.now() > end) throw new Error(timeoutMsg); await page.waitForTimeout(interval); }
      },
    }, "GH745 outline");
    const row = process.env.GH745_ROW || "90";
    const target = `[data-block-id="row-5-${row}"] > .block-main .block-content`;
    for (let i = 0; i < 400; i++) {
      if (await page.locator(target).count()) break;
      await page.evaluate(() => { document.querySelector(".main-content").scrollTop += 600; });
      await page.waitForTimeout(40);
    }
    await page.locator(target).waitFor();
    // Visit every image above the target, then wheel the editor toward the top.
    await page.waitForTimeout(500);
    await page.evaluate((selector) => {
      const sc = document.querySelector(".main-content");
      sc.scrollTop += document.querySelector(selector).getBoundingClientRect().top - sc.getBoundingClientRect().top - 600;
    }, target);
    await page.waitForTimeout(600);
    await page.mouse.move(1000, 400);
    await page.mouse.wheel(0, 480);
    await page.waitForTimeout(700);
    await page.locator(target).click();
    await page.waitForSelector("textarea.block-editor");
    await page.evaluate((place) => {
      const t = document.querySelector("textarea.block-editor");
      t.focus({ preventScroll: true }); const at = place === "start" ? 0 : place === "end" ? t.value.length : 10;
      t.setSelectionRange(at, at);
    }, caret);
    await page.waitForTimeout(300);
    for (const key of keys) {
      const before = await page.evaluate(() => {
        const sc = document.querySelector(".main-content"), ed = document.querySelector("textarea.block-editor");
        window.__samples = []; window.__focus = [];
        window.__aboveWindows = [...document.querySelectorAll("[data-outline-window]")]
          .filter((w) => w.getBoundingClientRect().bottom <= ed.getBoundingClientRect().top);
        // Every mounted window/block before the edit: any that is detached afterwards was remounted.
        window.__allWindows = [...document.querySelectorAll("[data-outline-window]")];
        window.__windowLabel = new Map(window.__allWindows.map((w) => [w, `${w.parentElement.closest(".ls-block")?.dataset.blockId ?? "root"}:${w.querySelector(".ls-block")?.dataset.blockId ?? "spacer"}`]));
        window.__allBlocks = [...document.querySelectorAll(".ls-block")];
        window.__t0 = null; window.__frames = [];
        window.addEventListener("keydown", () => { window.__t0 ??= performance.now(); }, { capture: true, once: true });
        const snapshot = () => ({ scrollTop: sc.scrollTop, scrollHeight: sc.scrollHeight, editorTop: document.querySelector("textarea.block-editor")?.getBoundingClientRect().top - sc.getBoundingClientRect().top,
          missing: document.querySelectorAll(".inline-image-missing").length });
        const original = HTMLElement.prototype.focus;
        if (!window.__focusWrapped) {
          window.__focusWrapped = true;
          HTMLElement.prototype.focus = function (...args) {
            window.__focus.push({ ...snapshot(), windows: [...document.querySelectorAll("[data-outline-window]")].map((w) => ({
              parent: w.parentElement.closest(".ls-block")?.dataset.blockId ?? "root", first: w.querySelector(".ls-block")?.dataset.blockId,
              height: w.clientHeight, spacer: w.firstElementChild?.getAttribute("aria-hidden") === "true",
            })) });
            return original.apply(this, args);
          };
        }
        const end = performance.now() + 700;
        const tick = (ts) => { window.__frames.push(ts); window.__samples.push(snapshot()); if (performance.now() < end) requestAnimationFrame(tick); };
        requestAnimationFrame(tick);
        return { ...snapshot(), editorHeight: ed.clientHeight };
      });
      if (key === "type") {
        const samples = [];
        for (let i = 0; i < 40; i++) {
          await page.evaluate(() => { window.__typeCost = new Promise((resolve) => {
            window.addEventListener("keydown", () => { const t0 = performance.now();
              requestAnimationFrame(() => requestAnimationFrame(() => resolve(performance.now() - t0))); }, { capture: true, once: true }); }); });
          await page.keyboard.press("a");
          samples.push(await page.evaluate(() => window.__typeCost));
          await page.waitForTimeout(30);
        }
        samples.sort((a, b) => a - b);
        console.log(JSON.stringify({ name, key, typingKeyToSecondFrameMs: { median: +samples[20].toFixed(1), p95: +samples[38].toFixed(1), max: +samples[39].toFixed(1) } }));
        continue;
      }
      if (key === "drag") {
        const bullet = page.locator(`[data-block-id="row-5-${row}"] > .block-main .bullet-container`);
        const from = await bullet.boundingBox();
        const to = await page.locator(`[data-block-id="row-5-${Number(row) + 2}"] > .block-main`).boundingBox();
        await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
        await page.mouse.down();
        await page.mouse.move(to.x + to.width / 2, to.y + to.height - 2, { steps: 8 });
        await page.mouse.up();
        // A drag ends editing; reopening the moved block also exercises reveal.
        await page.locator(target).click();
      } else await page.keyboard.press(key);
      await page.waitForTimeout(900);
      if (late) {
        await page.evaluate(() => window.__gh745Release());
        await page.waitForTimeout(1500);
        await page.evaluate(() => {
          const sc = document.querySelector(".main-content");
          window.__samples.push({ scrollTop: sc.scrollTop, scrollHeight: sc.scrollHeight,
            editorTop: document.querySelector("textarea.block-editor")?.getBoundingClientRect().top - sc.getBoundingClientRect().top,
            missing: document.querySelectorAll(".inline-image-missing").length });
        });
      }
      const after = await page.evaluate(() => ({ samples: window.__samples, focus: window.__focus,
        remountedWindowsAbove: window.__aboveWindows.filter((w) => !w.isConnected).length,
        remountedWindows: window.__allWindows.filter((w) => !w.isConnected).length, windowsBefore: window.__allWindows.length,
        remountedWindowLabels: window.__allWindows.filter((w) => !w.isConnected).map((w) => window.__windowLabel.get(w)),
        remountedBlocks: window.__allBlocks.filter((b) => !b.isConnected).length, blocksBefore: window.__allBlocks.length,
        firstFrameAfterKeyMs: window.__t0 === null ? null : Math.round((window.__frames.find((f) => f > window.__t0) ?? NaN) - window.__t0),
        maxFrameGapMs: Math.round(Math.max(0, ...window.__frames.slice(1).map((f, i) => f - window.__frames[i]))),
        viewportHeight: document.querySelector(".main-content").clientHeight }));
      const last = after.samples.at(-1);
      const result = { name, key, before, after: last, focus: after.focus,
        remountedWindowsAbove: after.remountedWindowsAbove, remountedWindows: after.remountedWindows, windowsBefore: after.windowsBefore,
        remountedBlocks: after.remountedBlocks, remountedWindowLabels: after.remountedWindowLabels, blocksBefore: after.blocksBefore, firstFrameAfterKeyMs: after.firstFrameAfterKeyMs, maxFrameGapMs: after.maxFrameGapMs,
        caretVisible: last.editorTop >= 0 && last.editorTop + before.editorHeight <= after.viewportHeight,
        maxScrollDelta: Math.max(...after.samples.map((s) => Math.abs(s.scrollTop - before.scrollTop))),
        maxMissing: late ? 0 : Math.max(...after.samples.map((s) => s.missing), ...after.focus.map((s) => s.missing)),
        samples: after.samples };
      results.push(result);
      console.log(JSON.stringify({ ...result, focus: result.focus.map(({ windows, ...rest }) => rest), samples: undefined }));
      await page.screenshot({ path: `${out}/${name}-${key.replaceAll("+", "-")}.png` });
    }
    await page.close();
  }
  fs.writeFileSync(`${out}/results.json`, JSON.stringify(results, null, 2));
  if (results.some((r) => !r.caretVisible || (r.name !== "late-image" && r.maxScrollDelta > 100) || r.maxMissing > 0
    || (r.name === "drag" && r.remountedWindowsAbove > 0))) process.exitCode = 1;
} finally { await browser.close(); await server.close(); }
