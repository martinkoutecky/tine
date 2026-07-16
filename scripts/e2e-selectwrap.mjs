// Verify #18 selection-wrap in the REAL frontend (Chromium + mock backend via
// vite preview): entering the editor, selecting text, and typing a wrap key must
// wrap the selection — `[[sel]]`/`((sel))` open the page/block search; emphasis
// marks (`*`/`~`/`=`) wrap and double, including literal delimiter events with
// Alt held (GH #83). Drives real keydown events, so it exercises Block.tsx's
// keydown→wrapSelectionEdit→autocomplete path, not just the pure logic.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = Number(process.env.E2E_PREVIEW_PORT || 5197);
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

let failures = 0;
const check = (name, got, want) => {
  const ok = got === want;
  if (!ok) failures++;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}: ${JSON.stringify(got)}${ok ? "" : ` (want ${JSON.stringify(want)})`}`);
};

// Enter the first block's editor, replace its text with `content`, then select
// [selStart,selEnd) and press each key in `keys`. Returns {value, acOpen}.
async function gesture(page, content, selStart, selEnd, keys) {
  await page.locator(".ls-block .block-content").first().click();
  await page.waitForSelector("textarea.block-editor", { timeout: 3000 });
  await page.keyboard.press("Control+a");
  await page.keyboard.type(content);
  await sleep(120);
  await page.evaluate(([s, e]) => {
    const ta = document.querySelector("textarea.block-editor");
    ta.focus();
    ta.setSelectionRange(s, e);
  }, [selStart, selEnd]);
  for (const k of keys) { await page.keyboard.press(k); await sleep(90); }
  await sleep(150);
  const value = await page.evaluate(() => document.querySelector("textarea.block-editor")?.value ?? null);
  const acOpen = await page.evaluate(() => !!document.querySelector(".autocomplete"));
  return { value, acOpen };
}

// Exercise the configured semantic command path rather than literal delimiter
// auto-wrapping. `direction=backward` matches Ctrl+Shift+Left's live selection.
async function formatGesture(page, content, selStart, selEnd, key, direction = "backward") {
  await page.locator(".ls-block .block-content").first().click();
  await page.waitForSelector("textarea.block-editor", { timeout: 3000 });
  await page.keyboard.press("Control+a");
  await page.keyboard.type(content);
  await sleep(120);
  await page.evaluate(([s, e, dir]) => {
    const ta = document.querySelector("textarea.block-editor");
    ta.focus();
    ta.setSelectionRange(s, e, dir);
  }, [selStart, selEnd, direction]);
  await page.keyboard.press(key);
  await sleep(150);
  return page.evaluate(() => {
    const ta = document.querySelector("textarea.block-editor");
    return {
      value: ta?.value ?? null,
      selection: ta ? [ta.selectionStart, ta.selectionEnd, ta.selectionDirection] : null,
    };
  });
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1000, height: 800 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });

  // 1) select "foo" in "hello foo world", type [ [ → page ref + search open
  {
    const { value, acOpen } = await gesture(page, "hello foo world", 6, 9, ["[", "["]);
    check("[[ wraps selection", value, "hello [[foo]] world");
    check("[[ opens page search", acOpen, true);
    // "hit [[ <enter>": accepting the completion must not leave a dangling `]]`.
    await page.keyboard.press("Enter");
    await sleep(150);
    const after = await page.evaluate(() => document.querySelector("textarea.block-editor")?.value ?? null);
    check("Enter accepts a clean page ref (no dangling ]])", /^hello \[\[[^\]]+\]\] world$/.test(after ?? ""), true);
  }
  // 2) select "bar" in "bar baz", type ( ( → block ref + search open
  {
    const { value, acOpen } = await gesture(page, "bar baz", 0, 3, ["(", "("]);
    check("(( wraps selection", value, "((bar)) baz");
    check("(( opens block search", acOpen, true);
    await page.keyboard.press("Escape");
  }
  // 3) select "bold" in "make bold now", type * * → **bold**, no search
  {
    const { value, acOpen } = await gesture(page, "make bold now", 5, 9, ["*", "*"]);
    check("** wraps selection (bold)", value, "make **bold** now");
    check("emphasis opens no search", acOpen, false);
    await page.keyboard.press("Escape");
  }
  // 4) select "hi" in "hi there", single ~ then = to sanity-check other marks
  {
    const { value } = await gesture(page, "hi there", 0, 2, ["~"]);
    check("~ wraps selection", value, "~hi~ there");
    await page.keyboard.press("Escape");
  }
  // 5) On layouts where Alt+[ still reports the literal `[`, match OG's
  // incidental two-press wrapping behavior without mapping the physical key.
  {
    const { value, acOpen } = await gesture(page, "alt selected text", 4, 12, ["Alt+[", "Alt+["]);
    check("Alt+[[ wraps a literal-key selection", value, "alt [[selected]] text");
    check("Alt+[[ opens page search", acOpen, true);
    await page.keyboard.press("Escape");
  }
  // 6) GH #178: Windows' Ctrl+Shift+Left commonly includes the trailing space.
  // The semantic bold command must keep it outside the Markdown delimiter,
  // preserve the backward inner selection, commit, and render as strong text.
  {
    const result = await formatGesture(page, "before selected after", 7, 16, "Control+b");
    check("format command excludes browser-selected trailing space", result.value, "before **selected** after");
    check("format command preserves backward inner selection", JSON.stringify(result.selection), JSON.stringify([9, 17, "backward"]));
    await page.keyboard.press("Escape");
    await sleep(200);
    const rendered = await page.locator(".ls-block .block-content strong").first().textContent().catch(() => null);
    check("saved source renders with the intended strong node", rendered, "selected");
  }

  await page.screenshot({ path: "screenshots/selectwrap.png" });
  console.log(errors.length ? "PAGE ERRORS:\n" + errors.join("\n") : "no page errors");
  console.log(failures ? `\n${failures} CHECK(S) FAILED` : "\nALL CHECKS PASSED");
  await browser.close();
  server.kill("SIGKILL");
  process.exit(failures ? 1 : 0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
