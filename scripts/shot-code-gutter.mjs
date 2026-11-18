// Synthetic routed PageView; measure the painted number, its box and code separation.
// node scripts/shot-code-gutter.mjs [artifact-directory]
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";

const out = process.argv[2] ?? "target/artifacts/code-gutter";
mkdirSync(out, { recursive: true });
const port = 24579;
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
const results = [];
try {
  for (let i = 0; ; i++) {
    if (server.exitCode !== null) throw new Error(`Vite exited ${server.exitCode}`);
    try { if ((await fetch(`http://localhost:${port}/`)).ok) break; } catch {}
    if (i === 120) throw new Error("Vite did not start");
    await delay(250);
  }
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  let page = await browser.newPage({ viewport: { width: 1000, height: 800 }, permissions: ["clipboard-read", "clipboard-write"] });
  await page.goto(`http://localhost:${port}/scripts/fixtures/code-gutter.html`, { waitUntil: "domcontentloaded" });
  await page.locator(".code-gutter").waitFor();
  if (process.argv.includes("--labels")) {
    for (const width of [1000, 390]) {
      if (width === 390) {
        await page.close();
        page = await browser.newPage({ viewport: { width, height: 800 }, hasTouch: true, permissions: ["clipboard-read", "clipboard-write"] });
        await page.goto(`http://localhost:${port}/scripts/fixtures/code-gutter.html`, { waitUntil: "domcontentloaded" });
        await page.locator(".code-gutter").waitFor();
      }
      for (const lang of ["js", ""]) for (const kind of ["textarea", "cm6"]) for (const mixed of [false, true]) for (const editing of [false, true]) {
      await page.setViewportSize({ width, height: 800 });
      await page.evaluate(([kind, mixed, lang, editing]) => {
        window.showCode(2, kind, mixed, false, lang);
        if (!editing) window.finishCode();
      }, [kind, mixed, lang, editing]);
      const label = page.getByRole("button", { name: "Edit code language", exact: true });
      await label.waitFor();
      assert.equal(await label.textContent(), lang || "plain");
      if (editing) await page.waitForFunction(() => document.activeElement?.matches("textarea.block-editor, .cm-content"));
      if (editing) { if (width === 390) await label.tap(); else await label.click(); }
      else { await page.evaluate(() => window.selectCode()); await label.focus(); await page.keyboard.press(mixed ? "Space" : "Enter"); }
      await page.waitForFunction(() => window.codeState().value === window.codeState().raw);
      const state = await page.evaluate(() => window.codeState());
      assert.equal(state.caret, (mixed ? "Before\n".length : 0) + 3);
      await page.keyboard.type("text");
      const saved = await page.evaluate(() => window.finishCode());
      assert.equal(saved, state.raw.slice(0, state.caret) + "text" + state.raw.slice(state.caret));
      // Check Copy separately so focusing it cannot blur the editor before the label test.
      await page.evaluate(([kind, mixed, lang, editing]) => {
        window.showCode(2, kind, mixed, false, lang);
        if (!editing) window.finishCode();
      }, [kind, mixed, lang, editing]);
      const card = page.locator(".code-block, .code-editor-region").filter({ has: label });
      await card.hover();
      const copy = card.getByRole("button", { name: "Copy code", exact: true });
      if (editing) await page.waitForFunction(() => document.activeElement?.matches("textarea.block-editor, .cm-content"));
      else await copy.focus();
      const labelBox = await label.boundingBox(), copyBox = await copy.boundingBox();
      assert.ok(labelBox.x + labelBox.width <= copyBox.x, "label leaves Copy's hit target clear");
      if (width === 390) await copy.tap(); else await copy.click();
      assert.equal((await page.evaluate(() => navigator.clipboard.readText())).trimEnd(), "const n0 = 0;\nconst n1 = 1;");
      if (!editing && kind === "textarea" && !mixed) await page.screenshot({ path: `${out}/label-${width}-${lang || "plain"}.png` });
      results.push({ width, lang, kind, mixed, editing, caret: state.caret });
      }
    }
    writeFileSync(`${out}/labels.json`, JSON.stringify(results, null, 2) + "\n");
    console.log(`PASS: ${results.length} language label activations and source edits`);
  } else {
  if (process.argv[3]) await page.addStyleTag({ content: readFileSync(process.argv[3], "utf8") });
  // GH #970: one number per source line, level with that line's first row, when a line wraps: by the
  // wrap setting, or because a theme or custom.css wraps `code` while the setting is off.
  // A theme's rule may select the code by descendant or by child; both must reach the numbers (review of fb13bd95d).
  const wrapRule = sel => `${sel} { white-space: pre-wrap !important; overflow-wrap: anywhere !important; }`;
  // "live": a theme switch after the card is shown, changing the code's font while it wraps (review of 3764978ba).
  const themeWrap = { themed: [wrapRule("pre code")], child: [wrapRule("pre > code")],
    live: [wrapRule("pre > code"), "pre > code { font-family: serif !important; }"],
    // white-space modes that collapse a whitespace-only line, or break a long run of spaces (review of 50aea73ba).
    preline: ["pre > code { white-space: pre-line !important; }"],
    breakspaces: ["pre > code { white-space: break-spaces !important; }"] };
  const aligned = [];
  for (const wrap of [false, true]) for (const kind of ["view", "textarea", "cm6"]) for (const name of ["long", "themed", "child", "live", "preline", "breakspaces"]) {
    if (name !== "long" && kind !== "view") continue; // a theme's `pre code` rule never reaches an editor
    // The classic editor paints no per-line boxes; its height is compared with the numbers' only
    // unwrapped, because its font stack differs from the gutter's on a host without the mono fonts.
    if (kind === "textarea" && wrap) continue;
    await page.evaluate(([k, w]) => window.showLong(k, w), [kind, wrap]);
    await page.locator(".code-gutter .calc-lineno").nth(1).waitFor();
    const settle = () => page.evaluate(() => document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))));
    const themed = [];
    for (const content of themeWrap[name] ?? []) { await settle(); themed.push(await page.addStyleTag({ content })); }
    await settle();
    const r = await page.evaluate(([kind]) => {
      const nums = [...document.querySelectorAll(".code-gutter .calc-lineno")];
      const top = el => el.getBoundingClientRect().top;
      let rows;
      if (kind === "cm6") rows = [...document.querySelectorAll(".cm-content > .cm-line")].map(top);
      else if (kind === "view") {
        // First painted row of every line of the code text, by walking its text to each line start.
        const code = document.querySelector(".code-block > code > .code-text") ?? document.querySelector(".code-block > code");
        const walker = document.createTreeWalker(code, NodeFilter.SHOW_TEXT);
        const starts = []; let atStart = true, node;
        while ((node = walker.nextNode())) {
          for (let i = 0; i < node.data.length; i++) {
            if (atStart) { starts.push([node, i]); atStart = false; }
            if (node.data[i] === "\n") atStart = true;
          }
        }
        // A whitespace-only line whose spaces collapse (pre-line) has no glyph box to measure: null, not compared.
        rows = starts.map(([n, i]) => { const range = document.createRange(); range.setStart(n, i); range.setEnd(n, i + 1);
          return range.getBoundingClientRect().height ? top(range) : null; });
      } else {
        // A textarea paints no per-line boxes: its content height must equal the numbered height.
        const ta = document.querySelector("textarea.block-editor"), s = getComputedStyle(ta);
        const g = document.querySelector(".code-gutter").getBoundingClientRect();
        return { numbers: nums.length, textareaOk: Math.abs(ta.scrollHeight - parseFloat(s.paddingTop) - parseFloat(s.paddingBottom) - g.height) <= 1.5,
          gutterH: g.height, textH: ta.scrollHeight };
      }
      // A number's ink sits at a constant offset inside its row's box, so compare spacing from the first.
      return { numbers: nums.length, lines: rows.length,
        drift: nums.map((n, i) => rows[i] == null ? 0 : Math.round(top(n) - rows[i] - (top(nums[0]) - rows[0]))) };
    }, [kind]);
    aligned.push({ wrap, kind, name, ...r });
    await page.screenshot({ path: `${out}/aligned-${wrap ? "wrap" : "nowrap"}-${kind}-${name}.png` });
    for (const el of themed) await el.evaluate(node => node.remove());
  }
  writeFileSync(`${out}/aligned.json`, JSON.stringify(aligned, null, 2) + "\n");
  const misaligned = aligned.filter(r => r.kind === "textarea" ? r.numbers !== 14 || !r.textareaOk
    : r.numbers !== 14 || r.lines !== 14 || r.drift.some(d => Math.abs(d) > 1));
  assert.deepEqual(misaligned, [], "every number sits level with its own line, whatever wraps inside it");
  console.log(`PASS: ${aligned.length} wrapped long-line layouts keep numbers level`);
  for (const wrap of [false, true]) for (const kind of ["view", "textarea", "cm6"]) for (const mixed of [false, true]) for (const count of [9, 10, 99, 100, 1000]) {
    await page.evaluate(([n, k, m, w]) => window.showCode(n, k, m, w), [count, kind, mixed, wrap]);
    const last = page.locator(".code-gutter .calc-lineno").last();
    await last.waitFor();
    assert.equal(await last.textContent(), String(count));
    await last.scrollIntoViewIfNeeded();
    const geometry = await last.evaluate(el => {
      const box = el.getBoundingClientRect();
      const range = document.createRange(); range.selectNodeContents(el);
      const ink = range.getBoundingClientRect();
      const bands = [];
      for (const rect of range.getClientRects()) {
        if (rect.width && !bands.some(b => rect.top < b.bottom && rect.bottom > b.top)) bands.push(rect);
      }
      const rows = bands.length;
      const card = el.closest(".code-block, .code-editor-region");
      const gutter = el.closest(".code-gutter").getBoundingClientRect();
      const outer = (card.matches(".code-editor-region:not(.mixed-code-region)") ? card.parentElement : card).getBoundingClientRect();
      const editor = card.querySelector(".block-editor, .cm-content, code.hljs");
      const text = editor.getBoundingClientRect();
      const codeLeft = text.left + (editor.matches("textarea") ? parseFloat(getComputedStyle(editor).paddingLeft) : 0);
      return { number: el.textContent, rows, boxWidth: box.width, inkWidth: ink.width,
        inside: ink.left >= outer.left && ink.right <= outer.right,
        separate: ink.right < codeLeft, aligned: Math.abs(codeLeft - gutter.left) <= 1.5,
        codeLeft, numberRight: ink.right };
    });
    results.push({ wrap, kind, mixed, count, ...geometry });
    if ([10, 100, 1000].includes(count)) await page.screenshot({ path: `${out}/${wrap ? "wrap" : "nowrap"}-${kind}-${mixed ? "mixed" : "whole"}-${count}.png` });
  }
  writeFileSync(`${out}/geometry.json`, JSON.stringify(results, null, 2) + "\n");
  const failures = results.filter(r => r.rows !== 1 || r.inkWidth > r.boxWidth + .5 || !r.inside || !r.separate || !r.aligned);
  assert.deepEqual(failures, [], "each complete number fits its box inside the card, clear of the code");
  console.log(`PASS: ${results.length} code gutter layouts`);
  const numbered = [];
  for (const kind of ["textarea", "cm6"]) {
    await page.evaluate(k => window.showNumbered(k), kind);
    const editor = page.locator(kind === "cm6" ? ".cm-content" : "textarea.block-editor");
    await editor.waitFor(); await editor.focus();
    await page.keyboard.press("Control+Home");
    await page.keyboard.type("1. ");
    await page.keyboard.press("End"); await page.keyboard.press("Enter");
    await page.keyboard.type("Second line");
    await page.keyboard.press("Escape");
    await page.locator("li.md-list-item").waitFor();
    const numbers = await page.locator("li.md-list-item").evaluateAll(items => items.map(item => {
      // HTML's ordinal algorithm: an explicit value wins, otherwise count from ol.start.
      let number = item.parentElement.start;
      for (const sibling of item.parentElement.children) {
        if (sibling.hasAttribute("value")) number = sibling.value;
        if (sibling === item) return number;
        number++;
      }
    }));
    numbered.push({ kind, numbers });
    await page.screenshot({ path: `${out}/numbered-${kind}.png` });
  }
  writeFileSync(`${out}/numbered.json`, JSON.stringify(numbered, null, 2) + "\n");
  assert.deepEqual(numbered.map(r => r.numbers), [[2], [2]], "the continued line still displays 2 after editing");
  console.log("PASS: numbered continuation in both editors");
  }
} finally { await browser?.close(); server.kill(); }
