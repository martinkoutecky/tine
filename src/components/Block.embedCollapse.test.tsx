// Family 24 / master af835249f + 3b908751a (GH #360), ported to og's document module.
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { blockProperty, hasSelection, pageByName, resetStore, setCollapsed, undo } from "../document";
import { ensurePageLoaded, loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { BlockDto, PageDto, RefGroup } from "../types";
import { Block, SurfaceContext } from "./Block";
import { endEdit } from "../editorController";
import { installKeybindings, runGlobalCommand } from "../keybindings";
import { For } from "solid-js";
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

// One page: the target (with two visible children) sits alongside TWO host
// blocks, each embedding it — a source + two occurrences of the same block.
function loadEmbedFixture() {
  const target: BlockDto = {
    id: "target",
    raw: "Embed target body",
    collapsed: false,
    children: [
      { id: "target-c1", raw: "child one", collapsed: false, children: [] },
      { id: "target-c2", raw: "child two", collapsed: false, children: [] },
    ],
  };
  const hosts: BlockDto[] = [
    { id: "host-1", raw: "{{embed ((target))}}", collapsed: false, children: [] },
    { id: "host-2", raw: "{{embed ((target))}}", collapsed: false, children: [] },
  ];
  const pg = page("Fixture", [target, ...hosts]);
  mockBlocks(pg, [target]);
  loadSingle(pg);
  return { target, hosts, pg };
}

function mountPageBlocks(): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => (
    <For each={pageByName("Fixture")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  return { root, dispose };
}

function occurrenceHosts(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(".embed-block")];
}

function occurrenceChildrenVisible(host: HTMLElement): string {
  return host.querySelector(".block-children")?.textContent ?? "";
}

function occurrenceToggle(host: HTMLElement): HTMLButtonElement {
  const btn = host.querySelector<HTMLButtonElement>(`.ls-block[data-block-id="target"] .collapse-toggle`);
  if (!btn) throw new Error(`missing collapse toggle inside occurrence: ${host.innerHTML.slice(0, 300)}`);
  return btn;
}

async function tick(times = 3): Promise<void> {
  for (let i = 0; i < times; i++) await new Promise((r) => setTimeout(r, 0));
}

describe("embed collapse contract (GH #360)", () => {
  it.each(["main", "sidebar:test"].flatMap(surface => (["md", "org"] as const).map(format => ({ surface, format }))))("GH #768: one-level commands follow occurrence folds in $surface / $format", async ({ surface, format }) => {
    const target: BlockDto = { id: "target", raw: "Target", collapsed: false, children: [
      { id: "child", raw: "Child", collapsed: false, children: [
        { id: "grand", raw: "Grand", collapsed: false, children: [] },
      ] },
    ] };
    const source = { ...page("Source", [target]), format };
    const pg = { ...page("Fixture", [{ id: "host-1", raw: "{{embed ((target))}}", collapsed: false, children: [] }]), format };
    mockBlocks(source, [target]); loadSingle(pg); ensurePageLoaded(source);
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <SurfaceContext.Provider value={surface}><Block id="host-1" /></SurfaceContext.Provider>, root);
    const run = runGlobalCommand;
    try {
      await tick();
      const embed = occurrenceHosts(root)[0];
      occurrenceToggle(embed).click();
      const content = embed.querySelector<HTMLElement>('[data-block-id="target"] .block-content')!;
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 })); await tick();
      expect(embed.querySelector("textarea")).not.toBeNull();
      run("editor/expand-children"); await tick();
      expect(occurrenceChildrenVisible(embed)).toContain("Child");
      expect(blockProperty("host-1", "collapsed")).toBe("false");
      undo(); await tick();
      expect(blockProperty("host-1", "collapsed")).toBe("true");
      run("editor/expand-children"); await tick();
      run("editor/collapse-children"); await tick();
      expect(embed.textContent).not.toContain("Grand");
      // Selection mode must retain the occurrence after Escape as well.
      embed.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      expect(hasSelection()).toBe(true);
      run("editor/expand-children"); await tick();
      expect(embed.textContent).toContain("Grand");
      run("editor/collapse-children"); run("editor/collapse-children"); await tick();
      expect(occurrenceChildrenVisible(embed)).not.toContain("Child");
      expect(blockProperty("host-1", "collapsed")).toBe("true");
      expect(doc.byId.target.collapsed).toBe(false);
      expect(doc.byId.child.collapsed).toBe(false);
    } finally { endEdit("page-navigation"); dispose(); }
  });

  it.each([false, true].flatMap(embedded => ["main", "sidebar:test"].flatMap(surface =>
    (["md", "org"] as const).map(format => ({ embedded, surface, format })))))
  ("GH #769: child page embeds open and close in $surface / $format / embedded=$embedded", async ({ embedded, surface, format }) => {
    vi.spyOn(backend(), "getPage").mockResolvedValue({ ...page("Source", [
      { id: "source", raw: "Page body", collapsed: false, children: [] },
    ]), id: "pages/Source.md", format });
    const parent: BlockDto = { id: "parent", raw: "Parent", collapsed: false, children: [
      { id: "page-host", raw: "{{embed [[Source]]}}\n" + (format === "org" ? ":PROPERTIES:\n:collapsed: true\n:END:" : "collapsed:: true"), collapsed: true, children: [] },
    ] };
    if (embedded) {
      const source = { ...page("Container", [parent]), format };
      mockBlocks(source, [parent]);
      loadSingle({ ...page("Fixture", [{ id: "host-1", raw: "{{embed ((parent))}}", collapsed: false, children: [] }]), format });
      ensurePageLoaded(source);
    } else loadSingle({ ...page("Fixture", [parent]), format });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <SurfaceContext.Provider value={surface}><Block id={embedded ? "host-1" : "parent"} /></SurfaceContext.Provider>, root);
    try {
      await tick();
      const content = root.querySelector<HTMLElement>('[data-block-id="parent"] .block-content')!;
      content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
      document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 })); await tick();
      runGlobalCommand("editor/expand-children"); await tick();
      expect(root.textContent).toContain("Page body");
      expect(doc.byId["page-host"].collapsed).toBe(embedded);
      runGlobalCommand("editor/collapse-children"); await tick();
      expect(root.textContent).not.toContain("Page body");
      expect(doc.byId["page-host"].collapsed).toBe(true);
      expect(doc.byId.parent.collapsed).toBe(false);
    } finally { endEdit("page-navigation"); dispose(); }
  });

  it("source-authoritative: a source collapse/expand updates ALL visible embeds immediately", async () => {
    loadEmbedFixture();
    const { root, dispose } = mountPageBlocks();
    try {
      const hosts = occurrenceHosts(root);
      expect(hosts).toHaveLength(2);
      await tick();
      expect(occurrenceChildrenVisible(hosts[0])).toContain("child one");
      expect(occurrenceChildrenVisible(hosts[1])).toContain("child one");

      // Collapse the SOURCE block (exactly what the outline bullet toggle does).
      setCollapsed("target", true);
      await tick();
      expect(occurrenceChildrenVisible(hosts[0])).not.toContain("child one");
      expect(occurrenceChildrenVisible(hosts[1])).not.toContain("child one");

      // And expanding again updates them back — no page-leave needed.
      setCollapsed("target", false);
      await tick();
      expect(occurrenceChildrenVisible(hosts[0])).toContain("child one");
      expect(occurrenceChildrenVisible(hosts[1])).toContain("child one");
    } finally {
      dispose();
    }
  });

  it("persists a local fold on occurrence-1's host without touching occurrence-2 or the source", async () => {
    loadEmbedFixture();
    const { root, dispose } = mountPageBlocks();
    try {
      const hosts = occurrenceHosts(root);
      await tick();
      occurrenceToggle(hosts[0]).click();
      await tick();
      expect(occurrenceChildrenVisible(hosts[0])).not.toContain("child one");
      expect(occurrenceChildrenVisible(hosts[1])).toContain("child one");
      expect(doc.byId.target.collapsed).toBe(false);
      expect(doc.byId.target.raw).not.toContain("collapsed::");
      expect(blockProperty("host-1", "collapsed")).toBe("true");
      expect(blockProperty("host-2", "collapsed")).toBeNull();

      // A host-owned choice remains independent when the source moves. The
      // untouched sibling continues to follow the source live.
      setCollapsed("target", true);
      await tick();
      expect(occurrenceChildrenVisible(hosts[0])).not.toContain("child one");
      expect(occurrenceChildrenVisible(hosts[1])).not.toContain("child one");
      setCollapsed("target", false);
      await tick();
      expect(occurrenceChildrenVisible(hosts[0])).not.toContain("child one");
      expect(occurrenceChildrenVisible(hosts[1])).toContain("child one");
    } finally {
      dispose();
    }
  });

  it("a host-owned fold survives an occurrence remount and remains out of the source raw", async () => {
    loadEmbedFixture();
    const first = mountPageBlocks();
    try {
      const hosts = occurrenceHosts(first.root);
      await tick();
      occurrenceToggle(hosts[0]).click();
      await tick();
      expect(occurrenceChildrenVisible(hosts[0])).not.toContain("child one");
    } finally {
      first.dispose();
    }
    document.body.innerHTML = "";
    const second = mountPageBlocks();
    try {
      const hosts = occurrenceHosts(second.root);
      await tick(5);
      expect(occurrenceChildrenVisible(hosts[0])).not.toContain("child one");
      expect(occurrenceChildrenVisible(hosts[1])).toContain("child one");
      expect(doc.byId.target.raw).not.toContain("collapsed::");
      expect(doc.byId.target.collapsed).toBe(false);
      expect(blockProperty("host-1", "collapsed")).toBe("true");
    } finally {
      second.dispose();
    }
  });

  it("persists an explicit false override when one occurrence expands a collapsed source", async () => {
    loadEmbedFixture();
    setCollapsed("target", true); // source starts collapsed
    const { root, dispose } = mountPageBlocks();
    try {
      const hosts = occurrenceHosts(root);
      await tick();
      expect(occurrenceChildrenVisible(hosts[0])).not.toContain("child one");
      // Expand inside occurrence 1 only.
      occurrenceToggle(hosts[0]).click();
      await tick();
      expect(occurrenceChildrenVisible(hosts[0])).toContain("child one");
      expect(occurrenceChildrenVisible(hosts[1])).not.toContain("child one");
      expect(doc.byId.target.collapsed).toBe(true); // source untouched
      expect(blockProperty("host-1", "collapsed")).toBe("false");
      expect(blockProperty("host-2", "collapsed")).toBeNull();
      // The untouched sibling keeps following later source moves, while the
      // explicit false override stays expanded when the source folds again.
      setCollapsed("target", false);
      await tick();
      expect(occurrenceChildrenVisible(hosts[0])).toContain("child one");
      expect(occurrenceChildrenVisible(hosts[1])).toContain("child one");
      setCollapsed("target", true);
      await tick();
      expect(occurrenceChildrenVisible(hosts[0])).toContain("child one");
      expect(occurrenceChildrenVisible(hosts[1])).not.toContain("child one");
    } finally {
      dispose();
    }
  });

  // Nested rows keep an ephemeral per-occurrence fold that records the source's
  // collapse write epoch: it governs until the source writes again, then the
  // source reclaims authority (including after fold/unfold round trips).
  it("a nested row's local fold yields to the next source collapse write", async () => {
    const target: BlockDto = {
      id: "target", raw: "Embed target body", collapsed: false,
      children: [{
        id: "target-c1", raw: "child one", collapsed: false,
        children: [{ id: "target-g1", raw: "grandchild one", collapsed: false, children: [] }],
      }],
    };
    const host: BlockDto = { id: "host-1", raw: "{{embed ((target))}}", collapsed: false, children: [] };
    const pg = page("Fixture", [target, host]);
    mockBlocks(pg, [target]);
    loadSingle(pg);
    const { root, dispose } = mountPageBlocks();
    try {
      const embed = occurrenceHosts(root)[0];
      await tick();
      expect(embed.textContent).toContain("grandchild one");
      embed.querySelector<HTMLButtonElement>(`.ls-block[data-block-id="target-c1"] .collapse-toggle`)!.click();
      await tick();
      expect(embed.textContent).not.toContain("grandchild one"); // local fold governs
      expect(doc.byId["target-c1"].collapsed).toBe(false); // source untouched
      expect(doc.byId["target-c1"].raw).not.toContain("collapsed::");
      setCollapsed("target-c1", true);
      setCollapsed("target-c1", false); // the source wrote again: it is authoritative
      await tick();
      expect(embed.textContent).toContain("grandchild one");
    } finally {
      dispose();
    }
  });
});
