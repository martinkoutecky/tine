// GH #929: routed margin comments with graph custom.css, wide and compact.
// npm run build && node scripts/shot-comment-styling.mjs [outDir]
import assert from "node:assert/strict";
import { mkdirSync, writeFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { chromium } from "playwright";

const out = process.argv[2] ?? "target/artifacts/csl-comment-styling-1111";
const port = 20000 + Math.floor(Math.random() * 20000);
const url = `http://localhost:${port}/?regressions`;
const css = `:root {
  --tine-comment-highlight-bg: rgb(253, 230, 138);
  --tine-comment-card-bg: rgb(255, 237, 213);
}
[data-author="claude"] { opacity: 0.65; }
`;
mkdirSync(out, { recursive: true });
writeFileSync(`${out}/custom.css`, css);
const server = spawn("npx", ["vite", "preview", "--port", String(port), "--strictPort"], { stdio: "ignore", detached: true });
let browser;
try {
  let ready = false;
  for (let i = 0; i < 80; i++) {
    try { if ((await fetch(url)).ok) { ready = true; break; } } catch {}
    await sleep(250);
  }
  assert(ready, "preview started");
  browser = await chromium.launch();
  const facts = [];
  for (const [width, custom] of [[1440, false], [1440, true], [900, true], [390, true]]) {
    const context = await browser.newContext({ viewport: { width, height: 900 } });
    await context.addInitScript(value => { globalThis.__tineMockCustomCss = value; }, custom ? css : "");
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(String(error)));
    await page.goto(url);
    await page.locator(".page-title").first().waitFor();
    await page.keyboard.press("Control+k");
    await page.locator(".switcher-input").fill("Margin comments regression");
    await page.locator(".switcher-row").first().click();
    if (await page.locator(".mobile-drawer-scrim").count()) {
      await page.keyboard.press("Escape");
      await page.locator(".mobile-drawer-scrim").waitFor({ state: "detached" });
    }
    const mark = page.locator(".comment-quote-anchor").first();
    await mark.waitFor();
    if (width < 960) {
      await mark.focus();
      await page.keyboard.press("Enter");
    }
    await page.locator('.comment-group [data-author="claude"]').first().waitFor();
    const got = await page.evaluate(() => {
      const source = document.querySelector('.page-blocks [data-author="claude"]');
      const mark = document.querySelector(".comment-quote-anchor");
      const reply = document.querySelector('.comment-group [data-author="claude"]');
      const card = reply.closest(".comment-group");
      const fallbackColor = (element, value) => {
        const probe = document.createElement("span");
        probe.style.background = value;
        element.append(probe);
        const color = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return color;
      };
      return {
        sourceAuthor: source?.getAttribute("data-author"),
        sourceOpacity: source && getComputedStyle(source).opacity,
        replyAuthor: reply?.getAttribute("data-author"),
        replyOpacity: reply && getComputedStyle(reply).opacity,
        highlight: getComputedStyle(mark).backgroundColor,
        card: getComputedStyle(card).backgroundColor,
        fallbackHighlight: fallbackColor(mark, `color-mix(in srgb, var(--accent) ${mark.matches(".page-blocks.margin-active .active") ? 30 : 14}%, transparent)`),
        fallbackCard: fallbackColor(card, "color-mix(in srgb, var(--accent) 6%, var(--bg-primary, transparent))"),
        customCss: document.querySelector("#tine-custom-css")?.textContent,
      };
    });
    assert.equal(got.sourceAuthor, "claude");
    assert.equal(got.sourceOpacity, custom ? "0.65" : "1");
    assert.equal(got.replyAuthor, "claude");
    assert.equal(got.replyOpacity, custom ? "0.65" : "1");
    assert.equal(got.highlight, custom ? "rgb(253, 230, 138)" : got.fallbackHighlight);
    assert.equal(got.card, custom ? "rgb(255, 237, 213)" : got.fallbackCard);
    if (custom) assert(got.customCss.includes("--tine-comment-card-bg"));
    assert.deepEqual(errors, []);
    facts.push({ width, custom, ...got });
    await page.screenshot({ path: `${out}/${custom ? "custom" : "default"}-${width}.png` });
    await context.close();
  }
  writeFileSync(`${out}/styling-facts.json`, JSON.stringify(facts, null, 2) + "\n");
  console.log(`PASS custom.css tokens and authors at 1440, 900, 390; screenshots in ${out}`);
} finally {
  await browser?.close();
  process.kill(-server.pid, "SIGTERM");
}
