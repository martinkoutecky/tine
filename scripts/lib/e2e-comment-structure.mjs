// GH #891 / #110, DESIGN §1. A routed synthetic page, literal native keys and
// pointer gestures, and saved outline semantics. No application test hooks.
import fs from "node:fs";
import { saveJourneyScreenshot } from "./e2e-screenshot.mjs";
import { openPageByName } from "./e2e-navigation.mjs";
import { waitForFileText } from "../e2e-file-poll.mjs";

export const STRUCTURE_PAGE = "Margin Body Structure";
export const STRUCTURE_INITIAL = `- Merge parent
  - Thread remark
    tine.quote:: Merge parent
    - Reply stays
      - Deep stays
  - Merge child
- Enter parent
  - Enter remark
    tine.quote:: Enter parent
- Drag source
- Drag target
  - Target remark
    tine.quote:: Drag target
`;

export async function checkCommentStructure(browser, pageFile, artifacts) {
  await browser.setWindowSize(1600, 1000); // visible margin cards expose thread drag/drop guards
  await openPageByName(browser, STRUCTURE_PAGE);
  const clearPaneMode = async () => {
    if (await browser.$(".pane-select-hint").isExisting()) await browser.keys(["Escape"]);
    await browser.waitUntil(() => browser.execute(() => !document.querySelector(".pane-select-hint")), { timeout: 5_000 });
  };
  const rowId = async text => {
    let id;
    await browser.waitUntil(async () => {
      id = await browser.execute(text => [...document.querySelectorAll(".block-content")]
        .find(el => el.textContent.trim() === text)?.closest(".ls-block")?.dataset.blockId ?? null, text);
      return !!id;
    }, { timeout: 10_000, timeoutMsg: `missing body or comment: ${text}` });
    return id;
  };
  const editEnd = async text => {
    await clearPaneMode();
    const id = await rowId(text);
    await browser.$(`[data-block-id="${id}"] > .block-main .block-content`).click();
    await browser.waitUntil(() => browser.execute(id => document.querySelector("textarea.block-editor")?.closest(".ls-block")?.dataset.blockId === id, id), { timeout: 5_000 });
    await browser.execute(() => {
      const ta = document.querySelector("textarea.block-editor");
      ta.focus(); ta.setSelectionRange(ta.value.length, ta.value.length);
    });
  };
  const threadBytes = "  - Thread remark\n    tine.quote:: Merge parent\n    - Reply stays\n      - Deep stays\n";
  await editEnd("Merge parent");
  await browser.keys(["Delete"]);
  await browser.keys(["Escape"]);
  await browser.keys(["Escape"]);
  await clearPaneMode();
  const merged = await waitForFileText(pageFile, text => text.includes("- Merge parentMerge child\n"), { timeoutMs: 20_000 });
  if (!merged.includes(threadBytes)) throw new Error(`Delete changed the comment thread: ${merged}`);
  await rowId("Merge parentMerge child");
  await saveJourneyScreenshot(browser, `${artifacts}/structure-delete.png`);

  await editEnd("Enter parent");
  await browser.keys(["Enter"]);
  await browser.keys("Body sibling");
  await browser.keys(["Escape"]);
  await browser.keys(["Escape"]);
  await clearPaneMode();
  const entered = await waitForFileText(pageFile, text => text.includes("\n- Body sibling\n"), { timeoutMs: 20_000 });
  if (!entered.includes("- Enter parent\n  - Enter remark\n    tine.quote:: Enter parent\n")) {
    throw new Error(`Enter changed its parent's comment: ${entered}`);
  }
  await rowId("Body sibling");
  await saveJourneyScreenshot(browser, `${artifacts}/structure-enter.png`);

  const point = async (id, selector) => browser.execute((id, selector) => {
    const el = document.querySelector(`[data-block-id="${id}"] ${selector}`);
    const r = el.getBoundingClientRect();
    return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) };
  }, id, selector);
  const move = p => ({ type: "pointerMove", duration: 150, x: p.x, y: p.y });
  const drag = async (from, to, checkDuring) => {
    await browser.performActions([{ type: "pointer", id: "comment-structure-mouse", parameters: { pointerType: "mouse" }, actions: [
      move(from), { type: "pointerDown", button: 0 }, move({ x: from.x + 12, y: from.y + 8 }), move(to),
    ] }]);
    try { await checkDuring?.(); }
    finally {
      await browser.performActions([{ type: "pointer", id: "comment-structure-mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerUp", button: 0 }] }]);
      await browser.releaseActions();
    }
  };
  const source = await rowId("Drag source"), target = await rowId("Drag target"), comment = await rowId("Target remark");
  // Start with an ordinary body editor, so pane or block selection cannot
  // intercept the comment-chip gesture or replace the requested drag source.
  await editEnd("Drag source");
  await drag(await point(comment, ".comment-handle"), await point(source, ".block-main"), async () => {
    const moving = await browser.execute(() => !!document.querySelector(".dragging"));
    if (moving) throw new Error("the comment author chip still starts a block drag");
  });
  await drag(await point(source, ".bullet-container"), await point(comment, ".block-main"), async () => {
    const target = await browser.execute(() => !!document.querySelector(".drop-before, .drop-after, .drop-child"));
    if (target) throw new Error("a comment thread is still a drop target");
  });
  await saveJourneyScreenshot(browser, `${artifacts}/structure-rejected-drag.png`);
  if (fs.readFileSync(pageFile, "utf8") !== entered) throw new Error("a rejected comment drag changed the page file");

  // Positive control: the same real pointer gesture can move an ordinary block.
  await drag(await point(source, ".bullet-container"), await point(target, ".block-content"));
  const dragged = await waitForFileText(pageFile, text => /- Drag target\n(?:[^\n]*\n)*?\s+- Drag source\n/.test(text), { timeoutMs: 20_000 });
  if (!dragged.includes("  - Target remark\n    tine.quote:: Drag target\n")) throw new Error(`body drag changed the target's comment: ${dragged}`);
  if (dragged.split("- Drag source").length !== 2) throw new Error(`body drag duplicated or lost its source: ${dragged}`);
  await browser.$(`[data-block-id="${source}"] > .block-main`).waitForDisplayed({ timeout: 5_000 });
  await browser.execute(() => new Promise(resolve => requestAnimationFrame(resolve)));
  await saveJourneyScreenshot(browser, `${artifacts}/structure-body-drag.png`);
  console.log("PASS: GH #891 body Delete and Enter preserve threads; native comment drags and thread drops refuse; an ordinary drag still persists");
}
