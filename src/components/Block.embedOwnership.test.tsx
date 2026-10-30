// GH #726: a page embed's host and each embedded source row have distinct
// bullets. OG 0.10.9 block.cljs block-embed/page-embed render source blocks
// inside the host's content. Real Cut inside an embed is an intentional OG
// difference. D-20261030-EMB (back to 0.8): a whole-block embed shows ONE
// bullet, the source root's row, and that bullet's menu acts on the host.
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

describe("D-20261030-EMB block embed: one bullet, owned by the host", () => {
  it("shows no host bullet; the source root's bullet opens the host's menu", async () => {
    const { root, dispose } = mount("block");
    try {
      rightClick(await bullet(root, "source"));
      expect(root.querySelector('[data-block-id="host"] > .block-main > .block-controls')).toBeNull();
      expect(root.querySelectorAll(".bullet-container")).toHaveLength(2); // root + child
      expect(contextMenu()).toMatchObject({ kind: "block", blockId: "host" });
    } finally { dispose(); }
  });

  it.each(["Copy block", "Cut block", "Delete block"])("%s on the one bullet acts on the host and keeps the source", async (action) => {
    const { root, dispose, macro } = mount("block");
    try {
      rightClick(await bullet(root, "source"));
      const item = [...root.querySelectorAll<HTMLElement>(".ctx-item")].find(el => el.textContent?.trim() === action);
      expect(item, action).toBeDefined();
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
});

describe.each(["block", "page"] as const)("GH #726 %s embed selection boundary", (kind) => {
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
});

describe("GH #726 page embed bullet ownership", () => {
  const kind = "page" as const;
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
