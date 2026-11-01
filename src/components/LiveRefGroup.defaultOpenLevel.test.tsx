// GH #793: `:ref/default-open-blocks-level` (OG state/get-ref-open-blocks-level +
// editor/block-default-collapsed?). References and query results fold a block at
// that level or deeper that has children; level 1 is the displayed hit. The fold
// is the view's own: no file is written. Harness shared with Block.embedHierarchy.test.tsx.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { setGraphMeta } from "../graphSession";
import type { BlockDto, PageDto } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";
import { foldsByDefaultInResults, refDefaultOpenBlocksLevel } from "../refOpenLevel";

beforeAll(async () => { await initParser(); });
afterEach(() => {
  resetStore();
  setGraphMeta(null);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
const leaf = (id: string, raw: string, collapsed = false): BlockDto => ({ id, raw, collapsed, children: [] });
const node = (id: string, raw: string, children: BlockDto[], collapsed = false): BlockDto => ({ id, raw, collapsed, children });

/** hit (level 1) > l2 > l3 > l4 > l5 (a leaf). Rows at levels 2 to 4 have children. */
const chain = (stored: { l3?: boolean } = {}): PageDto => ({
  name: "Source", kind: "page", title: "Source", pre_block: null,
  blocks: [node("hit", "Hit [[Target]]", [
    node("l2", "Level two", [node("l3", "Level three", [node("l4", "Level four", [leaf("l5", "Level five")])], stored.l3 ?? false)]),
  ])],
});
const meta = (level: unknown) => setGraphMeta({ root: "/graphs/A", ref_default_open_blocks_level: level } as never);

async function mount(surface: "ref", page: PageDto = chain()) {
  loadSingle(page);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const writes = vi.spyOn(backend(), "writeRich").mockResolvedValue();
  const dispose = render(() => <LiveRefGroup page={page.name} kind={page.kind} blocks={[{ ...page.blocks[0] }]} surface={surface} />, root);
  await tick(); await tick();
  const shown = (id: string) => root.querySelector(`[data-block-id="${id}"]`) !== null;
  return { root, shown, writes, dispose };
}

describe("the default open level of references (GH #793)", () => {
  it("is OG's 2 without the key: the hit opens and its first child folds", async () => {
    setGraphMeta({ root: "/graphs/A" } as never); // an older host that does not send the key
    const { shown, dispose } = await mount("ref");
    try {
      expect(shown("hit")).toBe(true);
      expect(shown("l2")).toBe(true);
      expect(shown("l3")).toBe(false);
    } finally { dispose(); }
  });

  it.each([
    [2, ["hit", "l2"]],
    [3, ["hit", "l2", "l3"]],
    [4, ["hit", "l2", "l3", "l4"]],
    [5, ["hit", "l2", "l3", "l4", "l5"]],
    [50, ["hit", "l2", "l3", "l4", "l5"]],
    // Level 1 and below fold the hit itself (OG: block level 1 >= 1, with children).
    [1, ["hit"]],
    [0, ["hit"]],
    [-3, ["hit"]],
  ])("level %s shows %j, folding the first block at that level that has children", async (level, expected) => {
    meta(level);
    const { shown, writes, dispose } = await mount("ref");
    try {
      for (const id of ["hit", "l2", "l3", "l4", "l5"]) expect(shown(id), `${id} at level ${level}`).toBe(expected.includes(id));
      expect(writes).not.toHaveBeenCalled();
      for (const id of ["hit", "l2", "l3", "l4"]) {
        expect(doc.byId[id].collapsed, id).toBe(false);
        expect(doc.byId[id].raw, id).not.toContain("collapsed");
      }
    } finally { dispose(); }
  });

  it("uses 2 for a value that is not an integer", async () => {
    for (const bad of [2.5, "5", null, undefined, Number.NaN]) {
      meta(bad);
      expect(refDefaultOpenBlocksLevel(), String(bad)).toBe(2);
    }
  });

  it("keeps a block's own collapsed:: true folded however high the level, as OG does", async () => {
    meta(50);
    const { shown, dispose } = await mount("ref", chain({ l3: true }));
    try {
      expect(shown("l3")).toBe(true);
      expect(shown("l4")).toBe(false);
    } finally { dispose(); }
  });

  it("opening and closing a row by hand still works, on the view's copy only", async () => {
    meta(2);
    const { root, shown, writes, dispose } = await mount("ref");
    try {
      expect(shown("l3")).toBe(false);
      const guide = root.querySelector<HTMLButtonElement>('[data-block-id="hit"] > .block-children-container > button.block-children-left-border')!;
      guide.click(); // the guide expands the whole subtree of this reference
      await tick();
      expect(shown("l3")).toBe(true);
      guide.click();
      await tick();
      expect(shown("l3")).toBe(false);
      expect(writes).not.toHaveBeenCalled();
      expect(doc.byId.l2.raw).not.toContain("collapsed");
    } finally { dispose(); }
  });

  it("pure rule: OG level is depth + 1, leaves never fold", () => {
    meta(3);
    expect(foldsByDefaultInResults(0, true)).toBe(false);
    expect(foldsByDefaultInResults(1, true)).toBe(false);
    expect(foldsByDefaultInResults(2, true)).toBe(true);
    expect(foldsByDefaultInResults(2, false)).toBe(false);
    expect(foldsByDefaultInResults(7, true)).toBe(true);
  });
});
