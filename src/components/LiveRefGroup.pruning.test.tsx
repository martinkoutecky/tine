import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { loadSingle, resetStore, setDoc } from "../store";
import type { BlockDto, PageDto } from "../types";
import { LiveRefGroup, __livRefGroupInternals } from "./LiveRefGroup";

// GH #185: LiveRefGroup garbage-collects its local collapse-state maps with a
// DFS over every descendant of every result root. Keeping the walk's
// doc.byId[...].children reads reactive subscribed this effect to the ENTIRE
// reference subtree, so any structural edit anywhere re-ran the whole O(subtree)
// walk. The GC only needs to run when result-root membership changes (a query
// refresh) — the one moment a stale collapse choice could leak into a new
// membership. This test pins that bound: the walk fires on membership change but
// NOT on an unrelated descendant edit, while collapse behavior is unchanged.

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(node, host);
  return { host, dispose };
}

// A single hit root with a live child on its source page.
function source(): { page: PageDto; result: BlockDto } {
  const child: BlockDto = { id: "c", raw: "Child body", collapsed: false, children: [] };
  const root: BlockDto = { id: "r", raw: "Root [[Target]]", collapsed: false, children: [child] };
  return {
    page: { name: "Source", title: "Source", kind: "page", pre_block: null, blocks: [root] },
    // Membership carries only the hit root; the live subtree comes from the page.
    result: { ...root, children: [] },
  };
}

describe("LiveRefGroup collapse-state pruning bound (GH #185)", () => {
  it("runs the collapse GC on membership change but not on an unrelated descendant edit", async () => {
    const { page, result } = source();
    loadSingle(page);
    __livRefGroupInternals.pruneRuns = 0;

    const [membership, setMembership] = createSignal<BlockDto[]>([result]);
    const { host, dispose } = mount(() => (
      <LiveRefGroup page={page.name} kind={page.kind} blocks={membership()} surface="ref" />
    ));

    try {
      // Hydrated from the source page and pruned at least once.
      await expect.poll(() => host.textContent).toContain("Child body");
      await expect.poll(() => __livRefGroupInternals.pruneRuns).toBeGreaterThan(0);
      const afterMount = __livRefGroupInternals.pruneRuns;

      // A structural edit inside the subtree (a new second child under the hit
      // root, which is rendered) must NOT re-trigger the O(subtree) GC walk.
      // The new bullet rendering is a deterministic signal that the reactive
      // update flushed, so the counter assertion is not racing an unflushed edit.
      setDoc("byId", "c2", {
        id: "c2",
        raw: "Second child body",
        collapsed: false,
        parent: "r",
        page: page.name,
        children: [],
      });
      setDoc("byId", "r", "children", ["c", "c2"]);
      await expect.poll(() => host.textContent).toContain("Second child body");
      expect(__livRefGroupInternals.pruneRuns).toBe(afterMount);

      // A result-membership change (query refresh) MUST still run the GC walk —
      // this is the moment a stale collapse choice could leak into new membership.
      setMembership([{ ...result }]);
      await expect.poll(() => __livRefGroupInternals.pruneRuns).toBeGreaterThan(afterMount);
    } finally {
      dispose();
    }
  });
});
