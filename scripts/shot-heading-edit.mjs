// Visual verification for Item 1(a): the block editor <textarea> renders
// heading-sized while editing a SINGLE-LINE heading block (OG parity), matching
// the rendered view, and stays BODY-sized for a multi-line heading (uniline gate).
// Real frontend (Chromium + mock backend via vite preview), modeled on
// e2e-selectwrap.mjs. Writes PNGs to screenshots/ (gitignored) to eyeball.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 5198;
const server = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { stdio: "ignore" });
async function waitForServer(url, tries = 60) {
  for (let i = 0; i < tries; i++) {
    try { if ((await fetch(url)).ok) return; } catch {}
    await sleep(250);
  }
  throw new Error("server did not start");
}

// Enter the first block's editor and replace its content with `content` (typed).
async function editFirstBlock(page, content) {
  await page.locator(".ls-block .block-content").first().click();
  await page.waitForSelector("textarea.block-editor", { timeout: 3000 });
  await page.keyboard.press("Control+a");
  await page.keyboard.type(content);
  await sleep(150);
}
// Set the editing textarea's value directly (for multi-line, which typing can't do
// without splitting the block) and fire input so the store commits.
async function setEditorValue(page, value) {
  await page.locator(".ls-block .block-content").first().click();
  await page.waitForSelector("textarea.block-editor", { timeout: 3000 });
  await page.evaluate((v) => {
    const ta = document.querySelector("textarea.block-editor");
    ta.focus();
    ta.value = v;
    ta.dispatchEvent(new Event("input", { bubbles: true }));
  }, value);
  await sleep(150);
}
async function shotBlock(page, path) {
  const el = page.locator(".ls-block").first();
  await el.screenshot({ path });
}
async function editorFontPx(page) {
  return page.evaluate(() => {
    const ta = document.querySelector("textarea.block-editor");
    return ta ? parseFloat(getComputedStyle(ta).fontSize) : null;
  });
}
async function renderedHeadingFontPx(page) {
  return page.evaluate(() => {
    const h = document.querySelector(".ls-block .heading-text");
    return h ? parseFloat(getComputedStyle(h).fontSize) : null;
  });
}

try {
  await waitForServer(`http://localhost:${PORT}/`);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 900, height: 700 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector(".ls-block", { timeout: 5000 });

  const report = {};

  // h1 editing — textarea should be h1-sized.
  await editFirstBlock(page, "# Big Heading One");
  report.h1_editing_px = await editorFontPx(page);
  await shotBlock(page, "screenshots/heading-edit-h1.png");
  // Blur to rendered, capture rendered h1 size to compare (should match).
  await page.keyboard.press("Escape");
  await sleep(150);
  report.h1_rendered_px = await renderedHeadingFontPx(page);
  await shotBlock(page, "screenshots/heading-rendered-h1.png");

  // h3 editing.
  await editFirstBlock(page, "### Heading Three");
  report.h3_editing_px = await editorFontPx(page);
  await shotBlock(page, "screenshots/heading-edit-h3.png");
  await page.keyboard.press("Escape");
  await sleep(120);

  // Plain (non-heading) editing — baseline body size.
  await editFirstBlock(page, "just a normal block");
  report.body_editing_px = await editorFontPx(page);
  await page.keyboard.press("Escape");
  await sleep(120);

  // Multi-line heading — UNILINE GATE: editor should be BODY size (not heading).
  await setEditorValue(page, "# Heading with a second line\ncontinuation text here");
  report.multiline_heading_editing_px = await editorFontPx(page);
  await shotBlock(page, "screenshots/heading-edit-multiline.png");
  await page.keyboard.press("Escape");
  await sleep(120);

  console.log(JSON.stringify(report, null, 2));
  // Assertions: h1 editing == h1 rendered (no focus jump); h1 > h3 > body;
  // multiline heading == body (uniline gate off).
  const near = (a, b) => a != null && b != null && Math.abs(a - b) < 0.6;
  let fail = 0;
  const check = (name, ok) => { if (!ok) fail++; console.log(`${ok ? "PASS" : "FAIL"}  ${name}`); };
  check("h1 editing matches h1 rendered (no focus jump)", near(report.h1_editing_px, report.h1_rendered_px));
  check("h1 editing larger than body", report.h1_editing_px > report.body_editing_px + 3);
  check("h3 editing between h1 and body", report.h3_editing_px > report.body_editing_px && report.h3_editing_px < report.h1_editing_px);
  check("multiline heading edits at body size (uniline gate)", near(report.multiline_heading_editing_px, report.body_editing_px));

  console.log(errors.length ? "PAGE ERRORS:\n" + errors.join("\n") : "no page errors");
  console.log(fail ? `\n${fail} CHECK(S) FAILED` : "\nALL CHECKS PASSED");
  await browser.close();
  server.kill("SIGKILL");
  process.exit(fail ? 1 : 0);
} catch (e) {
  console.error(String(e));
  server.kill("SIGKILL");
  process.exit(1);
}
