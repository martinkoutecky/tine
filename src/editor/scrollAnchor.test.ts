// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { captureEditorScrollAnchor } from "./scrollAnchor";
afterEach(() => { document.body.innerHTML = ""; });
function fixture() {
  const scroller = document.createElement("div"), editor = document.createElement("textarea");
  scroller.append(editor); document.body.append(scroller); editor.focus(); scroller.scrollTop = 50;
  let top = 100;
  editor.getBoundingClientRect = () => ({ top } as DOMRect);
  const anchor = captureEditorScrollAnchor(editor, scroller)!;
  return { scroller, editor, anchor, move: (next: number) => { top = next; } };
}
it.each([104, -30])("absorbs %s pixels of above-editor displacement", (delta) => {
  const f = fixture(); f.move(100 + delta); f.anchor.restore(); expect(f.scroller.scrollTop).toBe(50 + delta);
});
it.each(["wheel", "pointerdown", "touchstart"])("yields to %s even before scrollTop changes", (type) => {
  const f = fixture(); f.move(204); f.scroller.dispatchEvent(new Event(type)); f.anchor.restore(); expect(f.scroller.scrollTop).toBe(50);
});
it("does not undo another scroll owner", () => {
  const f = fixture(); f.move(204); f.scroller.scrollTop = 80; f.anchor.restore(); expect(f.scroller.scrollTop).toBe(80);
});
it.each(["blur", "disconnect", "cancel"])("drops a pending anchor on %s", (cause) => {
  const f = fixture(); f.move(204);
  if (cause === "blur") f.editor.blur(); else if (cause === "disconnect") f.editor.remove(); else f.anchor.cancel();
  f.anchor.restore(); expect(f.scroller.scrollTop).toBe(50);
});

it("restores to an explicitly supplied focused replacement in the same scroller", () => {
  const f = fixture();
  const replacement = document.createElement("textarea");
  f.editor.replaceWith(replacement); replacement.focus();
  replacement.getBoundingClientRect = () => ({ top: 70 } as DOMRect);
  f.anchor.restore(replacement);
  expect(f.scroller.scrollTop).toBe(20);
});
it("rejects a replacement in another scroller", () => {
  const f = fixture();
  const replacement = document.createElement("textarea");
  document.body.append(replacement); replacement.focus();
  replacement.getBoundingClientRect = () => ({ top: 70 } as DOMRect);
  f.anchor.restore(replacement);
  expect(f.scroller.scrollTop).toBe(50);
});
