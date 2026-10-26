// GH #726: the host and each embedded source row have distinct bullets.
// OG 0.10.9 block.cljs block-embed/page-embed render source blocks inside
// the host's content. Real Cut inside an embed is an intentional OG difference.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { For } from "solid-js";
import { backend } from "../backend";
import { clearClipboardPayload, peekClipboardPayload } from "../clipboard";
import { doc } from "../document/model";
import { extendSelectionTo, loadFeed, pageByName, resetStore, selectBlock, selectedIds, undo } from "../document";
import { initParser } from "../render/parse";
import { closeContextMenu, contextMenu } from "../ui";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";

beforeAll(initParser);
afterEach(() => {
  closeContextMenu();
  clearClipboardPayload();
  resetStore();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

const leaf = (id: string, raw: string, children: BlockDto[] = []): BlockDto =>
  ({ id, raw, collapsed: false, children });

function mount(kind: "block" | "page") {
  const target = leaf("source", "Source root\nid:: source", [leaf("child", "Source child")]);
  const source: PageDto = { name: "Source", title: "Source", kind: "page", pre_block: null, blocks: [target] };
  const macro = kind === "block" ? "{{embed ((source))}}" : "{{embed [[Source]]}}";
  loadFeed([
    { name: "Host", title: "Host", kind: "page", pre_block: null, blocks: [leaf("host", macro)] },
    source,
  ]);
  vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{ page: "Source", kind: "page", blocks: [target] }]);
  vi.spyOn(backend(), "getPage").mockResolvedValue(source as never);
  vi.spyOn(backend(), "writeRich").mockResolvedValue();
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <><ContextMenu /><For each={pageByName("Host")?.roots}>{id => <Block id={id} />}</For></>, root);
  return { root, dispose, macro };
}

async function bullet(root: Element, id: string): Promise<Element> {
  return vi.waitFor(() => {
    const el = root.querySelector(`[data-block-id="${id}"] > .block-main > .block-controls .bullet-container`);
    expect(el, `${id} has its own bullet`).not.toBeNull();
    return el!;
  });
}

function rightClick(el: Element) {
  el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 }));
}

describe.each(["block", "page"] as const)("GH #726 %s embed bullet ownership", (kind) => {
  it("gives the embed host its own bullet and menu alongside the source bullet", async () => {
    const { root, dispose } = mount(kind);
    try {
      await bullet(root, "source");
      rightClick(await bullet(root, "host"));
      expect(contextMenu()).toMatchObject({ kind: "block", blockId: "host" });
    } finally { dispose(); }
  });

  it.each(["Copy block", "Cut block", "Delete block"])("%s on the host acts only on the embed block", async (action) => {
    const { root, dispose, macro } = mount(kind);
    try {
      await bullet(root, "source");
      rightClick(await bullet(root, "host"));
      const item = [...root.querySelectorAll<HTMLElement>(".ctx-item")].find(el => el.textContent?.trim() === action);
      expect(item).toBeDefined();
      item!.click();
      if (action === "Copy block") {
        expect(peekClipboardPayload()?.blocks[0].raw).toBe(macro);
      } else {
        await vi.waitFor(() => expect(doc.byId.host).toBeUndefined());
        undo();
        expect(doc.byId.host.raw).toBe(macro);
      }
      expect(doc.byId.source.raw).toContain("id:: source");
      expect(doc.byId.source.children).toEqual(["child"]);
    } finally { dispose(); }
  });

  it.each([["host", "source"], ["source", "host"]])("refuses selection from %s across the embed boundary to %s", async (from, to) => {
    const { root, dispose } = mount(kind);
    try {
      await bullet(root, "source");
      selectBlock(from, { roots: [from], navOnly: true });
      extendSelectionTo(to);
      expect(selectedIds()).toEqual([from]);
      expect(doc.byId.host).toBeDefined();
      expect(doc.byId.source).toBeDefined();
    } finally { dispose(); }
  });

  it.each(["Copy block", "Cut block", "Delete block"])("%s on the source root acts on the source and keeps the embed", async (action) => {
    const { root, dispose, macro } = mount(kind);
    try {
      rightClick(await bullet(root, "source"));
      expect(contextMenu()).toMatchObject({ kind: "block", blockId: "source" });
      const item = [...root.querySelectorAll<HTMLElement>(".ctx-item")].find(el => el.textContent?.trim() === action);
      expect(item, action).toBeDefined();
      item!.click();
      if (action === "Copy block") {
        expect(peekClipboardPayload()?.blocks[0].raw).toContain("Source root");
        expect(doc.byId.source).toBeDefined();
      } else {
        await vi.waitFor(() => expect(doc.byId.source).toBeUndefined());
        expect(doc.byId.child).toBeUndefined();
        undo();
        expect(doc.byId.source.raw).toContain("id:: source");
        expect(doc.byId.source.children).toEqual(["child"]);
      }
      expect(doc.byId.host.raw).toBe(macro);
    } finally { dispose(); }
  });
});
