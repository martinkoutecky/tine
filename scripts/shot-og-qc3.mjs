// GH #474: real Block view/edit geometry and surrounding scroll in Chromium.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { writeFileSync, rmSync } from "node:fs";
import assert from "node:assert/strict";
const html = ".qc3-shot.html", entry = ".qc3-shot.tsx", port = 5223;
writeFileSync(html, `<html data-theme="light"><body><div id="root"></div><script type="module" src="/${entry}"></script></body></html>`);
writeFileSync(entry, `
import { render } from "solid-js/web";
import "./src/styles/inter.css";
import "./src/styles/theme.css";
import "./src/styles/editableEmoji.css";
import "./src/styles/app.css";
import { initParser } from "./src/render/parse";
import { resetStore } from "./src/document";
import { loadSingle } from "./src/document/workingSet";
import { Block } from "./src/components/Block";
import { endEdit } from "./src/editorController";
await initParser();
window.finishEdit = () => endEdit("page-navigation");
let dispose;
window.fixture = (raw, format) => { dispose?.(); endEdit("page-navigation"); resetStore(); loadSingle({ name: "QC3", title: "QC3", kind: "page", format, pre_block: null, blocks: [
{ id: "qc3-before", raw: "Before code", collapsed: false, children: [] },
{ id: "qc3-code", raw, collapsed: false, children: [] },
{ id: "qc3-after", raw: "After code", collapsed: false, children: [] }] });
dispose = render(() => <main style="width:620px;height:420px;overflow:auto;margin:40px auto" id="scroller"><div style="height:450px" />
<Block id="qc3-before"/><Block id="qc3-code"/><Block id="qc3-after"/><div style="height:500px"/></main>, document.getElementById("root")); };
`);
const server = spawn("./node_modules/.bin/vite", ["--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
const evidence = [];
try {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://localhost:${port}/${html}`)).ok) break; } catch {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const tab = await browser.newPage({ viewport: { width: 960, height: 680 } });
  tab.on("pageerror", error => console.error(error));
  tab.setDefaultTimeout(15000);
  await tab.goto(`http://localhost:${port}/${html}`);
  await tab.waitForFunction(() => !!window.fixture);
  const cases = [
    ["short", "```js\nconst x = 1;\nconst y = 2;\n```", "md"],
    ["empty", "```\n```", "md"],
    ["long", "```\n" + "0123456789".repeat(20) + "\nsecond line\n```", "md"],
    ["trailing", "~~~\nfirst\n\n~~~", "md"],
    ["tall", "```js\n" + Array.from({ length: 60 }, (_, i) => `const x${i} = ${i};`).join("\n") + "\n```", "md"],
    ["org", "#+BEGIN_SRC python\nx = 1\ny = 2\n#+END_SRC", "org"],
  ];
  const measure = () => tab.evaluate(() => {
    const rect = (el) => { const r = el.getBoundingClientRect(); return { top: r.top, height: r.height, width: r.width }; };
    const card = document.querySelector('[data-block-id="qc3-code"] pre.code-block, [data-block-id="qc3-code"] textarea');
    const style = getComputedStyle(card);
    return { card: rect(card), after: rect(document.querySelector('[data-block-id="qc3-after"]')), scroll: document.getElementById("scroller").scrollTop, font: style.fontFamily, lineHeight: style.lineHeight, background: style.backgroundColor };
  });
  for (const theme of ["light", "dark"]) for (const [name, raw, format] of cases) {
    await tab.evaluate(([raw, format, theme]) => { document.documentElement.dataset.theme = theme; window.fixture(raw, format); }, [raw, format, theme]);
    await tab.evaluate(() => document.getElementById("scroller").scrollTop = 390);
    await tab.waitForSelector('[data-block-id="qc3-code"] pre.code-block');
    const before = await measure();
    await tab.screenshot({ path: `/tmp/og-qc3-${theme}-${name}-view.png` });
    await tab.locator('[data-block-id="qc3-code"] pre.code-block').click({ position: { x: 35, y: 18 } });
    await tab.waitForSelector('textarea.code-edit');
    await tab.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const editing = await measure();
    await tab.screenshot({ path: `/tmp/og-qc3-${theme}-${name}-edit.png` });
    await tab.evaluate(() => window.finishEdit());
    const after = await measure();
    evidence.push({ theme, name, before, editing, after });
    console.log(JSON.stringify(evidence.at(-1)));
  }
  writeFileSync('/tmp/og-qc3-layout.json', JSON.stringify(evidence, null, 2));
  for (const { name, theme, before, editing, after } of evidence) for (const state of [editing, after]) {
    for (const key of ["height", "width", "top"]) assert.ok(Math.abs(state.card[key] - before.card[key]) <= 1, `${theme}/${name} card ${key}: ${before.card[key]} -> ${state.card[key]}`);
    assert.ok(Math.abs(state.after.top - before.after.top) <= 1, `${theme}/${name} surrounding layout shifts`);
    assert.equal(state.scroll, before.scroll, `${theme}/${name} scroll shifts`);
    assert.equal(state.background, before.background);
  }
} finally {
  await browser?.close(); server.kill(); rmSync(html, { force: true }); rmSync(entry, { force: true });
}
