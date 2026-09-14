// Real Block rendered-to-editor drag compared with a native textarea click at
// the same endpoint. The independent native control owns the pixel-to-offset
// oracle; no copy of the production caret algorithm is used here.
import { chromium } from "playwright";
import assert from "node:assert/strict";
import { realpathSync } from "node:fs";
import { createServer } from "vite";

const server = process.env.TINE_PROBE_URL ? null : await createServer({
  server: { host: "127.0.0.1", port: 5517, strictPort: true, fs: { allow: [
    process.cwd(), realpathSync("node_modules/inter-ui"), realpathSync("node_modules/@fontsource-variable/noto-emoji"),
  ] } },
});
await server?.listen();
const browser = await chromium.launch({ args: ["--no-sandbox"] });
const results = [];
try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 1000 } });
  page.setDefaultTimeout(5000);
  const cases = [
    ...["wrap", "hard", "unicode", "code"].flatMap(shape => [1, 2, 4, 6].map(row => ({shape, row, width: 420, zoom: 1}))),
    ...[300, 650].flatMap(width => [.9, 1.25].flatMap(zoom => [2, 4].map(row => ({shape: "wrap", row, width, zoom})))),
  ];
  for (const {shape, row, width, zoom} of cases) {
      await page.goto(`${process.env.TINE_PROBE_URL ?? "http://127.0.0.1:5517"}/scripts/fixtures/drag-selection/index.html?shape=${shape}&width=${width}&zoom=${zoom}`);
      await page.locator(".block-content").waitFor();
      await page.evaluate(() => document.fonts.ready);
      const content = await page.locator(".block-content").boundingBox();
      await page.mouse.move(content.x + 60 * zoom, content.y + 12 * zoom);
      await page.mouse.down();
      const editor = page.locator("textarea.block-editor");
      await editor.waitFor();
      const target = await editor.evaluate((ta, {row, zoom}) => {
        const rect = ta.getBoundingClientRect(), cs = getComputedStyle(ta);
        return { x: rect.left + 70 * zoom, y: rect.top + (parseFloat(cs.paddingTop) + parseFloat(cs.borderTopWidth) + parseFloat(cs.lineHeight) * (row + .5)) * zoom, anchor: ta.selectionStart };
      }, {row, zoom});
      await page.mouse.move(target.x, target.y, { steps: 5 });
      await page.mouse.up();
      const actual = await editor.evaluate(ta => ({ start: ta.selectionStart, end: ta.selectionEnd, direction: ta.selectionDirection }));
      await editor.evaluate((ta, zoom) => {
        const clone = ta.cloneNode(true);
        const cs = getComputedStyle(ta), rect = ta.getBoundingClientRect();
        for (const key of cs) clone.style.setProperty(key, cs.getPropertyValue(key));
        clone.value = ta.value;
        Object.assign(clone.style, { position: "fixed", left: `${rect.left / zoom}px`, top: `${rect.top / zoom}px`, margin: "0", zIndex: "2147483647" });
        clone.id = "native-oracle";
        clone.addEventListener("mousedown", event => event.stopPropagation());
        document.body.append(clone);
      }, zoom);
      await page.mouse.click(target.x, target.y);
      const expected = await page.locator("#native-oracle").evaluate(ta => ta.selectionStart);
      results.push({ shape, row, width, zoom, anchor: target.anchor, expected, actual, error: actual.end - expected });
      console.log(JSON.stringify(results.at(-1)));
  }
  assert.ok(results.every(r => r.actual.start === r.anchor && r.actual.end === r.expected), "drag endpoint must follow the same native text position as the pointer");
} finally { await browser.close(); await server?.close(); }
