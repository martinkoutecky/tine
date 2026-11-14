// GH #879 comment 3: routed mock-app input through Chromium's IME API.
import { chromium } from "playwright";
import { spawn, execFileSync } from "node:child_process";
import { once } from "node:events";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

const root = path.resolve(import.meta.dirname, "..");
const out = path.resolve(process.env.SHOT_OUT || "target/artifacts/fix-codeblock-mobile-879/browser");
mkdirSync(out, { recursive: true });
const port = 5397;
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { cwd: root, stdio: ["ignore", "pipe", "pipe"] });
let log = "", browser;
const evidence = { base: execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim(), input: [], presentation: [], errors: [] };
const failures = [];
let serial = 0;
try {
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(log || "Vite startup timed out")), 30000);
    server.stdout.on("data", chunk => { log += chunk; if (log.includes("Local:")) { clearTimeout(timer); resolve(); } });
    server.stderr.on("data", chunk => { log += chunk; });
    server.once("exit", code => { clearTimeout(timer); reject(new Error(`Vite exited ${code}: ${log}`)); });
  });
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  for (const [layout, width, touch] of [["phone", 390, true], ["desktop", 1200, false]]) {
    const tab = await browser.newPage({ viewport: { width, height: 844 }, hasTouch: touch, isMobile: touch });
    tab.on("pageerror", error => evidence.errors.push(String(error)));
    await tab.addInitScript(() => localStorage.setItem("logseq-claude.sidebarOpen", "0"));
    await tab.goto(`http://localhost:${port}`);
    await tab.locator("[data-search-trigger]").waitFor();
    await tab.evaluate(async () => {
      const backend = (await import("/src/backend.ts")).backend();
      const submit = backend.pageSubmit.bind(backend);
      const read = backend.getPageByPath.bind(backend);
      window.gh879Submitted = new Map();
      const byPath = new Map();
      backend.getPageByPath = async path => byPath.has(path) ? { ...byPath.get(path), id: path } : read(path);
      backend.pageSubmit = async (...args) => {
        const result = await submit(...args);
        window.gh879Submitted.set(args[3].name, structuredClone(args[3]));
        byPath.set(args[2], structuredClone(args[3]));
        return result;
      };
      let started;
      window.addEventListener("beforeinput", () => { started = performance.now(); });
      window.addEventListener("input", () => queueMicrotask(() => {
        window.gh879InputMs = performance.now() - started;
      }));
    });
    const cdp = await tab.context().newCDPSession(tab);
    const seed = async raw => {
      const name = `Code input ${++serial}`;
      await tab.evaluate(async ({ name, raw }) => {
        const doc = await import("/src/document/index.ts");
        const dto = { ...doc.emptyPage(name, "page"), blocks: [{ id: crypto.randomUUID(), raw, collapsed: false, children: [] }] };
        await doc.createPage(name, dto);
        (await import("/src/router.ts")).openPage(name);
      }, { name, raw });
      await tab.waitForFunction(async name => (await import("/src/router.ts")).route().name === name
        && [...document.querySelectorAll("h1.page-title")].some(el => el.textContent.includes(name)), name);
      await tab.locator(".main-content .ls-block:visible").first().waitFor();
      return name;
    };
    const editor = tab.locator(".main-content textarea.block-editor:visible");
    const state = () => editor.evaluate(el => ({ value: el.value, start: el.selectionStart, end: el.selectionEnd, card: el.classList.contains("code-edit"), focused: el === document.activeElement, restores: el.gh879Restores, inputMs: window.gh879InputMs }));
    const select = pos => editor.evaluate((el, pos) => { el.focus(); el.setSelectionRange(pos, pos); }, pos);
    const saved = async name => tab.evaluate(async name => {
      const doc = await import("/src/document/index.ts");
      (await import("/src/editorController.ts")).endEdit("page-navigation");
      await doc.flushPage(name);
      // The browser host acknowledges submissions in memory. Native file
      // persistence is covered by e2e-editor-context; this is the accepted DTO.
      return window.gh879Submitted.get(name)?.blocks.map(block => block.raw);
    }, name);
    for (const entry of ["typed", "slash"]) for (const mode of ["direct-digits", "direct-letters", "composition"]) {
      console.log(`${layout}/${entry}/${mode}`);
      const name = await seed("");
      const content = tab.locator(".main-content .ls-block:visible .block-content").first();
      if (touch) await content.tap(); else await content.click();
      await editor.waitFor();
      if (entry === "typed") {
        for (const ch of "```") await cdp.send("Input.insertText", { text: ch });
      } else {
        await cdp.send("Input.insertText", { text: "/code" });
        const command = tab.locator(".autocomplete .ac-item").filter({ hasText: "Code block" }).first();
        await command.waitFor();
        if (touch) await command.tap(); else await command.click();
      }
      await tab.locator(".autocomplete .ac-item").filter({ hasText: "JavaScript" }).first().waitFor();
      await select(4);
      await editor.evaluate(el => {
        const restore = el.setSelectionRange.bind(el);
        el.gh879Restores = 0;
        el.setSelectionRange = (...args) => { el.gh879Restores++; return restore(...args); };
      });
      const snapshots = [];
      const text = mode === "direct-letters" ? "abc" : "123";
      if (mode === "composition") {
        await cdp.send("Input.imeSetComposition", { text, selectionStart: text.length, selectionEnd: text.length });
        snapshots.push({ preedit: await state() });
        await cdp.send("Input.insertText", { text });
        snapshots.push(await state());
      } else {
        for (const ch of text) {
          await cdp.send("Input.insertText", { text: ch });
          // Native selectionchange is delivered after the DOM's input update.
          await tab.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
          snapshots.push(await state());
        }
      }
      await tab.screenshot({ path: path.join(out, `${layout}-${entry}-${mode}.png`) });
      const disk = await saved(name);
      const row = { layout, entry, mode, text, snapshots, saved: disk };
      evidence.input.push(row);
      try {
        assert.deepEqual(disk, [`\`\`\`\n${text}\n\`\`\``]);
        assert.equal(snapshots.at(-1).start, text.length);
        assert.equal(snapshots.at(-1).end, text.length);
        assert.ok(snapshots.at(-1).restores <= 1, "at most one projection restore across the input sequence");
        if (mode !== "composition") snapshots.forEach((s, i) => {
          assert.equal(s.value, text.slice(0, i + 1));
          assert.equal(s.start, i + 1);
          assert.equal(s.end, i + 1);
        });
      } catch (error) { failures.push({ ...row, error: error.message }); }
    }
    // Record the existing presentation rules without choosing a new default.
    for (const raw of ["```js\nx\n```", "```js\nx\n```\nafter", "before\n```js\nx\n```", "```js\n\n```", "```js\nx\n```\n "]) {
      await seed(raw);
      await tab.evaluate(async () => {
        const doc = await import("/src/document/index.ts");
        const route = (await import("/src/router.ts")).route();
        (await import("/src/editorController.ts")).startEditing(doc.pageByName(route.name).roots[0], 0);
      });
      await editor.waitFor();
      const closer = raw.lastIndexOf("```") + 3;
      await select(closer);
      await tab.evaluate(() => document.dispatchEvent(new Event("selectionchange")));
      evidence.presentation.push({ layout, raw, position: "right of closer, source entry", ...await state() });
      await tab.screenshot({ path: path.join(out, `${layout}-source-${serial}.png`) });
      const body = raw.indexOf("\n") + 1 + (raw.startsWith("before") ? "```js\n".length : 0);
      await select(body);
      await tab.evaluate(() => document.dispatchEvent(new Event("selectionchange")));
      evidence.presentation.push({ layout, raw, position: "body after source selection", ...await state() });
      await tab.evaluate(async () => (await import("/src/editorController.ts")).endEdit("page-navigation"));
    }
    // Siblings stay ordinary source editors on their first direct/IME input.
    for (const raw of ["> ", "$$\n\n$$", "| a | b |\n|---|---|\n|   |   |", "```\n\n```", "```calc\n\n```"] ) {
      for (const mode of ["direct", "composition"]) {
        const name = await seed(raw);
        const rawCaret = raw.startsWith(">") ? raw.length : raw.startsWith("|") ? raw.lastIndexOf("\n") + 3 : raw.indexOf("\n") + 1;
        await tab.evaluate(async ({ rawCaret }) => {
          const doc = await import("/src/document/index.ts");
          const route = (await import("/src/router.ts")).route();
          (await import("/src/editorController.ts")).startEditing(doc.pageByName(route.name).roots[0], rawCaret);
        }, { rawCaret });
        await editor.waitFor();
        const before = await state();
        const expected = before.value.slice(0, before.start) + "123" + before.value.slice(before.end);
        if (mode === "composition") await cdp.send("Input.imeSetComposition", { text: "123", selectionStart: 3, selectionEnd: 3 });
        await cdp.send("Input.insertText", { text: "123" });
        const after = await state();
        assert.equal(after.value, expected);
        assert.equal(after.start, before.start + 3);
        const disk = await saved(name);
        const expectedRaw = raw.slice(0, rawCaret) + "123" + raw.slice(rawCaret);
        assert.deepEqual(disk, [expectedRaw]);
        evidence.input.push({ layout, sibling: raw, mode, before, after, saved: disk });
      }
    }
    await tab.close();
  }
  assert.deepEqual(evidence.errors, []);
  assert.deepEqual(failures, [], "first-input regressions");
} catch (error) {
  console.error(error);
  if (browser) for (const [i, tab] of browser.contexts().flatMap(context => context.pages()).entries()) {
    await tab.screenshot({ path: path.join(out, `failure-${i}.png`) });
    evidence.errors.push(await tab.evaluate(() => document.body.innerText.slice(0, 3000)));
  }
  process.exitCode = 1;
} finally {
  writeFileSync(path.join(out, "evidence.json"), JSON.stringify({ ...evidence, failures }, null, 2));
  writeFileSync(path.join(out, "vite.log"), log);
  await browser?.close();
  const stopped = once(server, "exit");
  server.kill();
  await stopped;
}
