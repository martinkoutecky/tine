// With typographic glyphs drawn, Find's index counts the source text and its highlighter counts the page's text, so
// the two named different occurrences and Replace wrote one that was not highlighted (`a -- b `literal -- end``:
// the code span's dash highlighted, the prose dash replaced). While Find is open no glyphs are drawn, so the
// highlighted occurrence is the one Replace writes. UI-CM6-TYPOGRAPHY-WYSIWYG.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { For, Show } from "solid-js";
import { loadSingle } from "./document/workingSet";
import { pageByName } from "./document";
import { doc } from "./document/model";
import { activateInPageFindIndex, closeInPageFind, glyphsShown, inPageFindMatches, openInPageFind, setInPageFindQuery } from "./inpageFind";
import { replaceActiveInPageFindMatch } from "./inpageReplace";
import { resetPaneLayoutToSingle } from "./panes";
import { setTypographyModeSig, typographyMode } from "./ui";
import type { PaneSnapshot } from "./router";
import { Block } from "./components/Block";
import { blk, installBlockEditorLifecycle, mount, page } from "./tests/blockEditorTestkit";

installBlockEditorLifecycle();

const NAME = "Typo";
const snapshot = (name: string): PaneSnapshot => ({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
const TRIGGERS = ["<-->", "<->", "<--", "-->", "<-", "->", "=>", "---", "--"];
const initial = typographyMode();
let registry: Map<string, { ranges: Range[] }>;

beforeEach(() => {
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  // jsdom has no CSS Custom Highlight API: record the ranges Find paints.
  registry = new Map();
  vi.stubGlobal("CSS", { highlights: registry });
  vi.stubGlobal("Highlight", class { ranges: Range[]; constructor(...ranges: Range[]) { this.ranges = ranges; } });
  resetPaneLayoutToSingle(snapshot(NAME));
});
afterEach(() => {
  setInPageFindQuery("");
  closeInPageFind({ restoreFocus: false });
  setTypographyModeSig(initial);
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function mountPage(raw: string, format: "md" | "org") {
  loadSingle({ ...page(NAME, [blk("b", raw)]), format });
  return mount(() => (
    <div data-pane-id="main">
      <Show when={pageByName(NAME)} keyed>{(p) => <For each={p.roots}>{(id) => <Block id={id} />}</For>}</Show>
    </div>
  ));
}

const content = (root: HTMLElement) => root.querySelector<HTMLElement>(".block-content")!;
/** The text the page shows for the block, and where in it the active highlight is. */
function highlighted(root: HTMLElement): { text: string; start: number; end: number } {
  const active = registry.get("tine-find-active")?.ranges[0];
  expect(active, "an active highlight is painted").toBeDefined();
  const el = content(root);
  const before = document.createRange();
  before.setStart(el, 0);
  before.setEnd(active!.startContainer, active!.startOffset);
  const start = before.toString().length;
  return { text: el.textContent!, start, end: start + active!.toString().length };
}

describe("Find and Replace with typographic glyphs on", () => {
  it("draws no glyphs while Find is open, and draws them again when it closes", () => {
    expect(initial).toBe("render");
    const { root, dispose } = mountPage("a -- b -> c", "md");
    try {
      expect(content(root).textContent).toBe("a – b → c");
      openInPageFind();
      expect(glyphsShown()).toBe(false);
      expect(content(root).textContent).toBe("a -- b -> c");
      closeInPageFind({ restoreFocus: false });
      expect(content(root).textContent).toBe("a – b → c");
    } finally {
      dispose();
    }
  });

  for (const format of ["md", "org"] as const) {
    for (const t of TRIGGERS) {
      const raws = format === "md"
        ? [`a ${t} b \`literal ${t} end\``]
        // Org `a => b ~x => y~` parses `=> b ~x =` as verbatim, so the page shows no `=>` there: code first instead.
        : t === "=>" ? [`~literal ${t} end~ a ${t} b`, `a ${t} b\n~literal ${t} end~`]
        : [`a ${t} b ~literal ${t} end~`, `a ${t} b =literal ${t} end=`];
      for (const raw of raws) {
        it(`${format} ${JSON.stringify(raw)}: Replace writes the occurrence Find highlights`, async () => {
          const { root, dispose } = mountPage(raw, format);
          try {
            openInPageFind();
            setInPageFindQuery(t);
            const count = inPageFindMatches().length;
            expect(count).toBeGreaterThan(0);
            for (let n = count - 1; n >= 0; n--) {
              registry.clear();
              activateInPageFindIndex(n);
              await vi.waitFor(() => expect(registry.get("tine-find-active")).toBeDefined());
              const shown = highlighted(root);
              expect(shown.text.slice(shown.start, shown.end)).toBe(t);
              expect(replaceActiveInPageFindMatch("X").replaced).toBe(1);
              const expected = shown.text.slice(0, shown.start) + "X" + shown.text.slice(shown.end);
              await vi.waitFor(() => expect(content(root).textContent).toBe(expected));
              setInPageFindQuery(t);
            }
            expect(doc.byId.b.raw).not.toContain(t);
          } finally {
            dispose();
          }
        });
      }
    }
  }
});
