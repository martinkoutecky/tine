import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { mergeWithPrev, nextVisible, pageByName, prevVisible, resetStore, type OutlineScope } from "./document";
import { doc, setDoc } from "./document/model";
import { initParser } from "./render/parse";

// GH #341: a navOnly display-list scope (ref/query/embed group) drives arrow
// navigation through the RENDERED order while structural mutations stay on
// page order — merging into a rendered neighbor must never weld unrelated
// subtrees that merely sit adjacent in the result list.
//
// Display membership [rootA, rootB] deliberately differs from the page's own
// root order [rootB, rootA]:
//   page order     : rootB, b1, rootA, a1, a2
//   rendered order : rootA, a1, a2, rootB, b1

const navScope: OutlineScope = { roots: ["rootA", "rootB"], navOnly: true };

function installSourcePage() {
  setDoc({
    byId: {
      rootB: { id: "rootB", raw: "Root B", collapsed: false, parent: null, page: "Source", children: ["b1"] },
      b1: { id: "b1", raw: "Thing B", collapsed: false, parent: "rootB", page: "Source", children: [] },
      rootA: { id: "rootA", raw: "Root A", collapsed: false, parent: null, page: "Source", children: ["a1", "a2"] },
      a1: { id: "a1", raw: "Thing 1", collapsed: false, parent: "rootA", page: "Source", children: [] },
      a2: { id: "a2", raw: "Thing 2", collapsed: false, parent: "rootA", page: "Source", children: [] },
    },
    pages: [{
      name: "Source",
      kind: "page" as const,
      title: "Source",
      preBlock: null,
      roots: ["rootB", "rootA"],
      format: "md" as const,
      readOnly: false,
      guide: false,
    }],
    feed: [],
    loaded: true,
  });
}

beforeAll(async () => {
  await initParser();
});

beforeEach(() => {
  resetStore();
  installSourcePage();
});

describe("navOnly outline scope visible order (GH #341)", () => {
  it("walks the rendered display list, crossing root boundaries", () => {
    expect(nextVisible("a1", navScope)).toBe("a2");
    expect(nextVisible("a2", navScope)).toBe("rootB");
    expect(prevVisible("rootB", navScope)).toBe("a2");
    expect(prevVisible("rootA", navScope)).toBe(null);
  });

  it("page order answers differently — the distinction that hides the caret", () => {
    // End of the page's own order: today's null scope finds nothing here…
    expect(nextVisible("a2", null)).toBe(null);
    // …while ArrowUp from a first display root leaks to an unrelated in-page
    // block that is not rendered adjacent to it.
    expect(prevVisible("rootA", null)).toBe("b1");
  });

  it("honors the surface collapse contract instead of durable collapse flags", () => {
    const collapsedInSurface: OutlineScope = {
      roots: ["rootA", "rootB"],
      navOnly: true,
      collapsed: (id) => id === "rootA",
    };
    // rootA collapsed in the surface → its subtree is not rendered → next
    // rendered block after rootA is rootB, not a1.
    expect(nextVisible("rootA", collapsedInSurface)).toBe("rootB");
    // rootB expanded (default fallback from durable flags) → children visible.
    expect(nextVisible("rootB", collapsedInSurface)).toBe("b1");
    // Durable flags alone (no override) would walk the subtree as usual.
    expect(nextVisible("rootA", navScope)).toBe("a1");
  });

  it("merge from a navOnly scope merges the PAGE-order sibling, not the rendered neighbor", () => {
    // Backspace at the start of rootA inside a ref display whose previous
    // RENDERED row is a2 (via scope) — merging must use page order instead,
    // which is today's deliberate behavior: absorb rootA into b1.
    expect(mergeWithPrev("rootA", navScope)).toBe(true);
    expect(doc.byId.b1.raw).toBe("Thing BRoot A");
    expect(doc.byId.rootA).toBeUndefined();
    expect(doc.byId.b1.children).toEqual(["a1", "a2"]);
    expect(doc.byId.b1.parent).toBe("rootB");
    expect(pageByName("Source")!.roots).toEqual(["rootB"]);
  });

  it("merge refuses when page order has no previous sibling, sparing the rendered neighbor", () => {
    // Rendered order would merge rootB into a2 for a nav-only-reading merge —
    // page order has nothing before rootB, so nothing may happen.
    const rootsBefore = pageByName("Source")!.roots.slice();
    expect(mergeWithPrev("rootB", navScope)).toBe(false);
    expect(doc.byId.rootB.raw).toBe("Root B");
    expect(pageByName("Source")!.roots).toEqual(rootsBefore);
  });
});
