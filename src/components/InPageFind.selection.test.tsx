import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { closeInPageFind, inPageFindActiveIndex, openInPageFind, setInPageFindQuery } from "../inpageFind";
import { InPageFind } from "./InPageFind";
import { resetPaneLayoutToSingle } from "../panes";

beforeAll(async () => {
  await initParser();
  // jsdom has no geometry; matching and active-index assertions remain real.
  Range.prototype.getClientRects = vi.fn(() => [] as unknown as DOMRectList);
});
afterEach(() => { closeInPageFind({ restoreFocus: false }); window.getSelection()?.removeAllRanges(); document.body.innerHTML = ""; });

for (const mode of ["editor", "rendered"] as const) for (const text of ["needle", "needle\n", "needle\nsecond", ""]) {
  it(`${mode}: seeds only single-line selection ${JSON.stringify(text)}`, async () => {
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "query", id: "fixture", sourceKind: "search", source: "needle", presentation: "search" }], pos: 0, pinned: false }], activeIndex: 0 });
    document.body.innerHTML = '<main data-pane-id="main"><div data-inpage-find-surface="fixture">needle needle</div></main>';
    setInPageFindQuery("previous");
    if (mode === "editor") {
      const editor = document.createElement("textarea"); editor.className = "block-editor";
      editor.value = text; document.body.append(editor); editor.focus(); editor.setSelectionRange(0, text.length);
    } else {
      const span = document.createElement("span"); span.textContent = text; document.body.append(span);
      const range = document.createRange(); range.selectNodeContents(span);
      window.getSelection()?.addRange(range);
    }
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <InPageFind />, root);
    try {
      openInPageFind(); await Promise.resolve();
      const seeded = text === "needle";
      expect(root.querySelector<HTMLInputElement>("input")?.value).toBe(seeded ? "needle" : "previous");
      if (seeded) {
        expect(inPageFindActiveIndex()).toBe(0);
        expect(root.querySelector(".inpage-find-count")?.textContent).toBe("1 / 2");
      }
    } finally { dispose(); }
  });
}
