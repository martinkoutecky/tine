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
import { editingId, endEdit } from "../editorController";
import { journalTitle } from "../journal";
import type { RefGroup } from "../types";
import { TagPageTable, TagTableToggle } from "./Page";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  endEdit("blur");
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function page(name: string, kind: "page" | "journal", roots: string[], preBlock: string | null = null): FeedPage {
  return { name, kind, title: name, preBlock, roots, format: "md", readOnly: false };
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
