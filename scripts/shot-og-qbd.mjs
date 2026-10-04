// Focused Chromium proof for GH #612/#610; renders real Block/Embed components.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { writeFileSync, rmSync } from "node:fs";
import assert from "node:assert/strict";
const html = ".qbd-shot.html", entry = ".qbd-shot.tsx", port = 5218;
writeFileSync(html, `<html data-theme="light"><body><div id="root"></div><script type="module" src="/${entry}"></script></body></html>`);
writeFileSync(entry, `
import { render } from "solid-js/web";
import "./src/styles/theme.css";
import "./src/styles/app.css";
import { initParser } from "./src/render/parse";
import { loadSingle } from "./src/document/workingSet";
import { Block } from "./src/components/Block";
import { backend } from "./src/backend";
await initParser();
const target = { id: "qbd-source", raw: "Embedded source", collapsed: false, children: [
  { id: "qbd-child", raw: "Embedded child", collapsed: false, children: [] }
] };
const page = { name: "QBD", kind: "page", title: "QBD", pre_block: null, blocks: [
  { id: "qbd-props", raw: "what-is-this:: before 1\\nwhat-is-this:: before 2\\nclass:: test before\\nBlock property content\\nclass:: test after", collapsed: false, children: [] },
  target, { id: "qbd-host", raw: "{{embed ((qbd-source))}}", collapsed: false, children: [] }
] };
backend().resolveBlocks = async () => [{ page: "QBD", kind: "page", blocks: [target] }];
loadSingle(page);
render(() => <main style="max-width:700px;margin:40px auto"><h1>QBD rendering parity</h1>
  <Block id="qbd-props" /><h2>Ordinary source</h2><Block id="qbd-source" />
  <h2>Embedded occurrence</h2><Block id="qbd-host" /></main>, document.getElementById("root"));
`);
const server = spawn("./node_modules/.bin/vite", ["--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
try {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://localhost:${port}/${html}`)).ok) break; } catch {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const tab = await browser.newPage({ viewport: { width: 960, height: 680 } });
  await tab.goto(`http://localhost:${port}/${html}`);
  await tab.waitForSelector(".embed-block .ls-block");
  for (const theme of ["light", "dark"]) {
    await tab.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    const evidence = await tab.evaluate(() => {
      const embed = document.querySelector(".embed-block");
      const rows = [...document.querySelectorAll('[data-block-id="qbd-props"] .prop')];
      return { embed: getComputedStyle(embed).backgroundColor, page: getComputedStyle(document.body).backgroundColor,
        rows: rows.map(row => ({ top: row.getBoundingClientRect().top, bottom: row.getBoundingClientRect().bottom, text: row.textContent, linked: !!row.querySelector("a.page-ref") })) };
    });
    assert.notEqual(evidence.embed, "rgba(0, 0, 0, 0)", `${theme}: embed must have shading`);
    assert.notEqual(evidence.embed, evidence.page, `${theme}: shading differs from page`);
    assert.equal(evidence.rows.length, 2);
    assert.ok(evidence.rows.every(row => row.linked));
    assert.ok(evidence.rows[1].top >= evidence.rows[0].bottom, "properties occupy separate rows");
    assert.match(evidence.rows[0].text, /before 2/);
    await tab.screenshot({ path: `/tmp/og-qbd-${theme}.png` });
    console.log(theme, JSON.stringify(evidence));
  }
} finally {
  await browser?.close(); server.kill(); rmSync(html, { force: true }); rmSync(entry, { force: true });
}
