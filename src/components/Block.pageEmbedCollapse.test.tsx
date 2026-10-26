import { afterAll, afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { blockProperty, resetStore, setCollapsed } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { Block } from "./Block";
import { endEdit, startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";

let disposeKeys: (() => void) | undefined;
beforeAll(async () => { await initParser(); disposeKeys = installKeybindings(); });
afterAll(() => disposeKeys?.());
afterEach(() => { endEdit("page-navigation"); vi.restoreAllMocks(); resetStore(); document.body.innerHTML = ""; });
const tick = () => new Promise(resolve => setTimeout(resolve, 50));

for (const format of ["md", "org"] as const) for (const children of [false, true]) {
  it(`${format}: page embed shares the host fold with children=${children}, persists and leaves source untouched`, async () => {
    const source = { id: `pages/Source.${format}`, name: "Source", title: "Source", kind: "page" as const, format, pre_block: null,
      blocks: [{ id: "source", raw: "source body", collapsed: false, children: [] }] };
    vi.spyOn(backend(), "getPage").mockResolvedValue(source);
    loadSingle({ name: "Host", title: "Host", kind: "page", format, pre_block: null,
      blocks: [{ id: "host", raw: "{{embed [[Source]]}}", collapsed: false,
        children: children ? [{ id: "child", raw: "host child", collapsed: false, children: [] }] : [] }] });
    const root = document.createElement("div"); document.body.append(root);
    let dispose = render(() => <Block id="host" />, root);
    try {
      await tick();
      expect(root.textContent).toContain("source body");
      const toggle = root.querySelector<HTMLElement>(".collapse-toggle.has-children");
      expect(toggle).not.toBeNull();
      toggle!.click(); await tick();
      expect(root.querySelector(".embed-page-header")?.textContent).toContain("Source");
      expect(root.textContent).not.toContain("source body");
      expect(root.textContent).not.toContain("host child");
      expect(blockProperty("host", "collapsed")).toBe("true");
      expect(doc.byId.source.raw).toBe("source body");
      expect(doc.byId.source.collapsed).toBe(false);
      dispose(); root.innerHTML = "";
      dispose = render(() => <Block id="host" />, root); await tick();
      expect(root.textContent).not.toContain("source body");
      // Existing editor/selection keyboard commands enter this same boundary.
      setCollapsed("host", false); await tick();
      expect(root.textContent).toContain("source body");
      expect(blockProperty("host", "collapsed")).toBe("false");
      if (children) expect(root.textContent).toContain("host child");
      startEditing("host", 0); await tick();
      const editor = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", ctrlKey: true, bubbles: true, cancelable: true }));
      expect(doc.byId.host.collapsed).toBe(true);
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", ctrlKey: true, bubbles: true, cancelable: true }));
      expect(doc.byId.host.collapsed).toBe(false);
    } finally { dispose(); }
  });
}
