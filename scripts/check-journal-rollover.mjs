import { chromium } from "playwright";
import assert from "node:assert/strict";

const browser = await chromium.launch({ args: ["--no-sandbox", "--disable-dev-shm-usage"] });
try {
  const page = await browser.newPage();
  page.on("pageerror", error => console.error(error));
  await page.clock.setFixedTime(new Date(2030, 6, 15, 12));
  await page.goto(`${process.env.TINE_PROBE_URL ?? "http://127.0.0.1:5499"}/scripts/fixtures/journal-rollover/rollover.html`);
  await page.waitForFunction(() => window.rollover?.doc.feed.length > 0);
  await page.evaluate(() => window.rollover.startEditing("overnight-block", 3));
  await page.locator("textarea.block-editor").waitFor();
  await page.waitForFunction(() => document.activeElement?.tagName === "TEXTAREA");
  if (process.env.ROLLOVER_DIRTY === "1") {
    await page.locator("textarea.block-editor").fill("unsaved overnight notes");
  }
  await page.evaluate(() => {
    window.originalEditor = document.activeElement;
    window.originalEditor.setSelectionRange(2, 8, "backward");
    window.originalText = window.originalEditor.value;
    window.blurCount = 0;
    window.originalEditor.addEventListener("blur", () => window.blurCount++);
  });
  await page.clock.setFixedTime(new Date(2030, 6, 16, 12));
  await page.evaluate((visible) => visible
    ? document.dispatchEvent(new Event("visibilitychange"))
    : window.dispatchEvent(new Event("focus")), process.env.ROLLOVER_VISIBLE === "1");
  await page.waitForTimeout(1000);
  await page.waitForTimeout(100);
  const result = await page.evaluate(() => ({
    feed: [...window.rollover.doc.feed],
    editingId: window.rollover.editingId(),
    sameEditor: document.querySelector("textarea.block-editor") === window.originalEditor,
    focused: document.activeElement === window.originalEditor,
    textPreserved: window.originalEditor.value === window.originalText,
    selection: [window.originalEditor.selectionStart, window.originalEditor.selectionEnd, window.originalEditor.selectionDirection],
    blurCount: window.blurCount,
  }));
  console.log(JSON.stringify(result, null, 2));
  
  assert.equal(result.feed[0], "Jul 16th, 2030");
  assert.equal(result.editingId, "overnight-block");
  assert.equal(result.sameEditor, true);
  assert.equal(result.focused, true);
  assert.equal(result.textPreserved, true);
  assert.deepEqual(result.selection, [2, 8, "backward"]);
  assert.equal(result.blurCount, 0);
  const readsBefore = await page.evaluate(() => window.rollover.reads());
  await page.evaluate(() => {
    window.dispatchEvent(new Event("focus"));
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForTimeout(100);
  assert.equal(await page.evaluate(() => window.rollover.reads()), readsBefore);
  await page.keyboard.type("Z");
  assert.equal(await page.locator("textarea.block-editor").inputValue(), await page.evaluate(() =>
    window.originalText.slice(0, 2) + "Z" + window.originalText.slice(8)));
  console.log("PASS rollover preserves active editor and selection; duplicate returns do not fetch");
} finally {
  await browser.close();
}
