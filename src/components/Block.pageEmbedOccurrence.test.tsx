import { afterAll, afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { blockProperty, resetStore, setCollapsed } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { Block } from "./Block";
import { endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";

let disposeKeys: () => void;
beforeAll(async () => { await initParser(); disposeKeys = installKeybindings(); });
afterAll(() => disposeKeys());
afterEach(() => { endEdit("page-navigation"); resetStore(); vi.restoreAllMocks(); document.body.innerHTML = ""; });
const tick = async () => { for (let i = 0; i < 4; i++) await new Promise(r => setTimeout(r, 20)); };
const source = { id: "pages/Source", name: "Source", title: "Source", kind: "page" as const, pre_block: null,
  blocks: [{ id: "source", raw: "deep body", collapsed: false, children: [] }] };
function mount() {
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Block id="host" />, root);
  return { root, dispose };
}
function key(editor: HTMLTextAreaElement, direction: "Up" | "Down") {
  editor.dispatchEvent(new KeyboardEvent("keydown", { key: `Arrow${direction}`, ctrlKey: true, bubbles: true, cancelable: true }));
}
for (const format of ["md", "org"] as const) {
  it(`${format}: block embed root page body follows explicit host overrides with mouse and keyboard`, async () => {
    const target = { id: "target", raw: "{{embed [[Source]]}}", collapsed: false, children: [] };
    vi.spyOn(backend(), "getPage").mockResolvedValue({ ...source, format });
    vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => ({ page: "Host", kind: "page", blocks: [target] })));
    loadSingle({ name: "Host", title: "Host", kind: "page", format, pre_block: null,
      blocks: [target, { id: "host", raw: "{{embed ((target))}}", collapsed: false, children: [] }] });
    const { root, dispose } = mount();
    try {
      await tick(); expect(root.textContent).toContain("deep body");
      const row = root.querySelector<HTMLElement>('[data-block-id="target"]')!;
      row.querySelector<HTMLButtonElement>(".collapse-toggle")!.click(); await tick();
      expect(blockProperty("host", "collapsed")).toBe("true");
      expect(root.textContent).not.toContain("deep body");
      row.querySelector<HTMLElement>(".block-content")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 })); await tick();
      const editor = row.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      key(editor, "Down"); await tick();
      expect(blockProperty("host", "collapsed")).toBe("false");
      key(editor, "Up");
      expect(blockProperty("host", "collapsed")).toBe("true");
      key(editor, "Down");
      endEdit("page-navigation"); await tick(); expect(root.textContent).toContain("deep body");
      expect(blockProperty("target", "collapsed")).toBeNull();
      setCollapsed("target", true); await tick();
      expect(root.textContent).toContain("deep body"); // explicit false wins over source true
    } finally { dispose(); }
  });

  for (const container of ["page", "block"] as const) for (const action of ["mouse", "keyboard"] as const)
    it(`${format}: page embed nested in ${container} embed ${action} folds locally without changing source`, async () => {
    const nested = { id: "nested", raw: "{{embed [[Deep]]}}", collapsed: false, children: [] };
    const target = { id: "target", raw: "outer block", collapsed: false, children: [nested] };
    vi.spyOn(backend(), "getPage").mockImplementation(async name => name === "Source"
      ? { ...source, format, blocks: [nested] }
      : { ...source, name: "Deep", title: "Deep", format });
    vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => ({ page: "Host", kind: "page", blocks: [target] })));
    loadSingle({ name: "Host", title: "Host", kind: "page", format, pre_block: null,
      blocks: [...(container === "block" ? [target] : []),
        { id: "host", raw: container === "page" ? "{{embed [[Source]]}}" : "{{embed ((target))}}", collapsed: false, children: [] }] });
    const { root, dispose } = mount();
    try {
      await tick(); const row = root.querySelector<HTMLElement>('[data-block-id="nested"]')!;
      expect(row.textContent).toContain("deep body");
      if (action === "mouse") row.querySelector<HTMLButtonElement>(".collapse-toggle")!.click();
      else {
        row.querySelector<HTMLElement>(".block-content")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
        document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 })); await tick();
        key(row.querySelector<HTMLTextAreaElement>("textarea.block-editor")!, "Up");
      }
      await tick();
      expect(blockProperty("nested", "collapsed")).toBeNull();
      expect(doc.byId.nested.collapsed).toBe(false);
      expect(row.textContent).not.toContain("deep body");
      if (action === "keyboard") {
        key(row.querySelector<HTMLTextAreaElement>("textarea.block-editor")!, "Down");
        expect(blockProperty("nested", "collapsed")).toBeNull();
        endEdit("page-navigation");
      } else row.querySelector<HTMLButtonElement>(".collapse-toggle")!.click();
      await tick(); expect(row.textContent).toContain("deep body");
    } finally { dispose(); }
  });

  for (const raw of ["See {{embed [[Source]]}}", "Title\n{{embed [[Source]]}}"])
    it(`${format}: ordinary child folding preserves embed body in ${JSON.stringify(raw)}`, async () => {
      vi.spyOn(backend(), "getPage").mockResolvedValue({ ...source, format });
      loadSingle({ name: "Host", title: "Host", kind: "page", format, pre_block: null,
        blocks: [{ id: "host", raw, collapsed: false, children: [{ id: "child", raw: "child body", collapsed: false, children: [] }] }] });
      const { root, dispose } = mount();
      try {
        await tick(); expect(root.textContent).toContain("deep body");
        setCollapsed("host", true); await tick();
        expect(root.textContent).not.toContain("child body");
        expect(root.textContent).toContain("deep body");
      } finally { dispose(); }
    });

  it(`${format}: unresolved folded embed retains its identifying label`, async () => {
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    loadSingle({ name: "Host", title: "Host", kind: "page", format, pre_block: null,
      blocks: [{ id: "host", raw: "{{embed [[Absent]]}}", collapsed: false, children: [] }] });
    const { root, dispose } = mount();
    try {
      await tick(); root.querySelector<HTMLButtonElement>(".collapse-toggle")!.click(); await tick();
      expect(root.textContent).toContain("Absent");
    } finally { dispose(); }
  });
}
