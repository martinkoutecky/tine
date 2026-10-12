import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { isDirty, pageByName, readPageProperty, resetStore } from "../document";
import { backend } from "../backend";
import { loadSingle } from "../document/workingSet";
import { editingId } from "../editorController";
import { pagePropsPanel, closePageProps } from "../ui";
import type { BlockDto, PageDto } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";

// GH #486: a page whose page-level properties link here is listed under Linked
// References as its properties row. OG lists the referencing page's
// properties block (its pre-block) as an ordinary editable block, so a typo in a
// property name or value can be fixed in place (og-1.0.0
// frontend/components/block.cljs block-content -> properties-cp, `pre-block?`
// rendered as "page-properties" and edited like any block). Tine's edit path for
// that block on the source page is the page-header editor (Page.tsx
// editPageHeader -> beginPageHeaderEdit -> startEditing).

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  closePageProps();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

const HEADER = "tags:: Target\ntypo-key:: book";

function sourcePage(format: "md" | "org" = "md"): PageDto {
  return {
    name: "Source", title: "Source", kind: "page", format,
    pre_block: format === "md" ? `${HEADER}\n` : "#+tags: Target\n#+typo-key: book\n",
    blocks: [{ id: "body", raw: "Body block", collapsed: false, children: [] }],
  } as PageDto;
}

const propertyRow = (raw = HEADER): BlockDto => ({
  id: "page-property:Page:source", raw, collapsed: false, children: [], page_property: true,
});

const replaceValue = (textarea: HTMLTextAreaElement, value: string) => {
  textarea.value = value;
  textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value }));
};

describe("page properties in Linked References (GH #486)", () => {
  it("clicking the properties row edits the source page's properties in place", async () => {
    vi.spyOn(backend(), "getPage").mockResolvedValue(sourcePage());
    const { root, dispose } = mount(() => (
      <LiveRefGroup page="Source" kind="page" blocks={[propertyRow()]} surface="ref" eager />
    ));
    try {
      await vi.waitFor(() => expect(pageByName("Source")).toBeDefined());
      const row = await vi.waitFor(() => {
        const el = root.querySelector(".page-property-reference .block-content");
        expect(el).not.toBeNull();
        return el as HTMLElement;
      });
      row.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
      const textarea = await vi.waitFor(() => {
        const el = root.querySelector("textarea");
        expect(el).not.toBeNull();
        return el as HTMLTextAreaElement;
      });
      expect(textarea.value).toBe(HEADER);
      replaceValue(textarea, "tags:: Target\ntype:: book");
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(editingId()).toBeNull());
      expect(readPageProperty("Source", "type")).toBe("book");
      expect(readPageProperty("Source", "typo-key")).toBeNull();
      expect(isDirty("Source")).toBe(true);
    } finally {
      dispose();
    }
  });

  it("an org page opens its properties panel instead (no header editor there)", async () => {
    vi.spyOn(backend(), "getPage").mockResolvedValue(sourcePage("org"));
    const { root, dispose } = mount(() => (
      <LiveRefGroup page="Source" kind="page" blocks={[propertyRow("#+tags: Target\n#+typo-key: book")]} surface="ref" eager />
    ));
    try {
      await vi.waitFor(() => expect(pageByName("Source")).toBeDefined());
      const row = await vi.waitFor(() => {
        const el = root.querySelector(".page-property-reference .block-content");
        expect(el).not.toBeNull();
        return el as HTMLElement;
      });
      row.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, clientX: 10, clientY: 20 }));
      await vi.waitFor(() => expect(pagePropsPanel()?.scope).toEqual({ kind: "page", name: "Source" }));
    } finally {
      dispose();
    }
  });
});
