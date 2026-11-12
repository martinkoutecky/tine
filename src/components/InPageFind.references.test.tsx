// GH #935: Find does not count reference panels, whose groups mount as the reader scrolls.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { For, createSignal } from "solid-js";
import { backend } from "../backend";
import type { BlockDto, RefGroup } from "../types";
import { InPageFind } from "./InPageFind";
import { LinkedReferences, ReferenceSection } from "./LinkedReferences";
import { initParser } from "../render/parse";
import { closeInPageFind, inPageFindMatches, openInPageFind, setInPageFindQuery } from "../inpageFind";
import { resetPaneLayoutToSingle } from "../panes";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import { resetReferenceSectionState } from "../referenceSectionState";

// A group's rows exist only after it "scrolls near": the real LiveRefGroup mounts them lazily.
const [mounted, setMounted] = createSignal<string[]>([]);
vi.mock("./LiveRefGroup", () => ({
  LiveRefGroup: (props: { page: string; blocks: BlockDto[] }) => (
    <div class="live-ref-group">
      <For each={mounted().includes(props.page) ? props.blocks : []}>{block =>
        <div class="ls-block" data-block-id={`${props.page}:${block.id}`}><div class="block-content">{block.raw}</div></div>}</For>
    </div>
  ),
}));

const tick = () => new Promise<void>(resolve => setTimeout(resolve, 0));
const block = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const groups: RefGroup[] = ["Alpha", "Beta", "Gamma", "Delta"].map(page => ({
  page, kind: "page", blocks: [block("r", page === "Delta" ? "[[Target]] nothing here" : `[[Target]] needle in ${page}`)] }));

beforeAll(async () => { await initParser(); Range.prototype.getClientRects = vi.fn(() => [] as unknown as DOMRectList); });
afterEach(() => {
  closeInPageFind({ restoreFocus: false });
  setMounted([]);
  resetStore(); resetReferenceSectionState();
  document.body.innerHTML = "";
  localStorage.clear();
  vi.restoreAllMocks();
});

async function setup(unlinked = false) {
  setDoc({
    loaded: true, feed: ["Target"],
    pages: [{ name: "Target", kind: "page", title: "Target", preBlock: null, roots: ["body"], format: "md", readOnly: false, guide: false }],
    byId: { body: { id: "body", raw: "needle on the page itself", collapsed: false, parent: null, page: "Target", children: [] } },
  });
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: "Target", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue(groups);
  vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue({ entries: [], truncated: false } as never);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <>
    <div data-pane-id="main">
      <div class="ls-block" data-block-id="body"><div class="block-content">needle on the page itself</div></div>
      {unlinked ? <ReferenceSection name="Target" unlinked /> : <LinkedReferences name="Target" />}
    </div>
    <InPageFind />
  </>, host);
  await tick(); await tick();
  if (unlinked) { host.querySelector<HTMLElement>(".references-header")!.click(); await tick(); await tick(); }
  return { host, dispose };
}

describe("Find and reference panels (GH #935)", () => {
  it.each([false, true])("keeps the count fixed while groups mount on scroll, names the left-out panel (unlinked=%s)", async (unlinked) => {
    const { host, dispose } = await setup(unlinked);
    try {
      openInPageFind(); setInPageFindQuery("needle"); await tick();
      const kind = unlinked ? "Unlinked" : "Linked";
      expect(host.querySelector(".inpage-find-count")?.textContent).toBe("1 / 1");
      expect(host.querySelector(".inpage-find-retired")?.textContent).toContain(`${kind} references not searched`);
      for (const page of ["Alpha", "Beta", "Gamma"]) {
        setMounted(list => [...list, page]); await tick();
        expect(host.querySelectorAll(".reference-group .ls-block").length).toBeGreaterThan(0);
        expect(inPageFindMatches().map(match => match.blockId)).toEqual(["body"]);
        expect(host.querySelector(".inpage-find-count")?.textContent).toBe("1 / 1");
      }
    } finally { dispose(); }
  });

  it("hands the query to the section's own search, which covers groups that never mounted", async () => {
    const { host, dispose } = await setup();
    try {
      openInPageFind(); setInPageFindQuery("needle"); await tick();
      const input = host.querySelector<HTMLInputElement>(".inpage-find-input")!;
      input.value = "needle in"; input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      expect(host.querySelectorAll(".reference-group")).toHaveLength(4);
      expect(host.querySelector(".reference-filter-panel")).toBeNull();
      host.querySelector<HTMLButtonElement>(".inpage-find-retired button")!.click();
      await tick();
      expect(host.querySelector(".inpage-find-bar")).toBeNull();
      const search = host.querySelector<HTMLInputElement>(".reference-filter-search")!;
      expect(search.value).toBe("needle in");
      // No group was ever mounted, yet the filter narrowed all four to the three that contain the words.
      expect(host.querySelectorAll(".reference-group")).toHaveLength(3);
      expect(host.querySelector(".reference-filter-summary")?.textContent).toContain("3");
      expect(document.activeElement).toBe(search);
    } finally { dispose(); }
  });

  it("says nothing without a query", async () => {
    const { host, dispose } = await setup();
    try {
      openInPageFind(); setInPageFindQuery(""); await tick();
      expect(host.querySelector(".inpage-find-retired")).toBeNull();
      setInPageFindQuery("needle"); await tick();
      expect(host.querySelector(".inpage-find-retired")).not.toBeNull();
      setInPageFindQuery(""); await tick();
      expect(host.querySelector(".inpage-find-retired")).toBeNull();
    } finally { dispose(); }
  });
});

describe("Find and block reference panels (GH #935)", () => {
  it("never counts a panel's rows and names it without an action", async () => {
    setDoc({
      loaded: true, feed: ["Target"],
      pages: [{ name: "Target", kind: "page", title: "Target", preBlock: null, roots: ["body"], format: "md", readOnly: false, guide: false }],
      byId: { body: { id: "body", raw: "needle", collapsed: false, parent: null, page: "Target", children: [] } },
    });
    resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: "Target", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
    const host = document.createElement("div"); document.body.append(host);
    const dispose = render(() => <>
      <div data-pane-id="main">
        <div class="ls-block" data-block-id="body"><div class="block-content">needle</div></div>
        <div class="block-references-inner" data-inpage-find-references="block" data-inpage-find-references-page="body">
          <div class="reference-group"><div class="ls-block" data-block-id="other:r"><div class="block-content">needle</div></div></div>
        </div>
      </div>
      <InPageFind />
    </>, host);
    try {
      openInPageFind(); setInPageFindQuery("needle"); await tick();
      expect(host.querySelector(".inpage-find-count")?.textContent).toBe("1 / 1");
      expect(host.querySelector(".inpage-find-retired")?.textContent).toBe("Block references not searched");
      expect(host.querySelector(".inpage-find-retired button")).toBeNull();
    } finally { dispose(); }
  });
});
