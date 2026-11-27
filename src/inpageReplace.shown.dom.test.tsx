// Shown text step 2a (REG-FIND-REPLACE-WRITES-ONLY-SHOWN-TEXT): Replace and
// Replace all write a match only where the find pane draws those source bytes as
// themselves. Develop wrote what Find counted, so a page's text icon, an image's
// alt, a callout's lead, a macro's literal and blocks the pane does not draw were
// rewritten unseen. Each case below fails on develop where develop writes.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { For, Show } from "solid-js";
import { backend } from "./backend";
import { loadSingle } from "./document/workingSet";
import { pageByName, undo } from "./document";
import { doc } from "./document/model";
import { closeInPageFind, inPageFindMatches, openInPageFind, setInPageFindQuery } from "./inpageFind";
import { replaceActiveInPageFindMatch, replaceAllInPageFindMatches } from "./inpageReplace";
import { renderedBlocks } from "./lazyObserve";
import { resetPaneLayoutToSingle } from "./panes";
import { setToasts } from "./toasts";
import type { PaneSnapshot } from "./router";
import type { BlockDto } from "./types";
import { Block } from "./components/Block";
import { blk, installBlockEditorLifecycle, mount, page } from "./tests/blockEditorTestkit";

installBlockEditorLifecycle();

const NAME = "Shown";
const snapshot = (name: string): PaneSnapshot => ({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
const settle = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); await new Promise((r) => setTimeout(r, 0)); };

beforeEach(() => {
  setToasts([]);
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  // The referenced page `Target` has the text icon `Target` (`icon:: Target`).
  vi.spyOn(backend(), "pageIcons").mockImplementation(async (names: string[]) =>
    Object.fromEntries(names.filter((n) => n.toLowerCase() === "target").map((n) => [n, "Target"])));
  vi.spyOn(backend(), "getPage").mockResolvedValue(null as never);
  resetPaneLayoutToSingle(snapshot(NAME));
});
afterEach(() => {
  setInPageFindQuery("");
  closeInPageFind({ restoreFocus: false });
  renderedBlocks.clear();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function mountPage(blocks: BlockDto[], format: "md" | "org") {
  loadSingle({ ...page(NAME, blocks), format });
  return mount(() => (
    <div data-pane-id="main">
      <Show when={pageByName(NAME)} keyed>{(p) => <For each={p.roots}>{(id) => <Block id={id} />}</For>}</Show>
    </div>
  ));
}

type Case = { name: string; raw: string; query: string; all: string; one?: string };
const cases = (format: "md" | "org"): Case[] => [
  // The icon draws before the name: the ref's name is never written (a link is
  // renamed by the rename command), the plain word is.
  { name: "a page ref drawn with its text icon", raw: "see [[Target]]", query: "Target", all: "see [[Target]]", one: "see [[Target]]" },
  { name: "a page ref and the same word in prose", raw: "[[Target]] and Target", query: "Target", all: "[[Target]] and X" },
  // Markdown only: an Org `[[url][label]]` draws its label as link text, not as an image.
  ...(format === "md" ? [
    { name: "an image's alt text", raw: "![needle](https://e.org/x.png) needle", query: "needle", all: "![needle](https://e.org/x.png) X", one: "![needle](https://e.org/x.png) needle" },
    // The lead line is drawn as the callout's title, not as its own text.
    { name: "a callout's [!NOTE] lead", raw: "> [!NOTE] note\n> body note", query: "note", all: "> [!NOTE] note\n> body X", one: "> [!NOTE] note\n> body note" },
  ] : []),
  { name: "an embed macro's target", raw: "{{embed [[Foo]]}} Foo", query: "Foo", all: "{{embed [[Foo]]}} X", one: "{{embed [[Foo]]}} Foo" },
  // Positives develop already wrote, kept.
  format === "md"
    ? { name: "prose, an inline code body and a link label", raw: "a `needle` [needle](https://e.org/needle) needle", query: "needle", all: "a `X` [X](https://e.org/needle) X", one: "a `X` [needle](https://e.org/needle) needle" }
    : { name: "prose, a verbatim body and a link label", raw: "a =needle= [[https://e.org/needle][needle]] needle", query: "needle", all: "a =X= [[https://e.org/needle][X]] X", one: "a =X= [[https://e.org/needle][needle]] needle" },
  // The emoji the user searched for is its own source bytes, drawn as a Twemoji image.
  { name: "a Twemoji the user searched for", raw: "a 😄 b", query: "😄", all: "a X b", one: "a X b" },
];

describe.each(["md", "org"] as const)("%s: Replace writes only what the pane draws as itself", (format) => {
  for (const c of cases(format)) {
    for (const mode of ["all", "one"] as const) {
      const expected = mode === "all" ? c.all : c.one;
      if (expected === undefined) continue;
      it(`${c.name}: Replace ${mode === "all" ? "all" : "(active match)"} gives ${JSON.stringify(expected)}`, async () => {
        const { dispose } = mountPage([blk("b", c.raw)], format);
        try {
          await settle();
          openInPageFind();
          setInPageFindQuery(c.query);
          expect(inPageFindMatches().length).toBeGreaterThan(0);
          const outcome = mode === "all" ? replaceAllInPageFindMatches("X") : replaceActiveInPageFindMatch("X");
          expect(doc.byId.b.raw).toBe(expected);
          if (expected === c.raw) {
            expect(outcome.replaced).toBe(0);
            expect(outcome.skipped).toBeGreaterThan(0);
          }
        } finally {
          dispose();
        }
      });
    }
  }
});

describe("blocks the pane does not draw", () => {
  it("Replace all skips a lazy block and a folded child, counts them, and writes the drawn block as one undo step", async () => {
    // No IntersectionObserver callback ever fires: a block not yet rendered stays
    // a deferred placeholder, as one below the fold of a long page does.
    vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} takeRecords() { return []; } });
    renderedBlocks.add("drawn");
    renderedBlocks.add("parent");
    const folded: BlockDto = { id: "parent", raw: "parent", collapsed: true, children: [blk("child", "needle under a fold")] };
    const { root, dispose } = mountPage([blk("drawn", "needle drawn"), blk("lazy", "needle lazy"), folded], "md");
    try {
      await settle();
      expect(root.querySelector('[data-block-id="lazy"] .ast-deferred')).not.toBeNull();
      expect(root.querySelector('[data-block-id="child"]')).toBeNull();
      openInPageFind();
      setInPageFindQuery("needle");
      expect(inPageFindMatches()).toHaveLength(3);
      expect(replaceAllInPageFindMatches("X")).toEqual({ replaced: 1, skipped: 2 });
      expect([doc.byId.drawn.raw, doc.byId.lazy.raw, doc.byId.child.raw]).toEqual(["X drawn", "needle lazy", "needle under a fold"]);
      expect(undo()).toBe(true);
      expect(doc.byId.drawn.raw).toBe("needle drawn");
    } finally {
      dispose();
    }
  });
});
