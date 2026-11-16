// GH #770: collapsed page embeds align the glyph with the first title line.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import assert from "node:assert/strict";
import { setTimeout as sleep } from "node:timers/promises";

const port = Number(process.env.PARITY_PORT || 52809);
const out = `artifacts/parity-09/${process.env.PARITY_PHASE || "after"}`;
mkdirSync(out, { recursive: true });
const server = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "--port", String(port), "--strictPort"], { stdio: "ignore" });
let browser;
const results = [];
try {
  let ready = false;
  for (let i = 0; i < 120; i++) {
    try { ready = (await fetch(`http://localhost:${port}`)).ok; } catch {}
    if (ready) break;
    await sleep(250);
  }
  assert(ready, "Vite ready");
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  for (const width of [900, 390]) {
    const page = await browser.newPage({ viewport: { width, height: 760 } });
    await page.goto(`http://localhost:${port}`);
    await page.waitForSelector(".ls-block");
    await page.evaluate(async () => {
      const { backend } = await import("/src/backend.ts");
      const { openInNewTab } = await import("/src/router.ts");
      const title = "Long page title with enough words to wrap onto several lines inside a collapsed page embed";
      const leaf = (id, raw) => ({ id, raw, collapsed: false, children: [] });
      const original = backend().getPage.bind(backend());
      backend().getPage = async (name, kind) => name === "Parity title host" || name === title
        ? { id: `pages/${name}.md`, name, title: name, kind: "page", format: "md", pre_block: null,
          blocks: name === title ? [leaf("source-parity-title", "Source")]
            : [{ ...leaf("host-parity-title", `{{embed [[${title}]]}}`), collapsed: true }] }
        : original(name, kind);
      openInNewTab({ kind: "page", name: "Parity title host", pageKind: "page" }, true);
    });
    const header = page.locator(".main-content .embed-page-header");
    await header.waitFor();
    // Bound the container as a narrow pane would, including at desktop width.
    await header.evaluate(el => { el.style.maxWidth = "260px"; });
    const geometry = await header.evaluate(el => {
      const glyph = el.querySelector("svg").getBoundingClientRect();
      const link = el.querySelector(".page-ref");
      const range = document.createRange(); range.selectNodeContents(link);
      const lines = [...range.getClientRects()].filter(rect => rect.height > 0);
      const first = lines[0];
      return { glyphCenter: glyph.top + glyph.height / 2, firstTop: first.top, firstBottom: first.bottom,
        titleHeight: link.getBoundingClientRect().height, lineHeight: parseFloat(getComputedStyle(link).lineHeight) };
    });
    await page.screenshot({ path: `${out}/embed-title-${width}.png` });
    results.push({ width, ...geometry });
    await page.close();
  }
  writeFileSync(`${out}/geometry.json`, JSON.stringify(results, null, 2));
  for (const row of results) {
    assert(row.titleHeight > row.lineHeight * 1.5, `wrapped title at ${row.width}`);
    assert(row.glyphCenter >= row.firstTop && row.glyphCenter <= row.firstBottom, `glyph on first line at ${row.width}: ${JSON.stringify(row)}`);
  }
  console.log("PASS: wrapped page embed glyph on first line at desktop and phone widths");
} finally {
  await browser?.close();
  server.kill();
}
