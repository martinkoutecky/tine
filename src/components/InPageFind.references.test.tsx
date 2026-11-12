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
import { setGraphMeta } from "../graphSession";

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
  closeInPageFind({ restoreFocus: false }); setInPageFindQuery("");
  setMounted([]);
  setGraphMeta(null);
  resetStore(); resetReferenceSectionState();
  document.body.innerHTML = "";
  localStorage.clear();
  vi.restoreAllMocks();
});

async function setup(unlinked = false, opts: { open?: boolean; groups?: RefGroup[] } = {}) {
  setDoc({
    loaded: true, feed: ["Target"],
    pages: [{ name: "Target", kind: "page", title: "Target", preBlock: null, roots: ["body"], format: "md", readOnly: false, guide: false }],
    byId: { body: { id: "body", raw: "needle on the page itself", collapsed: false, parent: null, page: "Target", children: [] } },
  });
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: "Target", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue(opts.groups ?? groups);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue(opts.groups ?? groups);
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
  if (unlinked && opts.open !== false) { host.querySelector<HTMLElement>(".references-header")!.click(); await tick(); await tick(); }
  return { host, dispose };
}

// The hand-off reads what is typed in the find box, so type there like a user.
async function typeFind(host: HTMLElement, text: string) {
  openInPageFind(); await tick();
  const input = host.querySelector<HTMLInputElement>(".inpage-find-input")!;
  input.value = text; input.dispatchEvent(new InputEvent("input", { bubbles: true }));
  setInPageFindQuery(text);
  await tick(); await tick();
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

  it("still names the panel and offers the search while it is folded, by hand or automatically", async () => {
    for (const how of ["by hand", "at the threshold"]) {
      const { host, dispose } = await setup();
      try {
        if (how === "by hand") host.querySelector<HTMLElement>(".references-header")!.click();
        else setGraphMeta({ linked_references_collapsed_threshold: 0 } as never);
        await tick();
        // Folded: no group is mounted, so nothing below the header can announce the panel.
        expect(host.querySelectorAll(".reference-group")).toHaveLength(0);
        await typeFind(host, "needle in Gamma");
        expect(host.querySelector(".inpage-find-count")?.textContent).toBe("No results");
        expect(host.querySelector(".inpage-find-retired")?.textContent).toContain("Linked references not searched");
        host.querySelector<HTMLButtonElement>(".inpage-find-retired button")!.click();
        await tick(); await tick();
        expect(host.querySelector<HTMLInputElement>(".reference-filter-search")?.value).toBe("needle in Gamma");
        expect(host.querySelectorAll(".reference-group")).toHaveLength(1);
      } finally { dispose(); document.body.innerHTML = ""; closeInPageFind({ restoreFocus: false }); setInPageFindQuery(""); resetStore(); resetReferenceSectionState(); setGraphMeta(null); localStorage.clear(); }
    }
  });

  it("offers the search on an Unlinked panel that has not been opened, and not on a known-empty one", async () => {
    const { host, dispose } = await setup(true, { open: false });
    try {
      await typeFind(host, "needle");
      expect(host.querySelector(".inpage-find-retired")?.textContent).toContain("Unlinked references not searched");
      host.querySelector<HTMLButtonElement>(".inpage-find-retired button")!.click();
      await tick(); await tick();
      expect(host.querySelector<HTMLInputElement>(".reference-filter-search")?.value).toBe("needle");
      expect(host.querySelectorAll(".reference-group")).toHaveLength(3);
    } finally { dispose(); }
    document.body.innerHTML = ""; closeInPageFind({ restoreFocus: false }); setInPageFindQuery(""); resetStore(); resetReferenceSectionState(); localStorage.clear();
    const empty = await setup(true, { groups: [] });
    try {
      await typeFind(empty.host, "needle");
      expect(empty.host.querySelector(".inpage-find-retired")).toBeNull();
    } finally { empty.dispose(); }
  });

  it("hands over Find's text literally; the panel grammar returns when the box is edited", async () => {
    const literal: RefGroup[] = [
      { page: "Alpha", kind: "page", blocks: [block("a", "[[Target]] foo -draft kept")] },
      { page: "Beta", kind: "page", blocks: [block("b", "[[Target]] foo draft other")] },
      { page: "Gamma", kind: "page", blocks: [block("c", "[[Target]] foo ready")] },
    ];
    const { host, dispose } = await setup(false, { groups: literal });
    try {
      await typeFind(host, "foo -draft");
      host.querySelector<HTMLButtonElement>(".inpage-find-retired button")!.click();
      await tick(); await tick();
      const search = host.querySelector<HTMLInputElement>(".reference-filter-search")!;
      expect(search.value).toBe("foo -draft");
      // Literal: only the block that contains the characters `foo -draft`.
      expect(host.querySelectorAll(".reference-group")).toHaveLength(1);
      expect(host.querySelector(".reference-group")?.textContent).toContain("Alpha");
      expect(host.querySelector(".reference-filter-summary")?.textContent).toContain("1");
      expect(host.querySelector(".reference-filter-error")).toBeNull();
      // Editing the box restores the ordinary grammar: `-draft` excludes, so the ready block remains only.
      search.value = "foo -draft "; search.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await new Promise(resolve => setTimeout(resolve, 200));
      expect(host.querySelectorAll(".reference-group")).toHaveLength(1);
      expect(host.querySelector(".reference-group")?.textContent).toContain("Gamma");
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
