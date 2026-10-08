// GH #262: hierarchical Ctrl/Cmd+A expansion and Shift+Up subtree
// completion. Store-level semantics: visible-order slices, no DOM.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { initParser } from "./render/parse";
import {
  clearSeededFacets,
} from "./render/facets";
import {
  clearSelection,
  expandBlockSelection,
  moveSelection,
  pageByName,
  resetStore,
  selectBlock,
  selectBlockSubtree,
  selectedIds,
  type OutlineScope,
} from "./document";
import { doc } from "./document/model";
import { loadSingle } from "./document/workingSet";
import type { BlockDto, PageDto } from "./types";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
});

let counter = 0;
function blk(raw: string, children: BlockDto[] = []): BlockDto {
  return { id: `t${counter++}`, raw, collapsed: false, children };
}
function load(blocks: BlockDto[]): PageDto {
  const dto: PageDto = { name: "Tree", kind: "page", title: "Tree", pre_block: null, blocks };
  loadSingle(dto);
  clearSeededFacets();
  return dto;
}

/** P[c1, c2[g1, g2], c3] + Sibling root. */
function loadTree() {
  const c1 = blk("c1");
  const g1 = blk("g1");
  const g2 = blk("g2");
  const c2 = blk("c2", [g1, g2]);
  const c3 = blk("c3");
  const p = blk("P", [c1, c2, c3]);
  const s = blk("S");
  load([p, s]);
  return { P: p.id, c1: c1.id, c2: c2.id, g1: g1.id, g2: g2.id, c3: c3.id, S: s.id };
}

describe("Shift+Up subtree completion (GH #262)", () => {
  it("extending upward onto the anchor's parent completes its visible subtree", () => {
    // Sibling root S sits ABOVE P so the focus can keep climbing past P.
    const c1 = blk("c1");
    const g1 = blk("g1");
    const g2 = blk("g2");
    const c2 = blk("c2", [g1, g2]);
    const c3 = blk("c3");
    const p = blk("P", [c1, c2, c3]);
    const s = blk("S");
    load([s, p]);
    const t = { P: p.id, c1: c1.id, c2: c2.id, g1: g1.id, g2: g2.id, c3: c3.id, S: s.id };
    selectBlock(t.c2);
    moveSelection(-1, true);
    expect(selectedIds()).toEqual([t.c1, t.c2]); // no parent in slice — unchanged
    moveSelection(-1, true); // focus on P
    expect(selectedIds()).toEqual([t.P, t.c1, t.c2, t.g1, t.g2, t.c3]); // subtree completed
    moveSelection(-1, true); // focus above P: P's subtree stays whole
    expect(selectedIds()).toEqual([t.S, t.P, t.c1, t.c2, t.g1, t.g2, t.c3]);
  });

  it("Shift+Down keeps its existing forward shape", () => {
    const t = loadTree();
    selectBlock(t.P);
    moveSelection(1, true);
    expect(selectedIds()).toEqual([t.P, t.c1]); // unchanged forward behavior
  });
});

describe("hierarchical Ctrl/Cmd+A expansion (GH #262)", () => {
  it("climbs subtree → parent subtree → whole visible outline, idempotent at the top", () => {
    const t = loadTree();
    selectBlockSubtree(t.c2);
    expect(selectedIds()).toEqual([t.c2, t.g1, t.g2]);
    expandBlockSelection();
    expect(selectedIds()).toEqual([t.P, t.c1, t.c2, t.g1, t.g2, t.c3]);
    expandBlockSelection();
    expect(selectedIds()).toEqual([t.P, t.c1, t.c2, t.g1, t.g2, t.c3, t.S]);
    expandBlockSelection(); // top of the ladder: no-op
    expect(selectedIds()).toEqual([t.P, t.c1, t.c2, t.g1, t.g2, t.c3, t.S]);
  });

  it("a collapsed parent is its own whole visible subtree", () => {
    const p = blk("P", [blk("c1")]);
    p.collapsed = true;
    const s = blk("S");
    load([p, s]);
    selectBlockSubtree(p.id);
    expect(selectedIds()).toEqual([p.id]); // hidden children are not selected
    expandBlockSelection();
    expect(selectedIds()).toEqual([p.id, s.id]); // climbs to the whole outline
  });

  it("respects the active outline scope: the ladder tops out at the scoped order", () => {
    const t = loadTree();
    const scope: OutlineScope = { roots: [t.P] };
    selectBlockSubtree(t.c2, scope);
    expect(selectedIds()).toEqual([t.c2, t.g1, t.g2]);
    expandBlockSelection();
    expect(selectedIds()).toEqual([t.P, t.c1, t.c2, t.g1, t.g2, t.c3]);
    expandBlockSelection(); // root of the scope: whole scoped order
    expect(selectedIds()).toEqual([t.P, t.c1, t.c2, t.g1, t.g2, t.c3]);
    expandBlockSelection(); // idempotent
    expect(selectedIds()).toEqual([t.P, t.c1, t.c2, t.g1, t.g2, t.c3]);
  });

  it("restarts the ladder at the anchor after the selection is changed", () => {
    const t = loadTree();
    selectBlockSubtree(t.c2);
    moveSelection(1, true); // focus beyond the subtree (resets the ladder)
    expandBlockSelection();
    expect(selectedIds()).toContain(t.P); // climbs, never shrinks
    expect(selectedIds()).toContain(t.c3);
  });

  it("does nothing without a selection", () => {
    loadTree();
    clearSelection();
    expandBlockSelection();
    expect(selectedIds()).toEqual([]);
  });

  it("undo-safe consumers receive the expanded ids: copy payload uses them verbatim", () => {
    const t = loadTree();
    selectBlockSubtree(t.c2);
    expandBlockSelection();
    // The selection drives copy/cut/move consumers directly.
    expect(selectedIds()).toEqual([t.P, t.c1, t.c2, t.g1, t.g2, t.c3]);
    expect(pageByName("Tree")!.roots).toHaveLength(2); // selection is not a mutation
    expect(doc.byId[t.P].children).toHaveLength(3);
  });
});
