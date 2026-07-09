import { afterEach, describe, expect, it } from "vitest";
import { ensurePageLoaded, pageByName, resetStore, selectBlock, isSelected, moveSelection, selectedIds } from "./store";
import type { PageDto } from "./types";

afterEach(() => { resetStore(); });

function blk(id: string, raw: string) { return { id, raw, collapsed: false, children: [] }; }
function page(name: string): PageDto {
  return { name, kind: "page", title: name, pre_block: null, blocks: [blk("p1", "alpha"), blk("p2", "beta"), blk("p3", "gamma")] };
}

describe("block selection on a routed (non-feed) page", () => {
  it("selects and walks blocks on a page loaded via ensurePageLoaded (not the feed)", () => {
    ensurePageLoaded(page("SelTest"));
    const roots = pageByName("SelTest")!.roots;
    expect(roots.length).toBe(3);

    // Esc-from-editing calls selectBlock; the block should become selected.
    selectBlock(roots[0]);
    expect(isSelected(roots[0])).toBe(true);
    expect(selectedIds()).toEqual([roots[0]]);

    // Arrow walks the selection down the page.
    moveSelection(1, false);
    expect(selectedIds()).toEqual([roots[1]]);

    // Shift+Arrow extends.
    moveSelection(1, true);
    expect(selectedIds()).toEqual([roots[1], roots[2]]);
  });
});
