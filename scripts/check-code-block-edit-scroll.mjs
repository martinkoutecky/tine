// GH #489: clicking into a large fenced code block must not strand the editor's
// horizontal view far to the right, leaving the code the user clicked off-screen.
//
// The textarea that replaces a code card is `wrap="off"` with `overflow-x: auto`,
// and autosize measures its content by transiently setting `height: auto`. While
// collapsed, the engine reveals the caret inside the now-scrollable box, which
// moves scrollLeft; restoring the height removes the vertical overflow but left
// scrollLeft parked at a long line's far end. `resizeNow` already pins the
// enclosing vertical scroller across that measure; this is the horizontal twin.
//
// Usage: npm run build && node scripts/check-code-block-edit-scroll.mjs
import { waitForHttpServer } from "./e2e-capabilities.mjs";
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5198;
const LONG = "x".repeat(600);
const FENCE = [
  "```js",
  ...Array.from({ length: 400 }, (_, i) => `const short_${i} = ${i};`),
  `const wide = "${LONG}";`,
  ...Array.from({ length: 400 }, (_, i) => `const tail_${i} = ${i};`),
  `const wide_tail = "${LONG}";`,
  "```",
].join("\n");

const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });

function fail(message) {
  console.error(`FAIL ${message}`);
  process.exitCode = 1;
}

try {
  await waitForHttpServer(`http://localhost:${PORT}/`, 60, 250, { failureMessage: "server did not start" });
  const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"] });
  const page = await browser.newPage({ viewport: { width: 1120, height: 820 } });
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 8000 });

  await page.keyboard.press("Control+k");
  await page.waitForSelector(".switcher-input", { timeout: 4000 });
  await page.locator(".switcher-input").fill("Tine");
  await sleep(500);
  const target = page.locator(".switcher-row", { hasText: /^pageTine$/ }).first();
  await target.waitFor({ state: "visible", timeout: 4000 });
  await target.click();
  await page.waitForSelector(".ls-block", { timeout: 4000 });

  const fixture = page.locator(".main-content .ls-block", { hasText: "Reads the same markdown graph" }).first();
  await fixture.locator(".block-content").first().click();
  await page.waitForSelector("textarea.block-editor", { timeout: 3000 });
  await page.evaluate((text) => {
    const ta = document.querySelector("textarea.block-editor");
    Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value").set.call(ta, text);
    ta.dispatchEvent(new InputEvent("input", { bubbles: true, cancelable: true }));
  }, FENCE);
  await sleep(200);
  await page.evaluate(() => document.activeElement.blur());
  await page.waitForSelector("pre.code-block", { timeout: 5000 });
  await sleep(600);

  const result = await page.evaluate(async () => {
    const pre = [...document.querySelectorAll("pre.code-block")].find((p) => p.textContent.includes("const wide"));
    if (!pre) return { error: "rendered code card not found" };
    // The card is taller than the viewport, so align its TOP: the click has to
    // land on a line the user can actually see.
    pre.scrollIntoView({ block: "start" });
    await new Promise((r) => setTimeout(r, 250));
    // Click the first visible line of the card: the user's "click inside the
    // code block", nowhere near the long line at its end.
    const box = pre.getBoundingClientRect();
    const x = box.left + 30;
    const y = Math.max(box.top + 14, 24);
    const probeRange = document.caretRangeFromPoint(x, y);
    const codeEl = pre.querySelector("code");
    let probeOffset = null;
    if (probeRange && codeEl && codeEl.contains(probeRange.startContainer)) {
      const walker = document.createTreeWalker(codeEl, NodeFilter.SHOW_TEXT);
      let acc = 0;
      let n;
      while ((n = walker.nextNode())) {
        if (n === probeRange.startContainer) { acc += probeRange.startOffset; probeOffset = acc; break; }
        acc += (n.textContent ?? "").length;
      }
    }
    window.__probe = {
      preBox: { top: box.top, left: box.left, height: box.height },
      probeStartContainerText: probeRange ? (probeRange.startContainer.textContent ?? "").slice(0, 40) : null,
      probeStartOffset: probeRange ? probeRange.startOffset : null,
      probeOffset,
      codeTextLength: codeEl ? (codeEl.textContent ?? "").length : null,
    };
    for (const type of ["mousedown", "mouseup", "click"]) {
      pre.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y }));
    }
    let ta = null;
    for (let i = 0; i < 60 && !ta; i++) {
      ta = document.querySelector("textarea.block-editor");
      if (!ta) await new Promise((r) => setTimeout(r, 50));
    }
    if (!ta) return { error: "editor did not appear" };
    // Let autosize's rAF measure run, then look at where the view ended up.
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    await new Promise((r) => setTimeout(r, 250));
    const scroller = ta.closest("[data-pane-scroll], main.main-content") ?? document.scrollingElement;
    return {
      editorScrollLeft: ta.scrollLeft,
      editorScrollWidth: ta.scrollWidth,
      editorClientWidth: ta.clientWidth,
      selectionStart: ta.selectionStart,
      valueLength: ta.value.length,
      ancestorScrollLeft: scroller ? scroller.scrollLeft : null,
      bodyScrollLeft: document.scrollingElement ? document.scrollingElement.scrollLeft : null,
      probe: window.__probe,
    };
  });

  console.log(JSON.stringify(result, null, 2));
  if (result.error) fail(result.error);
  else {
    // Two user outcomes, not two implementation constants. The click was two
    // characters into the FIRST line of the card, so (a) the caret must land
    // there rather than 18k characters away at the end of the block, and (b)
    // the editor must be showing that column. Any implementation that reveals
    // the caret satisfies (b); the allowance is one small reveal margin, not a
    // demand for exactly 0.
    const REVEAL_MARGIN = 40;
    if (result.probe?.probeOffset != null && Math.abs(result.selectionStart - result.probe.probeOffset) > 2)
      fail(
        `click at code offset ${result.probe.probeOffset} put the caret at ${result.selectionStart} of ${result.valueLength} (GH #489)`,
      );
    if (result.editorScrollLeft > REVEAL_MARGIN)
      fail(`editor is scrolled ${result.editorScrollLeft}px right after a click on its first line; the code clicked is off-screen (GH #489)`);
    if (result.ancestorScrollLeft) fail(`enclosing scroller moved right by ${result.ancestorScrollLeft}px`);
    if (result.bodyScrollLeft) fail(`the page itself scrolled right by ${result.bodyScrollLeft}px`);
  }
  if (!process.exitCode) console.log("code-block edit scroll OK: a click inside a large code block puts the caret where it landed, on screen");
  await browser.close();
} finally {
  server.kill("SIGTERM");
}
