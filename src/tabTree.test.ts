// GH #704 v2 (DESIGN 2026-10-vertical-tabs): the pane's tab tree. T1 (the tab
// array is the tree in depth-first order) and T2 (pinned tabs are roots and
// never parents, and come first) must hold after every router mutation, and a
// session restores the tree, or flat, but is never refused.
import { describe, expect, it, vi } from "vitest";
import { createPaneRouter, type PaneRouter, type PaneSnapshot, type SerializedTab, type Tab } from "./router";
import {
  moveTabOutOfGroup, moveTabSubtree, normalizeTabTree, removeTabPromoting, settleTabTree, tabDepths,
  type TreeTab,
} from "./tabTree";
import { backend } from "./backend";

const tt = (id: string, parent: string | null = null, extra: Partial<TreeTab> = {}): TreeTab =>
  ({ id, parent, pinned: false, collapsed: false, ...extra });
const shape = (list: readonly TreeTab[]) => {
  const d = tabDepths(list);
  return list.map((t, i) => `${"-".repeat(d[i])}${t.id}${t.pinned ? "*" : ""}${t.collapsed ? "+" : ""}`).join(" ");
};

/** T1/T2 checked from first principles (not with the code under test). */
function assertTree(list: readonly TreeTab[]) {
  const ids = list.map((t) => t.id);
  expect(new Set(ids).size).toBe(ids.length);
  let seenUnpinned = false;
  list.forEach((t, i) => {
    if (!t.pinned) seenUnpinned = true;
    else expect(seenUnpinned, `pinned ${t.id} after an unpinned tab`).toBe(false);
    if (t.pinned) expect(t.parent, `pinned ${t.id} is nested`).toBeNull();
    if (t.parent === null) return;
    const p = ids.indexOf(t.parent);
    expect(p, `${t.id}'s parent is earlier`).toBeGreaterThanOrEqual(0);
    expect(p).toBeLessThan(i);
    expect(list[p].pinned, `${t.id} nested under pinned`).toBe(false);
    // Contiguity: every tab between the parent and this one descends from the parent.
    for (let k = p + 1; k < i; k++) {
      let a: string | null = list[k].parent;
      while (a !== null && a !== t.parent) a = list[ids.indexOf(a)].parent;
      expect(a, `${list[k].id} sits inside ${t.parent}'s subtree`).toBe(t.parent);
    }
  });
  for (const t of list) if (t.collapsed) expect(list.some((c) => c.parent === t.id)).toBe(true);
}

/** A seeded LCG in exact 32-bit integer arithmetic (a float product loses the
 *  low bits), drawing from the high bits. */
function seeded(seed: number) {
  let state = seed >>> 0;
  return (n: number) => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return Math.floor((state / 0x100000000) * n);
  };
}

describe("tab tree (pure)", () => {
  it("normalize keeps a valid tree as the same array and repairs every broken link", () => {
    const ok = [tt("a"), tt("b", "a"), tt("c", "b"), tt("d", "a"), tt("e")];
    expect(normalizeTabTree(ok)).toBe(ok);
    expect(shape(ok)).toBe("a -b --c -d e");
    // missing parent, later parent, subtree already ended, pinned parent, self
    const broken = [tt("p", null, { pinned: true }), tt("a", "p"), tt("b", "zz"), tt("c", "d"), tt("d"), tt("e", "a"), tt("f", "f"), tt("g", null, { collapsed: true })];
    const fixed = normalizeTabTree(broken);
    assertTree(fixed);
    expect(shape(fixed)).toBe("p* a b c d e f g");
  });

  it("removing a parent promotes its children one level, in place", () => {
    const list = [tt("a"), tt("b", "a", { collapsed: true }), tt("c", "b"), tt("d", "b"), tt("e", "a"), tt("f")];
    expect(shape(removeTabPromoting(list, "b"))).toBe("a -c -d -e f");
    expect(shape(removeTabPromoting(list, "a"))).toBe("b+ -c -d e f");
  });

  it("a subtree moves whole; depth clamps to the slot; nesting into a collapsed tab expands it", () => {
    const list = [tt("a", null, { collapsed: true }), tt("a1", "a"), tt("b"), tt("b1", "b"), tt("c")];
    expect(shape(moveTabSubtree(list, "b", null, 0))).toBe("a+ -a1 c b -b1");
    expect(shape(moveTabSubtree(list, "c", "b", 1))).toBe("a -a1 -c b -b1");
    // Asking deeper than "one under the row above" clamps.
    expect(shape(moveTabSubtree(list, "c", "b", 9))).toBe("a -a1 --c b -b1");
    // Before a child, a root is not possible: at least the depth of the row below.
    expect(shape(moveTabSubtree(list, "c", "b1", 0))).toBe("a+ -a1 b -c -b1");
    // Onto its own subtree: no move.
    expect(shape(moveTabSubtree(list, "b", "b1", "flat"))).toBe(shape(list));
  });

  it("a flat (strip / overview) move never creates or changes a parent: it lands on the nearest boundary that keeps it", () => {
    const list = [tt("a"), tt("a1", "a"), tt("a2", "a"), tt("b"), tt("c")];
    // A child moves among its siblings.
    expect(shape(moveTabSubtree(list, "a2", "a1", "flat"))).toBe("a -a2 -a1 b c");
    // Past its group: stays the group's last child, never un-nests.
    expect(shape(moveTabSubtree(list, "a2", null, "flat"))).toBe("a -a1 -a2 b c");
    expect(shape(moveTabSubtree(list, "a1", "c", "flat"))).toBe("a -a2 -a1 b c");
    // A root dropped inside a group lands before or after the group, never in it
    // (moving up: the earlier boundary on a tie; moving down: the later one).
    expect(shape(moveTabSubtree(list, "c", "a1", "flat"))).toBe("c a -a1 -a2 b");
    expect(shape(moveTabSubtree(list, "c", "a2", "flat"))).toBe("a -a1 -a2 c b");
    const down = [tt("z"), tt("a"), tt("a1", "a"), tt("a2", "a"), tt("b")];
    expect(shape(moveTabSubtree(down, "z", "a2", "flat"))).toBe("a -a1 -a2 z b");
    // The review's case: A > A1, B, C; C dropped between A and A1.
    const review = [tt("A"), tt("A1", "A"), tt("B"), tt("C")];
    const moved = moveTabSubtree(review, "C", "A1", "flat");
    expect(moved.find((t) => t.id === "C")!.parent).toBeNull();
    expect(shape(moved)).toBe("C A -A1 B");
    // A flat move of a child of a collapsed group keeps it collapsed.
    const closed = [tt("a", null, { collapsed: true }), tt("a1", "a"), tt("a2", "a"), tt("b")];
    expect(shape(moveTabSubtree(closed, "a2", "a1", "flat"))).toBe("a+ -a2 -a1 b");
  });

  it("never nests under or into a pinned tab, and a pinned tab stays a root up front", () => {
    const list = [tt("p", null, { pinned: true }), tt("q", null, { pinned: true }), tt("a"), tt("a1", "a")];
    expect(shape(moveTabSubtree(list, "a", "q", 1))).toBe("p* q* a -a1");
    expect(shape(moveTabSubtree(list, "p", null, 3))).toBe("q* p* a -a1");
    expect(shape(moveTabSubtree(list, "a1", "q", 1))).toBe("p* q* a1 a");
  });

  it("move out of group lands right after the parent's subtree, one level up", () => {
    const list = [tt("a"), tt("b", "a"), tt("c", "b"), tt("d", "b"), tt("e", "a"), tt("f")];
    expect(shape(moveTabOutOfGroup(list, "c"))).toBe("a -b --d -c -e f");
    expect(shape(moveTabOutOfGroup(list, "b"))).toBe("a -e b -c -d f");
    expect(moveTabOutOfGroup(list, "a")).toBe(list);
  });

  it("random edit sequences keep T1/T2 (property-style, seeded)", () => {
    const rnd = seeded(704);
    const ops = [0, 0, 0, 0, 0, 0];
    let nestedSteps = 0;
    let collapsedSteps = 0;
    let steps = 0;
    for (let run = 0; run < 200; run++) {
      let list: TreeTab[] = [];
      let next = 0;
      for (let step = 0; step < 40; step++) {
        const pick = () => list[rnd(list.length)]?.id;
        // Opens and nesting moves are drawn twice as often, so trees grow deep.
        const op = [0, 0, 1, 2, 2, 3, 4, 5][rnd(8)];
        ops[op]++;
        switch (op) {
          case 0: list = settleTabTree([...list, tt(`t${next++}`, null, { pinned: rnd(8) === 0 })]); break;
          case 1: if (list.length) list = removeTabPromoting(list, pick()!); break;
          case 2: if (list.length) list = moveTabSubtree(list, pick()!, rnd(4) ? pick() ?? null : null, rnd(4) ? 1 + rnd(3) : 0); break;
          case 3: if (list.length) {
            const before = new Map(list.map((t) => [t.id, t.parent] as const));
            list = moveTabSubtree(list, pick()!, pick() ?? null, "flat");
            expect(new Map(list.map((t) => [t.id, t.parent] as const))).toEqual(before);
          } break;
          case 4: if (list.length) list = moveTabOutOfGroup(list, pick()!); break;
          case 5: if (list.length) {
            const parents = list.filter((t) => list.some((c) => c.parent === t.id));
            const id = parents.length && rnd(4) ? parents[rnd(parents.length)].id : pick()!;
            list = normalizeTabTree(list.map((t) => t.id === id ? { ...t, collapsed: rnd(4) !== 0 } : t));
          } break;
        }
        assertTree(list);
        steps++;
        if (list.some((t) => t.parent !== null)) nestedSteps++;
        if (list.some((t) => t.collapsed)) collapsedSteps++;
      }
    }
    // The sequences really exercise every edit on real trees.
    for (const count of ops) expect(count).toBeGreaterThan(steps / 8 / 2);
    expect(nestedSteps).toBeGreaterThan(steps / 4); // each run starts empty
    expect(collapsedSteps).toBeGreaterThan(steps / 20); // nesting into a collapsed tab opens it
    if (process.env.TAB_TREE_STATS) console.log("pure", { ops, steps, nestedSteps, collapsedSteps });
  });
});

const page = (name: string) => ({ kind: "page" as const, name, pageKind: "page" as const });
const names = (r: PaneRouter) => {
  const d = tabDepths(r.tabs());
  return r.tabs().map((t, i) => `${"-".repeat(d[i])}${(r.tabRoute(t) as { name?: string }).name ?? t.history[0].kind}${t.pinned ? "*" : ""}${t.collapsed ? "+" : ""}`).join(" ");
};
const idOf = (r: PaneRouter, name: string) => r.tabs().find((t) => (r.tabRoute(t) as { name?: string }).name === name)!.id;
const snap = (tabs: SerializedTab[], activeIndex = 0): PaneSnapshot => ({ tabs, activeIndex });
const st = (name: string, extra: Partial<SerializedTab> = {}): SerializedTab => ({ history: [page(name)], pos: 0, pinned: false, ...extra });

/** A pane with A > (B > C), D, built through the sidebar's own verb. */
function nestedPane(id = "tree-a") {
  const r = createPaneRouter(id);
  r.restoreSnapshot(snap([st("A"), st("B"), st("C"), st("D")]));
  r.moveTabInTree(idOf(r, "B"), idOf(r, "C"), 1);
  r.moveTabInTree(idOf(r, "C"), idOf(r, "D"), 2);
  expect(names(r)).toBe("A -B --C D");
  return r;
}

describe("tab tree in the pane router", () => {
  it("closing a parent never closes its children: they move up one level, in place", async () => {
    const r = nestedPane();
    await r.closeTab(idOf(r, "B"));
    expect(names(r)).toBe("A -C D");
    await r.closeTab(idOf(r, "A"));
    expect(names(r)).toBe("C D");
    assertTree(r.tabs());
  });

  it("pinning a nested tab lifts it out and promotes its children; unpinning returns a root", async () => {
    const r = nestedPane("tree-pin");
    r.togglePin(idOf(r, "B"));
    expect(names(r)).toBe("B* A -C D");
    r.togglePin(idOf(r, "B"));
    expect(names(r)).toBe("B A -C D");
    assertTree(r.tabs());
  });

  it("the strip's index move carries the subtree and never changes a parent; reorderTab too", () => {
    const r = nestedPane("tree-strip");
    r.moveTabToIndex(idOf(r, "A"), 4);
    expect(names(r)).toBe("D A -B --C");
    // D (a root) dropped onto C (inside A's group): after the group, still a root.
    r.reorderTab(idOf(r, "D"), idOf(r, "C"));
    expect(names(r)).toBe("A -B --C D");
    // C (B's child) dragged to the strip's front: stays B's only child.
    r.moveTabToIndex(idOf(r, "C"), 0);
    expect(names(r)).toBe("A -B --C D");
    // Every strip move, of every tab to every index: no tab's parent changes.
    const parents = () => new Map(r.tabs().map((t) => [t.id, t.parent] as const));
    const before = parents();
    for (let i = 0; i <= 4; i++) for (const name of ["A", "B", "C", "D"]) {
      r.moveTabToIndex(idOf(r, name), i);
      expect(parents()).toEqual(before);
      assertTree(r.tabs());
    }
    assertTree(r.tabs());
  });

  it("a tab leaving for another pane goes alone (its children stay, promoted); an arriving tab is a root", () => {
    const r = nestedPane("tree-src");
    const target = createPaneRouter("tree-dst");
    target.restoreSnapshot(snap([st("X"), st("Y"), st("Z")]));
    target.moveTabInTree(idOf(target, "Y"), idOf(target, "Z"), 1);
    expect(names(target)).toBe("X -Y Z");
    const moved = r.extractTabForAdoption(idOf(r, "B"))!;
    expect(names(r)).toBe("A -C D");
    // Dropped at index 1 (between X and its child Y): lands after X's group, as a root.
    target.adoptTab(moved.tab, true, 1);
    expect(names(target)).toBe("X -Y B Z");
    assertTree(target.tabs());
  });

  it("collapse is kept only on a tab with children; move out of group", () => {
    const r = nestedPane("tree-collapse");
    r.setTabCollapsed(idOf(r, "D"), true);
    expect(names(r)).toBe("A -B --C D");
    r.setTabCollapsed(idOf(r, "B"), true);
    expect(names(r)).toBe("A -B+ --C D");
    r.moveTabOutOfGroup(idOf(r, "C"));
    expect(names(r)).toBe("A -B -C D");
  });

  it("random router operations keep T1/T2 (property-style, seeded)", async () => {
    vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    try {
      const rnd = seeded(1704);
      const r = createPaneRouter("tree-prop");
      const other = createPaneRouter("tree-prop-other");
      const ops = Array(10).fill(0);
      let nestedSteps = 0;
      let collapsedSteps = 0;
      let nestedRoundTrips = 0;
      let n = 0;
      const STEPS = 400;
      for (let step = 0; step < STEPS; step++) {
        const list = r.tabs();
        const pick = () => list[rnd(list.length)].id;
        const op = rnd(10);
        ops[op]++;
        switch (op) {
          case 0: case 1: r.openInNewTab(page(`P${n++}`), rnd(2) === 0); break;
          case 2: if (list.length > 3) await r.closeTab(pick()); break;
          case 3: r.togglePin(pick()); break;
          case 4: {
            const before = new Map(list.map((t) => [t.id, t.parent] as const));
            r.moveTabToIndex(pick(), rnd(list.length + 1));
            expect(new Map(r.tabs().map((t) => [t.id, t.parent] as const))).toEqual(before);
            break;
          }
          case 5: r.moveTabInTree(pick(), rnd(4) ? pick() : null, 1 + rnd(3)); break;
          case 6: r.moveTabOutOfGroup(pick()); break;
          case 7: {
            const parents = list.filter((t) => list.some((c) => c.parent === t.id));
            r.setTabCollapsed(parents.length ? parents[rnd(parents.length)].id : pick(), rnd(3) !== 0);
            break;
          }
          case 8: {
            const moved = r.extractTabForAdoption(pick());
            if (moved) other.adoptTab(moved.tab, false, rnd(other.tabs().length + 1));
            const back = other.tabs().length > 1 ? other.extractTabForAdoption(other.tabs()[rnd(other.tabs().length)].id) : null;
            if (back) r.adoptTab(back.tab, rnd(2) === 0, rnd(r.tabs().length + 1));
            break;
          }
          case 9: {
            const restored = createPaneRouter("tree-prop-restore");
            expect(restored.restoreSnapshot(JSON.parse(JSON.stringify(r.snapshot())))).toBe(true);
            expect(names(restored)).toBe(names(r));
            if (r.tabs().some((t) => t.parent !== null)) nestedRoundTrips++;
            break;
          }
        }
        assertTree(r.tabs());
        assertTree(other.tabs());
        expect(r.tabs().some((t) => t.id === r.activeId())).toBe(true);
        if (r.tabs().some((t) => t.parent !== null)) nestedSteps++;
        if (r.tabs().some((t) => t.collapsed)) collapsedSteps++;
      }
      // Every operation ran, mostly on nested (and often collapsed) trees, and
      // snapshots of nested trees round-tripped: the sequence cannot go flat unseen.
      for (const count of ops) expect(count).toBeGreaterThan(STEPS / 10 / 2);
      expect(nestedSteps).toBeGreaterThan(STEPS / 3);
      expect(collapsedSteps).toBeGreaterThan(STEPS / 10);
      expect(nestedRoundTrips).toBeGreaterThan(10);
      if (process.env.TAB_TREE_STATS) console.log("router", { ops, nestedSteps, collapsedSteps, nestedRoundTrips });
    } finally { vi.restoreAllMocks(); }
  });
});

describe("tab tree in the snapshot", () => {
  it("round-trips nesting and collapse; a flat snapshot carries no tree fields", () => {
    const r = nestedPane("tree-snap");
    r.setTabCollapsed(idOf(r, "B"), true);
    const saved = r.snapshot();
    expect(saved.tabs.map(({ parent, collapsed }) => ({ parent, collapsed }))).toEqual([
      { parent: undefined, collapsed: undefined },
      { parent: 0, collapsed: true },
      { parent: 1, collapsed: undefined },
      { parent: undefined, collapsed: undefined },
    ]);
    const back = createPaneRouter("tree-snap-2");
    back.restoreSnapshot(JSON.parse(JSON.stringify(saved)));
    expect(names(back)).toBe("A -B+ --C D");
    const flat = createPaneRouter("tree-flat");
    flat.restoreSnapshot(snap([st("A"), st("B")]));
    expect(JSON.stringify(flat.snapshot().tabs)).toBe(JSON.stringify([st("A"), st("B")]));
  });

  it("a corrupt parent index restores that tab as a root, never refusing the snapshot", () => {
    const r = createPaneRouter("tree-corrupt");
    expect(r.restoreSnapshot(snap([
      st("A", { parent: 0 }), // self
      st("B", { parent: 7 }), // later / missing
      st("C", { parent: -1 }),
      st("D", { parent: 1.5 }),
      st("E", { parent: "0" as unknown as number }),
      st("F", { pinned: true, parent: 0 }), // pinned is a root
      st("G", { parent: 5, collapsed: true }), // under a pinned tab; collapse with no children
      st("H", { parent: 0 }), // A's subtree ended at B
    ]))).toBe(true);
    expect(names(r)).toBe("F* A B C D E G H");
    assertTree(r.tabs());
  });
});

// Typing guard: the router's Tab is a TreeTab.
const _typing: TreeTab = {} as Tab;
void _typing;
