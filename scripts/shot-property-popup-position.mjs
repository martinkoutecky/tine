// GH #737: the property key/value completion popup hangs against the CARET'S line
// of a multi-line property editor (a page-properties block with many rows), not
// below the editor's last row; it flips above the line near the bottom edge and
// stays inside the viewport's right edge. jsdom has no layout, so this measures
// the REAL engine over the real Block over the browser mock backend.
// Usage: source scripts/env.sh && node scripts/shot-property-popup-position.mjs
import { chromium } from "playwright";
import { createServer } from "vite";
import { mkdirSync, writeFileSync } from "node:fs";
import assert from "node:assert/strict";

const out = "artifacts/prop-values";
mkdirSync(out, { recursive: true });
writeFileSync(`${out}/popup-fixture.html`, `<html data-theme="light"><meta name="viewport" content="width=device-width,initial-scale=1"><body><div id="root"></div><script type="module" src="/${out}/popup-fixture.tsx"></script></body></html>`);
writeFileSync(`${out}/popup-fixture.tsx`, `
import { render } from "solid-js/web";
import "/src/styles/theme.css";
import "/src/styles/app.css";
import { initParser } from "/src/render/parse";
import { loadSingle } from "/src/document/workingSet";
import { startEditing } from "/src/editorController";
import { Block } from "/src/components/Block";
await initParser();
let dispose;
window.fixture = ({ rows, titled, sidebar, top }) => {
  dispose?.();
  const lines = Array.from({ length: rows }, (_, i) => "prop" + String(i + 1).padStart(2, "0") + ":: value " + (i + 1));
  const raw = (titled ? "A titled block\\n" : "") + lines.join("\\n");
  loadSingle({ name: "Popup", title: "Popup", kind: "page", format: "md", pre_block: null, blocks: [{ id: "props", raw, collapsed: false, children: [] }] });
  dispose = render(() => <main data-probe-scroller class={sidebar ? "right-sidebar" : "main-content pane-main-content"} style={"position:relative;margin:" + top + "px 12px 0;height:520px;overflow:auto;padding:0;width:calc(100% - 24px);max-width:none;flex-shrink:0"}><section style="padding:0 12px"><Block id="props" /></section></main>, document.getElementById("root"));
  startEditing("props", 0);
};
`);

const server = await createServer({ server: { port: 5262, strictPort: true } });
await server.listen();
let browser;
const results = [];
const failures = [];
try {
  browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
  const page = await browser.newPage();
  await page.goto(`http://localhost:5262/${out}/popup-fixture.html`);
  await page.waitForFunction(() => !!window.fixture);
  const cases = [
    // [label, viewport, options, caret line index, expected placement]
    ["page-props-main-line3", { width: 1100, height: 900 }, { rows: 14, titled: false, sidebar: false, top: 20 }, 2, "below"],
    ["block-props-main-line4", { width: 1100, height: 900 }, { rows: 14, titled: true, sidebar: false, top: 20 }, 3, "below"],
    ["page-props-sidebar-line3", { width: 1100, height: 900 }, { rows: 14, titled: false, sidebar: true, top: 20 }, 2, "below"],
    ["page-props-phone-line3", { width: 390, height: 800 }, { rows: 14, titled: false, sidebar: false, top: 20 }, 2, "below"],
    ["page-props-narrow-right-edge-line3", { width: 280, height: 800 }, { rows: 14, titled: false, sidebar: false, top: 20 }, 2, "below"],
    ["page-props-main-near-bottom", { width: 1100, height: 520 }, { rows: 14, titled: false, sidebar: false, top: 20 }, 12, "above"],
    ["page-props-sidebar-near-bottom", { width: 1100, height: 520 }, { rows: 14, titled: false, sidebar: true, top: 20 }, 12, "above"],
  ];
  for (const [label, viewport, options, lineIndex, expected] of cases) {
    await page.setViewportSize(viewport);
    await page.evaluate((o) => window.fixture(o), options);
    const ta = page.locator("textarea.block-editor");
    await ta.waitFor({ timeout: 10_000 });
    // Put the caret at the end of an inner line, then type its property delimiter.
    await page.evaluate((idx) => {
      const t = document.querySelector("textarea.block-editor");
      const lines = t.value.split("\n");
      lines[idx] = "aaa";
      t.value = lines.join("\n");
      const end = lines.slice(0, idx + 1).join("\n").length;
      t.focus();
      t.setSelectionRange(end, end);
      t.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "a" }));
    }, lineIndex);
    await page.keyboard.type("::");
    await page.locator(".autocomplete").first().waitFor({ timeout: 10_000 });
    const measure = () => page.evaluate((idx) => {
      const t = document.querySelector("textarea.block-editor");
      const r = t.getBoundingClientRect();
      const cs = getComputedStyle(t);
      const scale = r.height / t.offsetHeight;
      const lh = (parseFloat(cs.lineHeight) || 20) * scale;
      const lineTop = r.top + (t.clientTop + parseFloat(cs.paddingTop)) * scale + idx * lh - t.scrollTop * scale;
      const p = document.querySelector(".autocomplete").getBoundingClientRect();
      return {
        textarea: { top: r.top, bottom: r.bottom }, lh, lineTop, lineBottom: lineTop + lh,
        popup: { top: p.top, bottom: p.bottom, left: p.left, right: p.right },
        vw: window.innerWidth, vh: window.innerHeight,
      };
    }, lineIndex);
    const check = async (stage) => {
      const m = await measure();
      const slack = m.lh * 1.25;
      const near = expected === "below"
        ? Math.abs(m.popup.top - m.lineBottom) <= slack
        : Math.abs(m.popup.bottom - m.lineTop) <= slack;
      const inside = m.popup.top >= -0.5 && m.popup.bottom <= m.vh + 0.5 && m.popup.left >= -0.5 && m.popup.right <= m.vw + 0.5;
      const row = { label, stage, expected, near, inside, ...m };
      results.push(row);
      if (!near || !inside) {
        failures.push(row);
        await page.screenshot({ path: `${out}/popup-${label}-${stage}.png` });
      }
    };
    await check("key-popup");
    // Choose the new key; then type a value: the VALUE popup is anchored the same way.
    await page.keyboard.press("Enter");
    await page.keyboard.type("zz");
    await page.locator(".autocomplete").first().waitFor({ timeout: 10_000 });
    await check("value-popup");
    await page.screenshot({ path: `${out}/popup-${label}.png` });
    await page.keyboard.press("Escape");
  }
  writeFileSync(`${out}/popup-geometry.json`, JSON.stringify({ results, failures }, null, 2));
  assert.equal(failures.length, 0, `popup misplaced in ${failures.length} measurements: ${JSON.stringify(failures.map((f) => [f.label, f.stage]))}`);
  console.log(`PASS: ${results.length} real-browser popup placements`);
} finally {
  await browser?.close();
  await server.close();
}
