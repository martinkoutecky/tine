// GH #793, the reporter's goal: "select and copy all the linked references"
// without opening each by hand. `:ref/default-open-blocks-level` controls what
// is shown; GH #941 selections carry the whole subtree at either open level.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { buildClipboardPayload, expandBlockSelection, hasSelection, resetStore, selectedIds } from "../document";
import { endEdit } from "../editorController";
import { editorIn } from "../tests/editorDriver";
import { doc } from "../document/model";
import { setGraphMeta } from "../graphSession";
import { resetReferenceSectionState } from "../referenceSectionState";
import { resetNearObserverForTests } from "../lazyObserve";
import type { BlockDto, PageDto, RefGroup } from "../types";
import { LinkedReferences } from "./LinkedReferences";

class AllNearObserver implements IntersectionObserver {
  readonly root = null;
  readonly rootMargin = "0px";
  readonly thresholds = [0];
  constructor(private readonly callback: IntersectionObserverCallback) {}
  disconnect(): void {}
  takeRecords(): IntersectionObserverEntry[] { return []; }
  unobserve(): void {}
  observe(target: Element): void {
    this.callback([{ isIntersecting: true, target } as IntersectionObserverEntry], this);
  }
}

beforeAll(async () => { await initParser(); });
afterEach(() => {
  resetNearObserverForTests();
  resetReferenceSectionState();
  resetStore();
  setGraphMeta(null);
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
const node = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });
const pageOf = (name: string, tag: string): PageDto => ({
  name, kind: "page", title: name, pre_block: null,
  blocks: [node(`${tag}-hit`, `${tag} hit [[Target]]`, [
    node(`${tag}-c1`, `${tag} child one`, [node(`${tag}-g1`, `${tag} grandchild`, [node(`${tag}-d1`, `${tag} deep`)])]),
  ])],
});

async function mountReferences(level: number | undefined) {
  vi.stubGlobal("IntersectionObserver", AllNearObserver);
  setGraphMeta({ root: "/graphs/A", ref_default_open_blocks_level: level } as never);
  const pages = [pageOf("Alpha", "a"), pageOf("Beta", "b")];
  const groups: RefGroup[] = pages.map((p) => ({ page: p.name, kind: p.kind, blocks: [{ ...p.blocks[0], children: [] }] }));
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
  vi.spyOn(backend(), "getPage").mockImplementation(async (name) => ({ ...pages.find((p) => p.name === name)!, id: `pages/${name}.md`, format: "md" }) as never);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <LinkedReferences name="Target" />, root);
  await expect.poll(() => ["a-hit", "b-hit"].every(id =>
    root.querySelector(`.ls-block:not(.ref-block)[data-block-id="${id}"] > .block-main`))).toBe(true);
  return { root, dispose };
}
const shownIds = (root: HTMLElement) => [...root.querySelectorAll<HTMLElement>("[data-block-id]")].map((el) => el.dataset.blockId!);

/** The user's way in: click into a reference, Escape to block-select it, then widen with Mod+A. */
async function selectWholeGroup(root: HTMLElement, hit: string): Promise<string[]> {
  const content = root.querySelector<HTMLElement>(`[data-block-id="${hit}"] > .block-main .block-content`)!;
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  await tick();
  editorIn(root).keydown({ key: "Escape" });
  expect(hasSelection()).toBe(true);
  for (let i = 0; i < 4; i++) expandBlockSelection();
  return selectedIds();
}
const copied = (ids: string[]) => JSON.stringify(buildClipboardPayload(ids)?.blocks);

describe("linked references at a high default open level (GH #793)", () => {
  it("opens every reference row, so Mod+A selects them all and copy takes every line", async () => {
    const { root, dispose } = await mountReferences(10);
    try {
      expect(shownIds(root).sort()).toEqual(["a-c1", "a-d1", "a-g1", "a-hit", "b-c1", "b-d1", "b-g1", "b-hit"]);
      const ids = await selectWholeGroup(root, "a-hit");
      expect(ids).toEqual(["a-hit", "a-c1", "a-g1", "a-d1"]);
      for (const line of ["a hit", "a child one", "a grandchild", "a deep"]) expect(copied(ids), line).toContain(line);
    } finally { endEdit("page-navigation"); dispose(); }
  });

  it("at OG's default 2 deeper rows stay hidden while selecting their parent carries them (GH #941)", async () => {
    const { root, dispose } = await mountReferences(undefined);
    try {
      expect(shownIds(root).sort()).toEqual(["a-c1", "a-hit", "b-c1", "b-hit"]);
      const ids = await selectWholeGroup(root, "a-hit");
      expect(ids).toEqual(["a-hit", "a-c1", "a-g1", "a-d1"]);
      for (const line of ["a hit", "a child one", "a grandchild", "a deep"]) expect(copied(ids), line).toContain(line);
      expect(doc.byId["a-c1"].collapsed).toBe(false); // folded in the view only
    } finally { endEdit("page-navigation"); dispose(); }
  });
});
