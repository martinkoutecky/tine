import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { initParser } from "./render/parse";
import { pageByName, resetStore, undo } from "./document";
import { doc } from "./document/model";
import { loadSingle } from "./document/workingSet";
import { openPageAtBlock } from "./router";
import type { BlockDto, PageDto } from "./types";
import { Block } from "./components/Block";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.useRealTimers();
  resetStore();
  document.body.innerHTML = "";
});

function mount(name: string): () => void {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return render(
    () => <For each={pageByName(name)?.roots ?? []}>{(bid) => <Block id={bid} />}</For>,
    root
  );
}

/** parent (collapsed) → child → grandchild. The grandchild is what a Ctrl+Shift+K
 *  block result points at, and it is two collapsed levels down. */
function nestedPage(): PageDto {
  const grandchild: BlockDto = { id: "gc", raw: "needle in a haystack", collapsed: false, children: [] };
  const child: BlockDto = { id: "kid", raw: "child", collapsed: true, children: [grandchild] };
  const parent: BlockDto = { id: "root", raw: "parent", collapsed: true, children: [child] };
  return { name: "Nested", kind: "page", title: "Nested", pre_block: null, blocks: [parent] };
}

// GH #258: navigating to a block nested inside collapsed parents must reveal it.
// Before the fix, `openPageAtBlock` polled the DOM for the target element, but a
// collapsed parent renders no children at all, so the poll always timed out and
// the user landed on the page with nothing scrolled to or highlighted.
describe("openPageAtBlock reveals a block inside collapsed ancestors", () => {
  it("expands every collapsed ancestor so the target can render", async () => {
    loadSingle(nestedPage());
    const dispose = mount("Nested");
    try {
      expect(doc.byId["root"].collapsed).toBe(true);
      expect(doc.byId["kid"].collapsed).toBe(true);
      // Necessity: while collapsed, the target is genuinely absent from the DOM.
      expect(document.querySelector('.ls-block[data-block-id="gc"]')).toBeNull();

      openPageAtBlock("Nested", "page", "gc");
      await vi.waitFor(() => {
        expect(doc.byId["root"].collapsed).toBe(false);
        expect(doc.byId["kid"].collapsed).toBe(false);
      });
      await vi.waitFor(() => {
        expect(document.querySelector('.ls-block[data-block-id="gc"]')).not.toBeNull();
      });
    } finally {
      dispose();
    }
  });

  it("persists the reveal into collapsed:: and undoes as ONE step", async () => {
    // Expanding is on-disk state; leaving it unsaved would revert under the user
    // on the next load. One Ctrl+Z must put the whole chain back.
    loadSingle(nestedPage());
    const dispose = mount("Nested");
    try {
      openPageAtBlock("Nested", "page", "gc");
      await vi.waitFor(() => expect(doc.byId["kid"].collapsed).toBe(false));
      expect(doc.byId["root"].raw).not.toContain("collapsed:: true");
      expect(doc.byId["kid"].raw).not.toContain("collapsed:: true");

      undo();
      expect(doc.byId["root"].collapsed).toBe(true);
      expect(doc.byId["kid"].collapsed).toBe(true);
    } finally {
      dispose();
    }
  });

  it("leaves an already-visible target alone", async () => {
    // Necessity guard: navigation must not expand things the user had collapsed
    // for unrelated reasons — only the target's own ancestor chain.
    const sibling: BlockDto = { id: "sib", raw: "sibling", collapsed: true, children: [
      { id: "sib-kid", raw: "hidden", collapsed: false, children: [] },
    ] };
    const visible: BlockDto = { id: "vis", raw: "visible target", collapsed: false, children: [] };
    loadSingle({ name: "Flat", kind: "page", title: "Flat", pre_block: null, blocks: [sibling, visible] });
    const dispose = mount("Flat");
    try {
      openPageAtBlock("Flat", "page", "vis");
      await vi.waitFor(() => {
        expect(document.querySelector('.ls-block[data-block-id="vis"]')).not.toBeNull();
      });
      expect(doc.byId["sib"].collapsed).toBe(true);
    } finally {
      dispose();
    }
  });
});
