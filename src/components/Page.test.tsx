import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { Show, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import {
  doc,
  pageByName,
  readPageProperty,
  resetStore,
  setDoc,
  type FeedPage,
  type Node as StoreNode,
} from "../store";
import { editingId, endEdit, startEditing } from "../editorController";
import { journalTitle } from "../journal";
import type { RefGroup } from "../types";
import { TagPageTable, TagTableToggle } from "./Page";
import { PageView } from "./Page";
import { focusBlock, resetTabsToJournals } from "../router";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  endEdit("blur");
  resetStore();
  document.body.innerHTML = "";
  resetTabsToJournals();
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function page(name: string, kind: "page" | "journal", roots: string[], preBlock: string | null = null): FeedPage {
  return { name, kind, title: name, preBlock, roots, format: "md", readOnly: false, guide: false };
}

function node(id: string, raw: string, pageName: string, parent: string | null = null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: pageName, children };
}

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

describe("tag-page table", () => {
  it("toggles a query-sourced table and adds new rows to today's journal", async () => {
    const todayName = journalTitle(new Date());
    setDoc({
      byId: {
        existing: node("existing", "existing", todayName),
        row: node("row", "TODO Tagged row #Tag\nowner:: Martin", "Source"),
      },
      pages: [
        page("Tag", "page", []),
        page("Source", "page", ["row"]),
        page(todayName, "journal", ["existing"]),
      ],
      feed: ["Tag"],
      loaded: true,
    });
    const groups: RefGroup[] = [
      {
        page: "Source",
        kind: "page",
        blocks: [
          {
            id: "row",
            raw: doc.byId.row.raw,
            collapsed: false,
            children: [],
            marker: "TODO",
            tags: ["Tag"],
            properties: [["owner", "Martin"]],
          },
        ],
      },
    ];
    vi.spyOn(backend(), "runQuery").mockResolvedValue(groups);
    vi.spyOn(backend(), "savePage").mockResolvedValue("rev1");

    const tagPage = pageByName("Tag")!;
    const { root, dispose } = mount(() => (
      <>
        <TagTableToggle page={tagPage} />
        <Show when={readPageProperty("Tag", "tine.tag-table") === "true"}>
          <TagPageTable pageName="Tag" />
        </Show>
      </>
    ));

    await tick();
    const toggle = root.querySelector(".tag-table-toggle") as HTMLButtonElement | null;
    expect(toggle).not.toBeNull();
    toggle!.click();
    expect(readPageProperty("Tag", "tine.tag-table")).toBe("true");

    await tick();
    expect(root.textContent).toContain("Tagged row");
    expect(root.textContent).toContain("Martin");

    (root.querySelector(".sheet-add-row-ghost") as HTMLButtonElement).click();
    await tick();
    await tick();

    const today = pageByName(todayName)!;
    const newId = today.roots[today.roots.length - 1];
    expect(doc.byId[newId].raw).toMatch(/^#Tag\s*$/);
    expect(editingId()).toBe(newId);

    dispose();
  });
});

describe("zoomed block view", () => {
  it("reveals a collapsed root's children without changing its stored collapse state", async () => {
    const parent = "11111111-1111-4111-8111-111111111111";
    const child = "22222222-2222-4222-8222-222222222222";
    const dto = {
      name: "Outline",
      kind: "page" as const,
      title: "Outline",
      pre_block: null,
      blocks: [{
        id: parent,
        raw: "Collapsed section\ncollapsed:: true\nid:: 11111111-1111-4111-8111-111111111111",
        collapsed: true,
        children: [{ id: child, raw: "Hidden child", collapsed: false, children: [] }],
      }],
    };
    setDoc({
      byId: {
        [parent]: { ...node(parent, dto.blocks[0].raw, dto.name, null, [child]), collapsed: true },
        [child]: node(child, "Hidden child", dto.name, parent),
      },
      pages: [page(dto.name, "page", [parent])],
      feed: [dto.name],
      loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    focusBlock(parent);

    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      expect(root.querySelector(`[data-block-id="${child}"]`)).not.toBeNull();
      expect(doc.byId[parent].collapsed).toBe(true);
    } finally {
      dispose();
    }
  });

  it("Enter at a collapsed zoom root creates and focuses a rendered child, not an outside sibling", async () => {
    const parent = "11111111-1111-4111-8111-111111111111";
    const oldChild = "22222222-2222-4222-8222-222222222222";
    const outside = "33333333-3333-4333-8333-333333333333";
    const dto = {
      name: "Outline",
      kind: "page" as const,
      title: "Outline",
      pre_block: null,
      blocks: [
        { id: parent, raw: "Root\ncollapsed:: true", collapsed: true, children: [{ id: oldChild, raw: "Old", collapsed: false, children: [] }] },
        { id: outside, raw: "Outside", collapsed: false, children: [] },
      ],
    };
    setDoc({
      byId: {
        [parent]: { ...node(parent, dto.blocks[0].raw, dto.name, null, [oldChild]), collapsed: true },
        [oldChild]: node(oldChild, "Old", dto.name, parent),
        [outside]: node(outside, "Outside", dto.name),
      },
      pages: [page(dto.name, "page", [parent, outside])], feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
    focusBlock(parent);
    const { root, dispose } = mount(() => <PageView />);
    try {
      await tick();
      await tick();
      startEditing(parent, 0);
      await tick();
      const textarea = root.querySelector("textarea") as HTMLTextAreaElement;
      textarea.selectionStart = 0;
      textarea.selectionEnd = 0;
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
      await tick();
      const created = doc.byId[parent].children[0];
      expect(created).not.toBe(oldChild);
      expect(editingId()).toBe(created);
      expect(root.querySelector(`[data-block-id="${created}"] textarea`)).not.toBeNull();
      expect(doc.pages[0].roots).toEqual([parent, outside]);
      expect(doc.byId[parent].collapsed).toBe(true);
    } finally {
      dispose();
    }
  });
});
