// GH #794: Set depth N on a block embed. The embed root is level 0: its fold is the
// embedding block's own `collapsed::` (GH #360); rows below it are the source's own
// folds (D-20261030-EMB). Harness shared with Block.embedCollapse.test.tsx.
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { blockProperty, resetStore, undo } from "../document";
import { ensurePageLoaded, loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { BlockDto, PageDto, RefGroup } from "../types";
import { Block, SurfaceContext } from "./Block";
import { endEdit } from "../editorController";
import { maybeEditorIn } from "../tests/editorDriver";
import { installKeybindings, runGlobalCommand } from "../keybindings";
import { bumpDataRev } from "../graphSession";

// GH #360 (Martin-approved embed contract):
//   1. Embedded content is live and source-authoritative: a source fold or
//      unfold updates every visible embed immediately (no page-leave refresh
//      needed).
//   2. Before an occurrence is changed locally it follows the source. Once the
//      occurrence is folded or unfolded, the HOST block owns an explicit
//      `collapsed:: true|false` override. It survives remount/reload without
//      changing the source block or another occurrence.

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

let disposeKeys: () => void;
beforeAll(async () => {
  await initParser();
  disposeKeys = installKeybindings();
});
afterAll(() => disposeKeys());

beforeEach(() => {
  bumpDataRev();
  vi.stubGlobal("IntersectionObserver", AllNearObserver);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  resetStore();
  document.body.innerHTML = "";
});

function page(name: string, blocks: BlockDto[]): PageDto {
  return { name, title: name, kind: "page", pre_block: null, blocks };
}

function mockBlocks(pageDto: PageDto, targets: BlockDto[]): void {
  const groups = new Map<string, RefGroup>(targets.map((block) => [block.id, {
    page: pageDto.name,
    kind: pageDto.kind,
    blocks: [{ ...block, children: [] }],
  }]));
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) =>
    ids.map((id) => groups.get(id) ?? null)
  );
}

function occurrenceHosts(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(".embed-block")];
}

function occurrenceChildrenVisible(host: HTMLElement): string {
  return host.querySelector(".block-children")?.textContent ?? "";
}

async function tick(times = 3): Promise<void> {
  for (let i = 0; i < times; i++) await new Promise((r) => setTimeout(r, 0));
}

describe("Set depth on a block embed (GH #794)", () => {
  it.each(["main", "sidebar:test"])("opens the root, opens a level and folds the next, in %s", async (surface) => {
    const leaf: BlockDto = { id: "leaf", raw: "Leaf", collapsed: false, children: [] };
    const target: BlockDto = { id: "target", raw: "Target", collapsed: false, children: [
      { id: "child", raw: "Child", collapsed: false, children: [
        { id: "grand", raw: "Grand", collapsed: false, children: [leaf] },
      ] },
    ] };
    const source = page("Source", [target]);
    const pg = page("Fixture", [{ id: "host-1", raw: "{{embed ((target))}}", collapsed: false, children: [] }]);
    mockBlocks(source, [target]); loadSingle(pg); ensurePageLoaded(source);
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <SurfaceContext.Provider value={surface}><Block id="host-1" /></SurfaceContext.Provider>, root);
    try {
      await tick();
      const embed = occurrenceHosts(root)[0];
      const content = embed.querySelector<HTMLElement>('[data-block-id="target"] .block-content')!;
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 })); await tick();
      expect(maybeEditorIn(embed)).not.toBeNull();
      runGlobalCommand("editor/set-depth-0"); await tick();
      expect(occurrenceChildrenVisible(embed)).not.toContain("Child");
      expect(blockProperty("host-1", "collapsed")).toBe("true");
      expect(doc.byId.target.collapsed).toBe(false); // the source is not folded by its occurrence
      runGlobalCommand("editor/set-depth-2"); await tick();
      expect(blockProperty("host-1", "collapsed")).toBe("false");
      expect(occurrenceChildrenVisible(embed)).toContain("Grand"); // level 2 shows, folded
      expect(occurrenceChildrenVisible(embed)).not.toContain("Leaf");
      // D-20261030-EMB: below the embed root a fold is the source's own.
      expect(doc.byId.child.collapsed).toBe(false);
      expect(doc.byId.grand.collapsed).toBe(true);
      runGlobalCommand("editor/set-depth-9"); await tick();
      expect(occurrenceChildrenVisible(embed)).toContain("Leaf");
      expect(doc.byId.grand.collapsed).toBe(false);
      undo(); await tick();
      expect(doc.byId.grand.collapsed).toBe(true);
    } finally { endEdit("page-navigation"); dispose(); }
  });
});
