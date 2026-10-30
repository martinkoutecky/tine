// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";

// GH #783 (Android): tapping a line of a long block opened the keyboard and the line ended up above the top of the
// screen. jsdom has no layout, so the caret row and the rects are given; the rule under test is what the toolbar does.
vi.mock("../editor/caretRows", () => ({ textareaCaretRow: () => ({ top: 0, bottom: 20 }) }));

afterEach(() => { document.body.innerHTML = ""; });

it("a tapped line of a tall editor stays on screen when the keyboard toolbar appears (GH #783)", async () => {
  const { revealFocusedEditorAboveToolbar } = await import("./MobileKeyboardToolbar");
  const scroller = document.createElement("main");
  scroller.className = "main-content";
  const editor = document.createElement("textarea");
  editor.className = "block-editor";
  scroller.appendChild(editor);
  document.body.appendChild(scroller);
  editor.focus();
  Object.defineProperty(scroller, "clientHeight", { value: 500 });
  scroller.getBoundingClientRect = () => ({ top: 100 } as DOMRect);
  // an 800px block tapped on its first line, y 400; the toolbar sits at y 600
  editor.getBoundingClientRect = () => ({ top: 400 - scroller.scrollTop, bottom: 1200 - scroller.scrollTop, height: 800, left: 0, right: 300, width: 300 } as DOMRect);
  expect(revealFocusedEditorAboveToolbar(600)).toBe(false);
  expect(scroller.scrollTop).toBe(0); // the caret row (y 400) is above the toolbar: nothing to do
});
