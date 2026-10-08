// GH #465 (master b7cde3129, 82ea9af33, 401951955): a click in the run-out past
// the last glyph puts the caret at the end of the block, even when the span map
// answers a plausible interior offset (before an invisible closing `*`).
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { Block } from "./Block";

beforeAll(() => initParser());

type CaretDoc = Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
const originalRects = Range.prototype.getClientRects;
afterEach(() => {
  Range.prototype.getClientRects = originalRects;
  delete (document as { caretRangeFromPoint?: unknown }).caretRangeFromPoint;
  resetStore();
  document.body.innerHTML = "";
});

describe("click past the last glyph (GH #465)", () => {
  it("places the caret at the end of a block ending in italic text", async () => {
    const raw = "*some text in italics.*";
    loadSingle({ name: "Past end", kind: "page", title: "Past end", pre_block: null, blocks: [
      { id: "italic", raw, collapsed: false, children: [] },
    ] });
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <For each={pageByName("Past end")?.roots ?? []}>{(id) => <Block id={id} />}</For>, root);
    try {
      const content = root.querySelector<HTMLElement>('[data-block-id="italic"] .block-content')!;
      const text = content.querySelector("em")!.firstChild as Text;
      // The span map's answer for this click: the end of the italic text, one
      // byte before the closing delimiter.
      (document as CaretDoc).caretRangeFromPoint = () => {
        const range = document.createRange();
        range.setStart(text, text.length);
        return range;
      };
      // One rendered line from x=8 to x=164.6; the click lands at x=300.
      Range.prototype.getClientRects = function () {
        return [new DOMRect(8, 13, 156.6, 19)] as unknown as DOMRectList;
      };
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: 300, clientY: 22 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0, clientX: 300, clientY: 22 }));
      await vi.waitFor(() => expect(root.querySelector("textarea.block-editor")).not.toBeNull());
      const editor = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      await vi.waitFor(() => expect(editor.selectionStart).toBe(raw.length));
    } finally {
      dispose();
    }
  });
});
