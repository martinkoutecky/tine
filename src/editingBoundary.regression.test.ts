import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "./render/parse";
import {
  blockSubtreeMarkdown,
  clearSelection,
  deleteBlock,
  deleteSelection,
  doc,
  moveSelection,
  flushPage,
  forceSave,
  markDirty,
  resetStore,
  selectBlock,
  selectedIds,
  setBlockProperty,
  setDoc,
  splitBlock,
  toggleCollapse,
  type FeedPage,
  type Node,
} from "./store";
import { dirtyPages } from "./persistence";
import { backend } from "./backend";

beforeAll(() => initParser());

beforeEach(() => {
  resetStore();
  clearSelection();
});

function page(name: string, roots: string[], format: "md" | "org" = "md", readOnly = false): FeedPage {
  return { name, kind: "page", title: name, preBlock: null, roots, format, readOnly, guide: false };
}

function node(
  id: string,
  raw: string,
  pageName: string,
  parent: string | null = null,
  children: string[] = [],
  collapsed = false,
): Node {
  return { id, raw, page: pageName, parent, children, collapsed };
}

describe("editing/collapse boundary regressions", () => {
  it("read-only pages reject collapse, property, direct delete, and selection delete mutations", () => {
    setDoc({
      byId: { p: node("p", "Parent", "Org", null, ["c"]), c: node("c", "Child", "Org", "p") },
      pages: [page("Org", ["p"], "org", true)], feed: ["Org"], loaded: true,
    });
    const before = JSON.stringify(doc);
    toggleCollapse("p");
    setBlockProperty("p", "heading", "2");
    deleteBlock("c");
    selectBlock("c");
    deleteSelection();
    expect(JSON.stringify(doc)).toBe(before);
    expect([...dirtyPages()]).not.toContain("Org");
  });

  it("read-only pages cannot enter the dirty/save pipeline even through a direct call", async () => {
    setDoc({
      byId: { p: node("p", "Parent", "Org") },
      pages: [page("Org", ["p"], "org", true)], feed: ["Org"], loaded: true,
    });
    const save = vi.spyOn(backend(), "savePage");
    markDirty("Org");
    expect([...dirtyPages()]).not.toContain("Org");
    expect(await flushPage("Org")).toBe(true);
    expect(save).not.toHaveBeenCalled();
    expect(await forceSave("Org")).toBe(false);
    expect(save).not.toHaveBeenCalled();
  });

  it("selection in a collapsed zoom scope includes rendered children and excludes page siblings", () => {
    setDoc({
      byId: {
        a: node("a", "A\ncollapsed:: true", "P", null, ["a1"], true),
        a1: node("a1", "A1", "P", "a"), b: node("b", "B", "P"),
      },
      pages: [page("P", ["a", "b"])], feed: [], loaded: true,
    });
    const zoomScope = { roots: ["a"], forceExpandedRoot: "a" };
    (selectBlock as unknown as (id: string, scope: typeof zoomScope) => void)("a1", zoomScope);
    expect(selectedIds()).toEqual(["a1"]);
    moveSelection(1, true);
    expect(selectedIds()).toEqual(["a1"]);
    deleteSelection();
    expect(doc.byId.a.children).toEqual([]);
    expect(doc.byId.b.raw).toBe("B");
  });

  it("Enter at the head of a zoom root creates an in-scope child like OG", () => {
    setDoc({
      byId: {
        a: node("a", "Root\ncollapsed:: true", "P", null, ["old"], true),
        old: node("old", "Old child", "P", "a"),
        b: node("b", "Outside sibling", "P"),
      },
      pages: [page("P", ["a", "b"])], feed: [], loaded: true,
    });
    splitBlock("a", 0, true, true);
    const created = doc.byId.a.children[0];
    expect(doc.byId.a.raw).toBe("Root\ncollapsed:: true");
    expect(doc.byId[created].raw).toBe("");
    expect(doc.byId[created].parent).toBe("a");
    expect(doc.pages[0].roots).toEqual(["a", "b"]);
  });

  it("persists collapse in an Org drawer and removes it without Markdown syntax", () => {
    setDoc({
      byId: {
        p: node("p", "Parent\n:PROPERTIES:\n:owner: Martin\n:END:", "Org", null, ["c"]),
        c: node("c", "Child", "Org", "p"),
      },
      pages: [page("Org", ["p"], "org")], feed: ["Org"], loaded: true,
    });
    toggleCollapse("p");
    expect(doc.byId.p.raw).toBe("Parent\n:PROPERTIES:\n:owner: Martin\n:collapsed: true\n:END:");
    expect(doc.byId.p.raw).not.toContain("collapsed::");
    toggleCollapse("p");
    expect(doc.byId.p.raw).toBe("Parent\n:PROPERTIES:\n:owner: Martin\n:END:");
  });

  it("creates and removes a built-in-only Org collapse drawer canonically", () => {
    setDoc({
      byId: { p: node("p", "Parent", "Org", null, ["c"]), c: node("c", "Child", "Org", "p") },
      pages: [page("Org", ["p"], "org")], feed: ["Org"], loaded: true,
    });
    toggleCollapse("p");
    expect(doc.byId.p.raw).toBe("Parent\n:PROPERTIES:\n:collapsed: true\n:END:");
    toggleCollapse("p");
    expect(doc.byId.p.raw).toBe("Parent");
  });

  it("copies an Org subtree as OG-style Markdown while stripping real IDs but retaining literal source content", () => {
    setDoc({
      byId: {
        p: node("p", "Parent\n:PROPERTIES:\n:id: real-parent\n:END:\n#+BEGIN_SRC text\n:id: literal-code\n#+END_SRC", "Org", null, ["c"]),
        c: node("c", "Child\n:PROPERTIES:\n:id: real-child\n:END:", "Org", "p"),
      },
      pages: [page("Org", ["p"], "org")], feed: ["Org"], loaded: true,
    });
    const copied = blockSubtreeMarkdown("p", 0, true);
    expect(copied).toContain("- Parent");
    expect(copied).toContain("\t- Child");
    expect(copied).toContain(":id: literal-code");
    expect(copied).not.toContain(":id: real-parent");
    expect(copied).not.toContain(":id: real-child");
  });

  it("splits an ordered Org block without introducing Markdown properties", () => {
    setDoc({
      byId: { item: node("item", "Hello world\n:PROPERTIES:\n:logseq.order-list-type: number\n:END:", "Org") },
      pages: [page("Org", ["item"], "org")], feed: ["Org"], loaded: true,
    });
    splitBlock("item", 5);
    const next = doc.pages[0].roots[1];
    expect(doc.byId[next].raw).toContain(":logseq.order-list-type: number");
    expect(doc.byId[next].raw).not.toContain("logseq.order-list-type::");
  });

  it("keeps metadata-only ordered Org splits canonical at the head and end", () => {
    const ordered = "Item\n:PROPERTIES:\n:logseq.order-list-type: number\n:END:";
    setDoc({
      byId: { item: node("item", ordered, "Org") },
      pages: [page("Org", ["item"], "org")], feed: ["Org"], loaded: true,
    });
    splitBlock("item", 0);
    const head = doc.pages[0].roots[0];
    expect(doc.byId[head].raw).toBe(":PROPERTIES:\n:logseq.order-list-type: number\n:END:");

    resetStore();
    setDoc({
      byId: { item: node("item", ordered, "Org") },
      pages: [page("Org", ["item"], "org")], feed: ["Org"], loaded: true,
    });
    splitBlock("item", "Item".length);
    const end = doc.pages[0].roots[1];
    expect(doc.byId[end].raw).toBe(":PROPERTIES:\n:logseq.order-list-type: number\n:END:");
  });
});
