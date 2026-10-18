// GH #210 (Dieterbe, 0.8.0-beta.3), in the rendered page: a table block whose
// view is chosen before its title is typed is the first bullet of a page with no
// page properties. Typing its title and leaving the editor must neither toast
// "Page-header properties must contain only valid key:: value lines" nor stop
// the page saving.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { flushPage, isDirty, resetStore } from "../document";
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
import { pageToDto } from "../document/convert";

beforeAll(async () => { await initParser(); });
afterEach(() => {
  vi.clearAllTimers(); vi.useRealTimers(); vi.restoreAllMocks();
  endEdit("blur"); resetStore(); setGraphMeta(null); document.body.innerHTML = ""; resetTabsToJournals();
});
const tick = () => new Promise<void>((resolve) => queueMicrotask(resolve));
function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}
function type(editor: HTMLTextAreaElement, value: string) {
  editor.value = value;
  editor.dispatchEvent(new Event("input", { bubbles: true }));
}

it("typing the title of a table block that is a page's first bullet toasts nothing and keeps saving (GH #210)", async () => {
  const config = "tine.view:: table\ntine.table-widths:: prop%3Atest=171;title=341";
  const dto: PageDto = { name: "Table first", kind: "page", title: "Table first", pre_block: null, format: "md",
    blocks: [{ id: "t1", raw: "tine.view:: table", collapsed: false, children: [] },
      { id: "o1", raw: "other", collapsed: false, children: [] }] };
  loadSingle(structuredClone(dto));
  vi.spyOn(backend(), "getPage").mockResolvedValue(dto as PageRead);
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/Table first.md" });
  openAsLoaded(await bindTestHost());
  const save = vi.spyOn(backend(), "pageSubmit");
  mainPaneRouter.openPage(dto.name, "page", { inPlace: true });
  const { root, dispose } = mount(() => <PageView />);
  try {
    await tick(); await tick();
    // The title-less table block is an ordinary block now (not a page-properties row); clicking it opens its editor.
    expect(root.querySelector(".page-properties")).toBeNull();
    root.querySelector('[data-block-id="t1"] .block-content')!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    await tick();
    type(root.querySelector<HTMLTextAreaElement>('[data-block-id="t1"] textarea')!, config);
    endEdit("blur"); await tick(); await tick();
    expect(await flushPage(dto.name)).toBe(true);
    // The save kept it a bullet: no page properties exist on this page.
    expect(submittedPages(save)[0].pre_block ?? null).toBeNull();
    expect(submittedPages(save)[0].blocks[0].raw).toBe(config);

    // Now the title, typed above the config, and leave the editor.
    const { startEditing } = await import("../editorController");
    startEditing("t1", 0); await tick();
    const editor = root.querySelector<HTMLTextAreaElement>('[data-block-id="t1"] textarea')!;
    type(editor, `test table\n${config}`);
    setToasts([]);
    editor.dispatchEvent(new FocusEvent("blur"));
    await tick(); await tick();
    expect(toasts()).toEqual([]);
    expect(doc.byId.t1.originatedFromPageHeader).toBeUndefined();
    expect(pageToDto(dto.name)?.blocks[0].raw).toBe(`test table\n${config}`);
    expect(await flushPage(dto.name)).toBe(true);
    expect(isDirty(dto.name)).toBe(false);
    expect(submittedPages(save).at(-1)!.blocks.map((b) => b.raw)).toEqual([`test table\n${config}`, "other"]);
  } finally { dispose(); }
});

// GH #210 F4: page properties above the first bullet that hold a block-scoped key (an OG
// PDF-highlight page's `id::`, a schema page's page-level `tine.table-widths::`) must stay the
// page header after the header editor opens and closes, not turn into an outline bullet.
for (const pre of [
  "file:: [x.pdf](../assets/x.pdf)\nhls-page:: 3\nid:: 0f3966ad-17d6-4c01-9c55-675870a88c33",
  "tine.fields:: a;b\ntine.table-widths:: prop%3Aa=171",
]) {
  it(`a preamble with a block-scoped key stays the page header after its editor closes: ${pre.split("\n")[0]}`, async () => {
    const dto: PageDto = { name: "Hl", kind: "page", title: "Hl", pre_block: pre, format: "md",
      blocks: [{ id: "o1", raw: "other", collapsed: false, children: [] }] };
    loadSingle(structuredClone(dto));
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto as PageRead);
    vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "absent", id: "pages/Hl.md" });
    openAsLoaded(await bindTestHost());
    const save = vi.spyOn(backend(), "pageSubmit");
    mainPaneRouter.openPage(dto.name, "page", { inPlace: true });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick(); await tick();
      const rows = pre.split("\n").length;
      expect(root.querySelectorAll(".page-properties .prop-row")).toHaveLength(rows);
      root.querySelector(".page-properties")!.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0 }));
      await tick();
      expect(root.querySelector("textarea")).not.toBeNull();
      endEdit("blur"); await tick(); await tick();
      expect(root.querySelectorAll(".page-properties .prop-row")).toHaveLength(rows);
      expect([...root.querySelectorAll(".ls-block")].map((b) => (b as HTMLElement).dataset.blockId)).toEqual(["o1"]);
      expect(await flushPage(dto.name)).toBe(true);
      for (const written of submittedPages(save)) expect(written.pre_block).toBe(pre);
    } finally { dispose(); }
  });
}
