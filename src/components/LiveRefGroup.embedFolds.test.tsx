import { For } from "solid-js";
import { render } from "solid-js/web";
import { afterAll, afterEach, beforeAll, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { blockProperty, isDirty, node, pageRoots, resetStore, undo } from "../document";
import { pageToDto } from "../document/convert";
import { loadFeed } from "../document/workingSet";
import { editingId, endEdit } from "../editorController";
import { bumpGraphEpoch } from "../graphSession";
import { installKeybindings } from "../keybindings";
import { initParser } from "../render/parse";
import { editorIn } from "../tests/editorDriver";
import type { BlockDto, PageRead } from "../types";
import { Block, SurfaceContext } from "./Block";

let disposeKeys: () => void;
beforeAll(async () => { await initParser(); disposeKeys = installKeybindings(); });
afterAll(() => disposeKeys());
afterEach(() => {
  endEdit("page-navigation"); resetStore(); bumpGraphEpoch(); vi.restoreAllMocks(); document.body.replaceChildren();
});
const leaf = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, children, collapsed: false });
const page = (name: string, blocks: BlockDto[], format: "md" | "org"): PageRead => ({
  id: `pages/${name}.${format}`, name, title: name, kind: "page", format, pre_block: null, blocks,
});

async function mount(surface: string, kind: "page" | "block", format: "md" | "org", readOnly = false) {
  const tree = leaf("guide", "Shared outline", [
    leaf("branch", "Branch", [leaf("existing", "Existing child")]),
    leaf("second", "Second branch", [leaf("second-child", "Second child")]),
  ]);
  const source = { ...page("Source", [tree], format), read_only: readOnly };
  const host = page("Host", [leaf("host", kind === "page" ? "{{embed [[Source]]}}" : "{{embed ((guide))}}")], format);
  loadFeed([host, source]);
  vi.spyOn(backend(), "getPage").mockImplementation(async () => ({ ...pageToDto("Source")!, id: source.id, read_only: readOnly }));
  vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{ page: "Source", kind: "page", path: source.id,
    blocks: [{ ...tree, children: [] }] }]);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <>
    <div class="host"><SurfaceContext.Provider value={surface}><Block id="host" /></SurfaceContext.Provider></div>
    <div class="source"><SurfaceContext.Provider value="sidebar:source"><For each={pageRoots("Source")}>
      {id => <Block id={id} />}
    </For></SurfaceContext.Provider></div>
  </>, root);
  await vi.waitFor(() => expect(root.querySelector('.embed-block [data-block-id="existing"]')).not.toBeNull());
  const embed = root.querySelector<HTMLElement>(".embed-block")!;
  const mirror = root.querySelector<HTMLElement>(".source")!;
  const row = (scope: Element, id: string) => scope.querySelector<HTMLElement>(`[data-block-id="${id}"]`)!;
  const guide = () => row(embed, "guide").querySelector<HTMLButtonElement>(":scope > .block-children-container > .block-children-left-border")!;
  const toggle = (scope: Element, id: string) => row(scope, id).querySelector<HTMLButtonElement>(":scope > .block-main .collapse-toggle")!.click();
  const edit = async (id: string, scope = embed) => {
    row(scope, id).querySelector<HTMLElement>(":scope > .block-main .block-content")!
      .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    await vi.waitFor(() => expect(editingId()).toBe(id));
    return editorIn(scope, id);
  };
  return { root, embed, mirror, dispose, row, guide, toggle, edit };
}

for (const surface of ["main", "sidebar:host", "pane:split"]) for (const kind of ["page", "block"] as const)
  for (const format of ["md", "org"] as const) {
    it(`${surface}/${kind}/${format}: guide and arrow folds save the source, synchronize both views and undo together`, async () => {
      const { embed, mirror, dispose, row, guide, toggle } = await mount(surface, kind, format);
      try {
        guide().click();
        for (const id of ["branch", "second"]) {
          expect(blockProperty(id, "collapsed")).toBe("true");
          expect(node(id).collapsed).toBe(true);
          expect(row(embed, id).classList.contains("collapsed")).toBe(true);
          expect(row(mirror, id).classList.contains("collapsed")).toBe(true);
        }
        expect(node("guide").collapsed).toBe(false); // the guide's parent stays open
        expect(isDirty("Source")).toBe(true);
        expect(isDirty("Host")).toBe(false);
        undo();
        expect(embed.textContent).toContain("Existing child");
        expect(mirror.textContent).toContain("Second child");
        expect(node("branch").collapsed).toBe(false);
        expect(node("second").collapsed).toBe(false);
        toggle(embed, "branch");
        expect(blockProperty("branch", "collapsed")).toBe("true");
        expect(mirror.textContent).not.toContain("Existing child");
        toggle(mirror, "branch");
        expect(embed.textContent).toContain("Existing child");
        expect(blockProperty("host", "collapsed")).toBeNull();
      } finally { dispose(); }
    });

    it(`${surface}/${kind}/${format}: Enter after a guide-folded branch creates a visible sibling; indent/outdent retain it`, async () => {
      const { embed, dispose, guide, edit } = await mount(surface, kind, format);
      try {
        guide().click();
        const editor = await edit("branch");
        editor.select(editor.value().length);
        editor.keydown({ key: "Enter" });
        const id = editingId()!;
        expect(id).not.toBe("branch");
        expect(node(id).parent).toBe("guide");
        const added = editorIn(embed, id);
        added.type("New visible sibling");
        added.keydown({ key: "Tab" });
        await vi.waitFor(() => expect(node(id).parent).toBe("branch"));
        expect(node("branch").collapsed).toBe(false);
        expect(editorIn(embed, id).value()).toBe("New visible sibling");
        editorIn(embed, id).keydown({ key: "Tab", shiftKey: true });
        await vi.waitFor(() => expect(node(id).parent).toBe("guide"));
        expect(editorIn(embed, id).value()).toBe("New visible sibling");
        endEdit("page-navigation");
        expect(embed.textContent).toContain("New visible sibling");
      } finally { dispose(); }
    });
  }

it.each([false, true])("the source page's Enter rule uses a visible child or a folded branch's sibling (folded=%s)", async folded => {
  const { mirror, dispose, toggle, edit } = await mount("main", "page", "md");
  try {
    if (folded) toggle(mirror, "branch");
    const editor = await edit("branch", mirror);
    editor.select(editor.value().length);
    editor.keydown({ key: "Enter" });
    const id = editingId()!;
    expect(node(id).parent).toBe(folded ? "guide" : "branch");
    expect(editorIn(mirror, id)).toBeDefined();
  } finally { dispose(); }
});

it.each(["page", "block"] as const)("a %s embed pastes visible blocks after a guide-folded branch", async kind => {
  const { embed, dispose, guide, edit } = await mount("main", kind, "md");
  try {
    guide().click();
    const editor = await edit("branch");
    editor.select(editor.value().length);
    editor.paste({ text: "\n- Pasted first\n- Pasted second" });
    endEdit("page-navigation");
    await vi.waitFor(() => {
      expect(embed.textContent).toContain("Pasted first");
      expect(embed.textContent).toContain("Pasted second");
    });
    expect(isDirty("Source")).toBe(true);
  } finally { dispose(); }
});

it.each(["page", "block"] as const)("a read-only %s embed folds through the guide and arrow without document changes", async kind => {
  const { embed, dispose, guide, toggle } = await mount("main", kind, "md", true);
  try {
    const before = pageToDto("Source");
    guide().click();
    expect(embed.textContent).not.toContain("Existing child");
    toggle(embed, "branch");
    expect(embed.textContent).toContain("Existing child");
    expect(pageToDto("Source")).toEqual(before);
    expect(isDirty("Source")).toBe(false);
    expect(isDirty("Host")).toBe(false);
  } finally { dispose(); }
});
