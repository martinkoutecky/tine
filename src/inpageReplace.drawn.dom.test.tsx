// Shown text step 2a (REG-FIND-REPLACE-WRITES-ONLY-SHOWN-TEXT): Replace and
// Replace all write a match if and only if its source bytes lie inside one
// mapped segment of one stamped leaf the pane draws for that block, under a
// drawing root that drew the block's current text. Every other match Find holds
// is skipped and counted, and the find bar says how many.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { For, Show } from "solid-js";
import { backend } from "./backend";
import { loadSingle, reloadPage } from "./document/workingSet";
import { pageByName, resetStore } from "./document";
import { doc, setDoc, type FeedPage, type Node } from "./document/model";
import { startEditing } from "./editorController";
import { activateInPageFindIndex, closeInPageFind, inPageFindMatches, openInPageFind, setInPageFindQuery } from "./inpageFind";
import { inPageReplaceSkippedHere, replaceActiveInPageFindMatch, replaceAllInPageFindMatches, type InPageReplaceOutcome } from "./inpageReplace";
import { renderedBlocks } from "./lazyObserve";
import { adoptResults, captureResultsTicket } from "./queryResultsEpoch";
import { resetPaneLayoutToSingle } from "./panes";
import { resetCellSelectionForTests } from "./sheet/selection";
import { setToasts } from "./toasts";
import type { PaneSnapshot } from "./router";
import type { BlockDto } from "./types";
import { Block } from "./components/Block";
import { InPageFind } from "./components/InPageFind";
import { LiveRefGroup } from "./components/LiveRefGroup";
import { blk, installBlockEditorLifecycle, mount, page } from "./tests/blockEditorTestkit";

installBlockEditorLifecycle();

const NAME = "Drawn";
const snapshot = (name: string): PaneSnapshot => ({ tabs: [{ history: [{ kind: "page", name, pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
const settle = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); await new Promise((r) => setTimeout(r, 0)); };
const X_ID = "6501f00a-0000-4000-8000-0000000000aa";
const B_ID = "6501f00a-0000-4000-8000-0000000000bb";

beforeEach(() => {
  setToasts([]);
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  vi.spyOn(backend(), "pageIcons").mockImplementation(async (names: string[]) =>
    Object.fromEntries(names.filter((n) => n.toLowerCase() === "foobar").map((n) => [n, "Foobar"])));
  resetPaneLayoutToSingle(snapshot(NAME));
});
afterEach(() => {
  setInPageFindQuery("");
  closeInPageFind({ restoreFocus: false });
  renderedBlocks.clear();
  resetCellSelectionForTests();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

/** The page as a working-set install; sheets are installed as the sheet suites
 *  install them (`setDoc`), since only that draws their cells here. */
function install(blocks: BlockDto[], format: "md" | "org", sheet: boolean) {
  resetStore();
  if (!sheet) { loadSingle({ ...page(NAME, blocks), format }); return; }
  const byId: Record<string, Node> = {};
  const add = (b: BlockDto, parent: string | null) => {
    byId[b.id] = { id: b.id, raw: b.raw, collapsed: b.collapsed, parent, page: NAME, children: b.children.map((c) => c.id) };
    b.children.forEach((c) => add(c, b.id));
  };
  blocks.forEach((b) => add(b, null));
  setDoc({ byId, pages: [{ name: NAME, kind: "page", title: NAME, preBlock: null, roots: blocks.map((b) => b.id), format, readOnly: false, guide: false } as FeedPage], feed: [NAME], loaded: true });
}

function mountPage(blocks: BlockDto[], format: "md" | "org" = "md", sheet = false) {
  install(blocks, format, sheet);
  return mount(() => (
    <>
      <div data-pane-id="main">
        <Show when={pageByName(NAME)} keyed>{(p) => <For each={p.roots}>{(id) => <Block id={id} />}</For>}</Show>
      </div>
      <InPageFind />
    </>
  ));
}

/** The find bar's skipped line, or null. */
const skippedLine = (root: ParentNode) =>
  [...root.querySelectorAll("[role=status]")].map((e) => e.textContent ?? "").find((t) => t.includes("replaced here")) ?? null;

async function find(query: string) {
  openInPageFind();
  setInPageFindQuery(query);
  await settle();
  return inPageFindMatches().length;
}

/** Press Replace once per match Find held, as a user walking the list does. */
function walkReplace(found: number): InPageReplaceOutcome {
  activateInPageFindIndex(0);
  let replaced = 0, skipped = 0;
  for (let i = 0; i < found; i++) {
    const o = replaceActiveInPageFindMatch("X");
    replaced += o.replaced; skipped += o.skipped;
  }
  return { replaced, skipped };
}

const ID_MD = (id: string) => `\nid:: ${id}`;
const ID_ORG = (id: string) => `\n:PROPERTIES:\n:id: ${id}\n:END:`;

describe.each(["md", "org"] as const)("%s: what the pane draws as something else is skipped and counted", (format) => {
  // [raw, query, after Replace all]
  const cases: [string, string, string][] = [
    ["{{video https://example.com/Foo}} Foo", "Foo", "{{video https://example.com/Foo}} X"],
    [`((${B_ID})) e`, "e", `((${B_ID})) X`],
    ["$e$ e", "e", "$e$ X"],
    ["[[Foo]] x", "[[", "[[Foo]] x"],
    // A query's tail and a #+BEGIN_QUERY body are the query, not text.
    ['{{query (and "x }} needle")}}', "needle", '{{query (and "x }} needle")}}'],
    ["#+BEGIN_QUERY\n{:query [:find (pull ?b [*]) :where [?b :block/content \"needle\"]]}\n#+END_QUERY", "needle",
      "#+BEGIN_QUERY\n{:query [:find (pull ?b [*]) :where [?b :block/content \"needle\"]]}\n#+END_QUERY"],
    ["{{query \"needle\"}}", "needle", "{{query \"needle\"}}"],
  ];
  for (const [raw, query, all] of cases) {
    for (const mode of ["all", "one"] as const) {
      it(`${JSON.stringify(raw)}, Find ${JSON.stringify(query)}: Replace ${mode}`, async () => {
        const { dispose } = mountPage([blk("b", raw)], format);
        try {
          await settle();
          const found = await find(query);
          expect(found).toBeGreaterThan(0);
          const outcome = mode === "all" ? replaceAllInPageFindMatches("X") : walkReplace(found);
          expect(doc.byId.b.raw).toBe(all);
          expect(outcome.replaced + outcome.skipped).toBe(found);
        } finally {
          dispose();
        }
      });
    }
  }
});

describe.each(["md", "org"] as const)("%s: a Twemoji the user searched for is its own source bytes", (format) => {
  const em = format === "md" ? "*😄*" : "/😄/";
  // [raw, after Replace all]
  const cases: [string, string][] = [["😄", "X"], ["😄 a", "X a"], ["a 😄", "a X"], ["😄😄", "XX"], [em, em.replace("😄", "X")], ["a 😄 b", "a X b"]];
  for (const [raw, all] of cases) {
    it(`${JSON.stringify(raw)}: every glyph is written by Replace all and by Replace`, async () => {
      const { root, dispose } = mountPage([blk("b", raw)], format);
      try {
        await settle();
        expect(root.querySelector("img.emoji")).not.toBeNull();
        const found = await find("😄");
        expect(found).toBe([...raw.matchAll(/😄/gu)].length);
        expect(replaceAllInPageFindMatches("X")).toEqual({ replaced: found, skipped: 0 });
        expect(doc.byId.b.raw).toBe(all);
      } finally {
        dispose();
      }
      const again = mountPage([blk("b", raw)], format);
      try {
        await settle();
        const found = await find("😄");
        expect(found).toBe([...raw.matchAll(/😄/gu)].length);
        expect(walkReplace(found)).toEqual({ replaced: found, skipped: 0 });
        expect(doc.byId.b.raw).toBe(all);
      } finally {
        again.dispose();
      }
    });
  }
});

describe("surfaces that draw a block without its text positions", () => {
  it("a nested bullet in a sheet grid cell: the plain word is written, the embed macro is skipped", async () => {
    const grid: BlockDto = { id: "grid", raw: "Grid\ntine.view:: grid", collapsed: false, children: [
      { id: "row", raw: "", collapsed: false, children: [
        { id: "cell", raw: "Cell", collapsed: false, children: [blk("nested", "needle {{embed [[needle]]}}")] },
      ] },
    ] };
    for (const mode of ["all", "one"] as const) {
      const { root, dispose } = mountPage([grid], "md", true);
      try {
        await settle();
        expect(root.querySelector('[data-sheet-block-id="nested"]')).not.toBeNull();
        const found = await find("needle");
        expect(found).toBe(2);
        const outcome = mode === "all" ? replaceAllInPageFindMatches("X") : walkReplace(found);
        expect(doc.byId.nested.raw).toBe("X {{embed [[needle]]}}");
        expect(outcome).toEqual({ replaced: 1, skipped: 1 });
      } finally {
        dispose();
      }
    }
  });

  it("a board card title, an annotation label, a lazy shell and a query shell: unchanged, and every match counted in the find bar", async () => {
    vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} takeRecords() { return []; } });
    for (const id of ["board", "card", "annotation", "query", "begin"]) renderedBlocks.add(id);
    const board: BlockDto = { id: "board", raw: "Board\ntine.view:: board", collapsed: false, children: [blk("card", "needle card")] };
    const raws: Record<string, string> = {
      annotation: "needle label\nls-type:: annotation\nhl-page:: 1",
      query: "{{query \"zzz-no-result needle\"}}",
      begin: "#+BEGIN_QUERY\n{:query [:find (pull ?b [*]) :where [?b :block/content \"needle\"]]}\n#+END_QUERY",
      lazy: "needle lazy",
    };
    const { root, dispose } = mountPage([board, ...Object.entries(raws).map(([id, raw]) => blk(id, raw))], "md", true);
    try {
      await settle();
      expect(root.querySelector('[data-block-id="card"] .sheet-board-card-title')?.textContent).toContain("needle card");
      expect(root.querySelector('[data-block-id="lazy"] .ast-deferred')).not.toBeNull();
      const found = await find("needle");
      expect(found).toBe(5);
      expect(replaceAllInPageFindMatches("X")).toMatchObject({ replaced: 0, skipped: 5 });
      expect(doc.byId.card.raw).toBe("needle card");
      for (const [id, raw] of Object.entries(raws)) expect(doc.byId[id].raw).toBe(raw);
      expect(inPageReplaceSkippedHere()).toBe(5);
      await settle();
      expect(skippedLine(root)).toBe("5 matches can't be replaced here");
      expect(walkReplace(found)).toEqual({ replaced: 0, skipped: 5 });
    } finally {
      dispose();
    }
  });

  it("the block open in the editor: `top ![needle](x.png)` is skipped and counted", async () => {
    const { dispose } = mountPage([blk("open", "top ![needle](x.png)"), blk("closed", "needle")]);
    try {
      startEditing("open", 0);
      await settle();
      const found = await find("needle");
      expect(found).toBe(2);
      expect(replaceAllInPageFindMatches("X")).toEqual({ replaced: 1, skipped: 1 });
      expect([doc.byId.open.raw, doc.byId.closed.raw]).toEqual(["top ![needle](x.png)", "X"]);
    } finally {
      dispose();
    }
  });
});

describe("which copy of a block counts", () => {
  it("a block drawn twice (itself and an embed of it) is written once; the embed's copy alone is enough", async () => {
    for (const folded of [false, true]) {
      const own = blk(X_ID, `needle here${ID_MD(X_ID)}`);
      vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids: string[]) => ids.map((id) => (id === X_ID ? { page: NAME, kind: "page" as const, blocks: [{ ...own, raw: doc.byId[X_ID]?.raw ?? own.raw }] } : null)) as never);
      // Folded: the block itself is not drawn (under a collapsed parent); only the embed draws it.
      const tree = folded ? [{ id: "parent", raw: "parent", collapsed: true, children: [own] }] : [own];
      const { root, dispose } = mountPage([blk("host", `{{embed ((${X_ID}))}}`), ...tree]);
      try {
        await settle();
        await vi.waitFor(() => expect(root.querySelectorAll(`[data-block-id="${X_ID}"] [data-so]`).length).toBeGreaterThan(0));
        expect(root.querySelectorAll(`[data-block-id="${X_ID}"]`).length).toBe(folded ? 1 : 2);
        const found = await find("needle");
        const outcome = replaceAllInPageFindMatches("X");
        expect(doc.byId[X_ID].raw).toBe(`X here${ID_MD(X_ID)}`);
        expect(outcome.replaced).toBe(1);
        expect(outcome.replaced + outcome.skipped).toBe(found);
      } finally {
        dispose();
      }
    }
  });

  it("a query result's snapshot of an older text authorizes nothing in the block's current text", async () => {
    // The result was drawn from `**needle**`; a reload installs an image whose
    // alt is `needle` at the same source bytes, under the same id.
    const hit: BlockDto = { id: X_ID, raw: "**needle**", collapsed: false, children: [] };
    install([{ ...hit }], "md", false);
    expect(adoptResults(hit, captureResultsTicket())).toBe(hit);
    const { root, dispose } = mount(() => (
      <>
        <div data-pane-id="main">
          <LiveRefGroup page={NAME} kind="page" path={`pages/${NAME}.md`} blocks={[hit]} surface="query" eager />
          <Show when={pageByName(NAME)} keyed>{(p) => <For each={p.roots}>{(id) => <Block id={id} />}</For>}</Show>
        </div>
        <InPageFind />
      </>
    ));
    try {
      await settle();
      reloadPage({ id: `pages/${NAME}.md`, name: NAME, title: NAME, kind: "page", pre_block: null, rev: "r2",
        blocks: [{ id: X_ID, raw: "![needle](https://e.org/x.png)", collapsed: false, children: [] }] });
      await settle();
      expect(doc.byId[X_ID].raw).toBe("![needle](https://e.org/x.png)");
      await vi.waitFor(() => expect(root.querySelector(`[data-block-id="${X_ID}"] strong`)?.textContent).toBe("needle"));
      const found = await find("needle");
      expect(found).toBeGreaterThan(0);
      for (const press of [() => replaceAllInPageFindMatches("X"), () => walkReplace(found)]) {
        const outcome = press();
        expect(doc.byId[X_ID].raw).toBe("![needle](https://e.org/x.png)");
        expect(outcome).toMatchObject({ replaced: 0, skipped: found });
      }
    } finally {
      dispose();
    }
  });
});

describe.each(["md", "org"] as const)("%s: every match Find holds is written or counted", (format) => {
  it("an embedded block from another page, a macro, an iconned link and plain hits: written + skipped = found", async () => {
    const other = { ...page("Other", [blk(B_ID, `foo\nbar${format === "md" ? ID_MD(B_ID) : ID_ORG(B_ID)}`)]), format };
    vi.spyOn(backend(), "getPage").mockImplementation(async (name: string) => (name.toLowerCase() === "other" ? other : null) as never);
    const blocks = [blk("embed", "{{embed [[Other]]}}"), blk("macro", "{{video https://e.org/foobar}}"), blk("icon", "see [[Foobar]]"),
      blk("p1", "foobar one"), blk("p2", "two foobar")];
    for (const mode of ["all", "one"] as const) {
      const { root, dispose } = mountPage(blocks, format);
      try {
        await vi.waitFor(() => expect(root.querySelector(".embed-block")?.textContent).toContain("bar"));
        const found = await find("foobar");
        expect(found).toBeGreaterThanOrEqual(4);
        const outcome = mode === "all" ? replaceAllInPageFindMatches("X") : walkReplace(found);
        expect([doc.byId.p1.raw, doc.byId.p2.raw]).toEqual(["X one", "two X"]);
        expect([doc.byId.macro.raw, doc.byId.icon.raw]).toEqual(["{{video https://e.org/foobar}}", "see [[Foobar]]"]);
        expect(outcome.replaced).toBe(2);
        expect(outcome.replaced + outcome.skipped).toBe(found);
      } finally {
        dispose();
      }
    }
  });
});

describe("the find bar's skipped line", () => {
  it("shows the count the last Replace left and clears when the query changes", async () => {
    const { root, dispose } = mountPage([blk("b", "{{embed [[needle]]}} needle"), blk("c", "{{embed [[needle]]}}")]);
    try {
      await settle();
      const line = () => skippedLine(root);
      expect(await find("needle")).toBe(3);
      expect(replaceAllInPageFindMatches("X")).toEqual({ replaced: 1, skipped: 2 });
      await settle();
      expect(line()).toBe("2 matches can't be replaced here");
      setInPageFindQuery("needl");
      await settle();
      expect(line()).toBeNull();
      setInPageFindQuery("needle");
      await settle();
      activateInPageFindIndex(0);
      expect(replaceActiveInPageFindMatch("X")).toMatchObject({ replaced: 0, skipped: 1 });
      await settle();
      expect(line()).toBe("1 match can't be replaced here");
    } finally {
      dispose();
    }
  });
});
