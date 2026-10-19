// GH #210 review r1 F1 (UI-GH210-HEAD-ENTER), in the rendered page: click the
// page properties, put the caret at the start, press Enter. OG's
// page-properties editor takes that as a newline in the field
// (`dwim-in-properties`) and its save drops it (`string/triml`). Before the
// fix the header draft then started with a blank line, pageToDto refused it
// and the page never saved again. Both Enter's keydown and a newline the
// mobile/IME keyboard inserts itself (keyCode 229: the textarea's own input).
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { flushPage, isDirty, resetStore, setRaw } from "../document";
import { doc } from "../document/model";
import { loadSingle } from "../document/workingSet";
import { endEdit } from "../editorController";
import type { PageDto, PageRead } from "../types";
import { PageView } from "./Page";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { bindTestHost, submittedPages } from "../document/host/wiring.test.support";
import { openAsLoaded } from "./hostConflict.test.support";
import { setToasts, toasts } from "../toasts";
import { setGraphMeta } from "../graphSession";

beforeAll(async () => { await initParser(); });
afterEach(() => {
  vi.clearAllTimers(); vi.useRealTimers(); vi.restoreAllMocks();
  endEdit("blur"); resetStore(); setGraphMeta(null); document.body.innerHTML = ""; resetTabsToJournals(); setToasts([]);
});
const tick = () => new Promise<void>((resolve) => queueMicrotask(resolve));
const HEADER = "tags:: reading\nicon:: 📚";

for (const kind of ["page", "journal"] as const) {
  for (const entry of ["Enter key", "keyboard-inserted newline"] as const) {
    it(`${kind}: ${entry} at the start of the page properties keeps them the header and the page saving`, async () => {
      const name = kind === "journal" ? "2026_10_19" : "Reading list";
      const dto: PageDto = { name, kind, title: kind === "journal" ? "Oct 19th, 2026" : name, pre_block: HEADER, format: "md",
        blocks: [{ id: "o1", raw: "other", collapsed: false, children: [] }] };
      loadSingle(structuredClone(dto));
      vi.spyOn(backend(), "getPage").mockResolvedValue(dto as PageRead);
      vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: `pages/${name}.md` });
      openAsLoaded(await bindTestHost());
      const save = vi.spyOn(backend(), "pageSubmit");
      mainPaneRouter.openPage(name, kind, { inPlace: true });
      const root = document.createElement("div");
      document.body.appendChild(root);
      const dispose = render(() => <PageView />, root);
      try {
        await tick(); await tick();
        expect(root.querySelectorAll(".page-properties .prop-row")).toHaveLength(2);
        root.querySelector(".page-properties")!.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0 }));
        await tick();
        const editor = root.querySelector<HTMLTextAreaElement>("textarea")!;
        expect(editor.value).toBe(HEADER);
        const header = doc.pages.find((p) => p.name === name)!.roots[0];
        editor.setSelectionRange(0, 0);
        if (entry === "Enter key") {
          editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
        } else {
          editor.value = `\n${HEADER}`;
          editor.setSelectionRange(1, 1);
          editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertLineBreak" }));
        }
        await tick(); await tick();
        const roots = doc.pages.find((p) => p.name === name)!.roots;
        expect(roots).toEqual([header, "o1"]); // no block above the header
        expect(doc.byId[header].raw).toBe(`\n${HEADER}`); // a newline in the field, as OG
        editor.dispatchEvent(new FocusEvent("blur"));
        endEdit("blur");
        await tick(); await tick();
        expect(toasts()).toEqual([]);
        expect(root.querySelectorAll(".page-properties .prop-row")).toHaveLength(2);
        expect(await flushPage(name)).toBe(true);
        expect(isDirty(name)).toBe(false);
        for (const written of submittedPages(save)) {
          expect(written.pre_block).toBe(HEADER);
          expect(written.blocks.map((b) => b.raw)).toEqual(["other"]);
        }
        // A later edit on the page still saves.
        setRaw("o1", "other, edited");
        expect(await flushPage(name)).toBe(true);
        expect(isDirty(name)).toBe(false);
        expect(submittedPages(save).at(-1)!.pre_block).toBe(HEADER);
        expect(submittedPages(save).at(-1)!.blocks.map((b) => b.raw)).toEqual(["other, edited"]);
      } finally { dispose(); }
    });
  }
}
