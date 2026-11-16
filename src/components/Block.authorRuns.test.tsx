import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { openCommentThread } from "../tests/commentEntry";
import { render } from "solid-js/web";
import { BlockList } from "./BlockList";
import { Block, SurfaceContext } from "./Block";
import { blockRegions, initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { pageByName, resetStore, setRaw, setCollapsed, setBlockProperty, deleteBlock, undo } from "../document";
import { endEdit, startEditing } from "../editorController";
import { bindTestHost } from "../document/host/wiring.test.support";
import { editorIn } from "../tests/editorDriver";
import { backend } from "../backend";
import type { BlockDto } from "../types";
import * as runs from "./authorRuns";
import * as commentChildren from "../commentChildren";
import { setAuthorMarksHidden } from "../authorMarksPref";
import { readFileSync } from "node:fs";

let dispose: (() => void) | undefined;
beforeAll(() => initParser());
beforeEach(async () => {
  await bindTestHost();
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async ids => ids.map(() => null));
});
afterEach(() => {
  endEdit("page-navigation"); dispose?.(); resetStore(); vi.restoreAllMocks();
  document.body.innerHTML = "";
});
const b = (id: string, by: string | null = "Ada", children: BlockDto[] = [], body = id): BlockDto => {
  const raw = body + (by === null ? "" : `\ntine.author:: ${by}`);
  return { id, raw, children, collapsed: false,
    properties: blockRegions(raw, "md").properties.filter(p => p.primary).map(p => [p.key, p.value]) };
};
function mount(blocks: BlockDto[], second = false, format: "md" | "org" = "md", idsFor?: () => readonly string[]) {
  loadSingle({ name: "Runs", title: "Runs", kind: "page", format, pre_block: null, blocks });
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <><div class="main"><BlockList ids={idsFor?.() ?? pageByName("Runs")!.roots} /></div>
    {second && <div class="copy"><SurfaceContext.Provider value="embed:test"><BlockList ids={["b", "c"]} /></SurfaceContext.Provider></div>}</>, root);
  return root;
}
const chipNames = (root: ParentNode) => [...root.querySelectorAll(".author-chip")].map(e => e.textContent);
const row = (root: ParentNode, id: string) => root.querySelector<HTMLElement>(`[data-block-id="${id}"]`)!;

const tinted = (root: ParentNode, id: string) => row(root, id).classList.contains("author-tinted");
const wrapperOf = (root: ParentNode, id: string) => row(root, id).querySelector<HTMLElement>(":scope > .block-main > .block-content-wrapper")!;

it("same-author siblings share a chip, while each tinted row names its own author", () => {
  const root = mount([b("a"), b("b"), b("c"), b("own", null), b("d"), b("e", "Bo")]);
  expect(chipNames(root)).toEqual(["Ada", "Ada", "Bo"]);
  for (const id of ["a", "b", "c", "d", "e"]) {
    expect(tinted(root, id)).toBe(true);
    expect(wrapperOf(root, id).getAttribute("title")).toContain(id === "e" ? "Bo" : "Ada");
    expect(row(root, id).querySelector(".author-mark")).toBeNull();   // GH #893: no gutter line element exists
  }
  expect(tinted(root, "own")).toBe(false);
  expect(wrapperOf(root, "own").getAttribute("title")).toBeNull();
  // A run is one card: its members carry the join classes the tint's corners and overlap follow (card),
  // and the public run classes the thread follows (run).
  expect([...row(root, "b").classList].filter(name => /^author-(card|run)-/.test(name)).sort())
    .toEqual(["author-card-continued", "author-card-continues", "author-run-continued", "author-run-continues"]);
});

/** The two public run classes of a row, as the contract's section 8 names them. */
const runOf = (root: ParentNode, id: string) => [row(root, id).classList.contains("author-run-continued") ? "up" : "", row(root, id).classList.contains("author-run-continues") ? "down" : ""].filter(Boolean).join("+");
const cardOf = (root: ParentNode, id: string) => [row(root, id).classList.contains("author-card-continued") ? "up" : "", row(root, id).classList.contains("author-card-continues") ? "down" : ""].filter(Boolean).join("+");

it("a run crosses depth: parent into its first child, out of the last descendant to the next block; the tint's card does not", () => {
  const root = mount([b("p", "Ada", [b("k1", "Ada", [b("g")]), b("k2")]), b("n"), b("x", "Bo")]);
  expect(["p", "k1", "g", "k2", "n", "x"].map(id => runOf(root, id))).toEqual(["down", "up+down", "up+down", "up+down", "up", ""]);
  // Cards join siblings only, and only childless ones: nothing bends around a child.
  expect(["p", "k1", "g", "k2", "n", "x"].map(id => cardOf(root, id))).toEqual(["", "", "", "", "", ""]);
});

it("a run ends at a block of someone else, at your own block and at a comment; a folded block ends where it is drawn", () => {
  const folded = { ...b("f", "Ada", [b("hidden")]), collapsed: true };
  const root = mount([b("a", "Ada", [b("mine", null)]), b("a2"), folded, b("f2"),
    b("c", "Ada", [b("thread", "Bo", [], "reply\ntine.quote:: c")]), b("c2"),
    b("p", "Ada", [b("other", "Bo")]), b("p2")]);
  // a: its first child is yours, so no run down; a2 follows a's last row (yours), so none up.
  expect([runOf(root, "a"), runOf(root, "a2")]).toEqual(["", "down"]);
  // A folded block hides its child: the run goes on to its next sibling.
  expect([runOf(root, "f"), runOf(root, "f2")]).toEqual(["up+down", "up+down"]);
  // A margin comment is not a row of the outline.
  expect([runOf(root, "c"), runOf(root, "c2")]).toEqual(["up+down", "up+down"]);
  // A first child by someone else ends the run below p, and p2 follows that child.
  expect([runOf(root, "p"), runOf(root, "other"), runOf(root, "p2")]).toEqual(["up", "", ""]);
});

it("names compare by key (case, spaces), data-author keeps the stored text", () => {
  const root = mount([b("a", "Ada  Lovelace"), b("b", "ada lovelace"), b("c", "Bo")]);
  expect([runOf(root, "a"), runOf(root, "b"), runOf(root, "c")]).toEqual(["down", "up", ""]);
  expect([row(root, "a"), row(root, "b")].map(r => r.getAttribute("data-author"))).toEqual(["Ada  Lovelace", "ada lovelace"]);
  // The chip carries the trimmed name, so an icon rule can name it (and a run's one chip is the first row's).
  expect([...root.querySelectorAll(".author-chip")].map(c => c.getAttribute("data-author"))).toEqual(["Ada  Lovelace", "Bo"]);
});

it("a results list keeps hits apart, and the outline rules apply inside one hit", () => {
  loadSingle({ name: "Runs", title: "Runs", kind: "page", format: "md", pre_block: null, blocks: [b("a", "Ada", [b("k")]), b("b")] });
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <BlockList ids={pageByName("Runs")!.roots} results />, root);
  expect([runOf(root, "a"), runOf(root, "k"), runOf(root, "b")]).toEqual(["down", "up", ""]);
});

it("the Show/Hide author marks switch turns the tint off and leaves the chips and the file alone", () => {
  const root = mount([b("a"), b("b"), b("own", null)]);
  const raw = doc.byId.a.raw;
  try {
    expect(tinted(root, "a")).toBe(true);
    setAuthorMarksHidden(true);
    expect(tinted(root, "a") || tinted(root, "b")).toBe(false);
    expect(wrapperOf(root, "a").getAttribute("title")).toBeNull();
    expect(chipNames(root)).toEqual(["Ada"]);
    expect(doc.byId.a.raw).toBe(raw);
  } finally { setAuthorMarksHidden(false); }
  expect(tinted(root, "a")).toBe(true);
});

it("GH #893: no gutter-line rule or element survives in the stylesheet or the components", () => {
  const css = readFileSync("src/styles/comments.css", "utf8");
  expect(css).not.toMatch(/\.author-mark\b/);
  expect(css).toMatch(/:where\(:root:not\(\.tine-author-marks-thread\)\) \.ls-block\.author-tinted > \.block-main > \.block-content-wrapper \{[^}]*padding-inline: var\(--author-tint-bleed\);[^}]*margin-inline: calc\(-1 \* var\(--author-tint-bleed\)\)/);
});

it("child outlines, comment threads and embeds bound runs; no inheritance", () => {
  const root = mount([b("a"), b("parent", "Ada", [b("kid"), b("kid2"), b("owner", null)]),
    b("after"), b("commented", "Ada", [b("thread", "Bo", [], "reply\ntine.quote:: commented")]),
    b("afterThread"), b("embed", "Ada", [], "{{embed [[Missing source]]}}"), b("afterEmbed")]);
  // The parent can finish the preceding run; neither it nor a thread lends
  // authorship to a child, and a child outline cannot bridge to a later sibling.
  for (const id of ["a", "kid", "after", "afterThread", "afterEmbed"])
    expect(row(root, id).querySelector(":scope > .block-main .author-chip")?.textContent).toBe("Ada");
  for (const id of ["parent", "kid2", "commented"])
    expect(row(root, id).querySelector(":scope > .block-main .author-chip")).toBeNull();
  openCommentThread(row(root, "commented"));
  expect(tinted(document.body, "thread")).toBe(false);
  expect(tinted(root, "embed")).toBe(true);
  expect(tinted(root, "owner") || row(root, "owner").querySelector(".author-chip") !== null).toBe(false);
});

it("a bulletless whole-block embed host does not tint itself", () => {
  const root = mount([b("a"), b("embed", "Ada", [], "{{embed ((12345678-1234-4000-8000-123456789abc))}}"), b("after")]);
  expect(tinted(root, "embed")).toBe(false);
  expect(row(root, "after").querySelector(".author-chip")?.textContent).toBe("Ada");
});

it("a separately rendered subset starts its own run", () => {
  const root = mount([b("a"), b("b"), b("c")], true);
  expect(chipNames(root.querySelector(".main")!)).toEqual(["Ada"]);
  expect(chipNames(root.querySelector(".copy")!)).toEqual(["Ada"]);
  expect(row(root.querySelector(".copy")!, "b").querySelector(".author-chip")).not.toBeNull();
});

it("inline embeds bound runs; literal macro examples do not", () => {
  const root = mount([b("a"), b("inline", "Ada", [], "Text before {{embed [[Missing source]]}}"), b("after"),
    b("code", "Ada", [], "```\n{{embed [[Example]]}}\n```"),
    b("literal", "Ada", [], "An inline `{{embed [[Example]]}}` example"), b("last")]);
  expect(chipNames(root)).toEqual(["Ada", "Ada", "Ada"]);
  expect(row(root, "inline").querySelector(":scope > .block-main .author-chip")?.textContent).toBe("Ada");
  expect(row(root, "after").querySelector(".author-chip")?.textContent).toBe("Ada");
  for (const id of ["code", "literal", "last"]) expect(row(root, id).querySelector(".author-chip")).toBeNull();
});

it("both editors keep the tint and original author bytes as runs change live", () => {
  const root = mount([b("a"), b("b"), b("c")]);
  const raw = doc.byId.b.raw;
  startEditing("b", 0);
  expect(tinted(root, "b")).toBe(true);
  expect(wrapperOf(root, "b").getAttribute("title")).toBeNull();   // no tooltip over the editor
  expect(editorIn(row(root, "b")).value()).toBe("b");
  editorIn(row(root, "b")).input("edited"); endEdit("page-navigation");
  expect(doc.byId.b.raw).toBe(raw.replace("b\n", "edited\n"));
  expect(chipNames(root)).toEqual(["Ada"]);
  setBlockProperty("b", "tine.author", "Bo");
  expect(chipNames(root)).toEqual(["Ada", "Bo", "Ada"]);
  setBlockProperty("b", "tine.author", null);
  expect(chipNames(root)).toEqual(["Ada", "Ada"]);
  setRaw("b", raw);
  expect(chipNames(root)).toEqual(["Ada"]);
});

it("removing the first sibling and undoing updates the run's leading chip", () => {
  const root = mount([b("a"), b("b"), b("c")]);
  deleteBlock("a");
  expect(row(root, "b").querySelector(".author-chip")?.textContent).toBe("Ada");
  expect(row(root, "c").querySelector(".author-chip")).toBeNull();
  undo();
  expect(row(root, "a").querySelector(".author-chip")?.textContent).toBe("Ada");
  expect(row(root, "b").querySelector(".author-chip")).toBeNull();
});

it("individually rendered blocks keep their chip", () => {
  mount([b("a"), b("b")]); dispose?.();
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <Block id="b" />, root);
  expect(chipNames(root)).toEqual(["Ada"]);
});

it("Org author drawers form runs and remain byte-exact through editing", () => {
  const blocks = ["a", "b", "c"].map(id => ({ id, raw: `${id}\n:PROPERTIES:\n:tine.author: Ada  \t\n:END:`,
    properties: [["tine.author", "Ada"]] as [string, string][], collapsed: false, children: [] }));
  const root = mount(blocks, false, "org");
  expect(chipNames(root)).toEqual(["Ada"]);
  startEditing("b", 0); const editor = editorIn(row(root, "b"));
  expect(editor.value()).toBe("b"); editor.input("revised"); endEdit("page-navigation");
  expect(doc.byId.b.raw).toBe(blocks[1].raw.replace("b\n", "revised\n"));
  expect(chipNames(root)).toEqual(["Ada"]);
});

it.each([8, 32])("typing in one of %i siblings only revisits neighbouring author comparisons", count => {
  mount(Array.from({ length: count }, (_, i) => b(`row${i}`)));
  const comparisons = vi.spyOn(runs, "authorRunContinues");
  const middle = count / 2;
  setRaw(`row${middle}`, `edited words\ntine.author:: Ada`);
  const touched = new Set(comparisons.mock.calls.flatMap(([left, right]) => [left, right]).filter(Boolean));
  expect(comparisons.mock.calls.length).toBeGreaterThan(0);
  expect(comparisons.mock.calls.length).toBeLessThanOrEqual(12);
  expect(touched.size).toBeLessThanOrEqual(5);
  for (const id of touched) expect(Math.abs(Number(id!.slice(3)) - middle)).toBeLessThanOrEqual(2);
});

it.each([1, 2, 3].flatMap(depth => [8, 32].map(count => [depth, count] as const)))("editing an ancestor at depth %i does not recompute its %i authored descendants' runs", (depth, count) => {
  let children = Array.from({ length: count }, (_, i) => b(`child${i}`));
  for (let level = 1; level < depth; level++) children = [b(`nested${level}`, "Ada", children)];
  mount([b("parent", "Ada", children)]);
  const comparisons = vi.spyOn(runs, "authorRunContinues");
  setRaw("parent", "edited parent words\ntine.author:: Ada");
  const touched = new Set(comparisons.mock.calls.flatMap(([left, right]) => [left, right]).filter(Boolean));
  expect(comparisons.mock.calls.length).toBeGreaterThan(0);
  expect(comparisons.mock.calls.length).toBeLessThanOrEqual(12);
  expect([...touched]).toEqual(["parent"]);
});

it.each([8, 32])("a view re-emitting the same %i sibling IDs does not recompute all runs", count => {
  const middle = count / 2;
  mount(Array.from({ length: count }, (_, i) => b(`row${i}`)), false, "md", () => {
    // View filters can observe body changes but retain the same membership.
    void doc.byId[`row${middle}`].raw;
    return [...pageByName("Runs")!.roots];
  });
  const comparisons = vi.spyOn(runs, "authorRunContinues");
  setRaw(`row${middle}`, "edited words\ntine.author:: Ada");
  expect(comparisons.mock.calls.length).toBeGreaterThan(0);
  expect(comparisons.mock.calls.length).toBeLessThanOrEqual(12);
});

it.each([8, 32])("typing in one of %i siblings revisits only the thread sides of its two neighbours", count => {
  mount(Array.from({ length: count }, (_, i) => b(`row${i}`)));
  const sides = vi.spyOn(runs, "authorThreadSides");
  const middle = count / 2;
  setRaw(`row${middle}`, `edited words\ntine.author:: Ada`);
  const touched = new Set(sides.mock.calls.map(([id]) => id));
  expect(sides.mock.calls.length).toBeGreaterThan(0);
  expect(sides.mock.calls.length).toBeLessThanOrEqual(12);
  for (const id of touched) expect(Math.abs(Number(id.slice(3)) - middle)).toBeLessThanOrEqual(1);
});

it.each([1, 2, 3].flatMap(depth => [8, 32].map(count => [depth, count] as const)))("editing an ancestor at depth %i revisits its first child's thread sides, not its %i descendants'", (depth, count) => {
  let children = Array.from({ length: count }, (_, i) => b(`child${i}`));
  for (let level = 1; level < depth; level++) children = [b(`nested${level}`, "Ada", children)];
  mount([b("parent", "Ada", children)]);
  const sides = vi.spyOn(runs, "authorThreadSides");
  setRaw("parent", "edited parent words\ntine.author:: Ada");
  const touched = new Set(sides.mock.calls.map(([id]) => id));
  expect(sides.mock.calls.length).toBeGreaterThan(0);
  expect(sides.mock.calls.length).toBeLessThanOrEqual(12);
  expect([...touched].sort()).toEqual(["parent", depth === 1 ? "child0" : `nested${depth - 1}`].sort());
});

it("renders every public authorship hook the contract's section 8 table promises", () => {
  const root = mount([b("a", "Ada", [b("k")]), b("b", "Ada"), b("c", "Bo")]);
  const section = readFileSync("docs/contracts/theme-tokens.md", "utf8").split("## 8. Authorship styling")[1]?.split("## Tests")[0] ?? "";
  const cells = [...section.matchAll(/^\| (.+?) \|/gm)].map(m => m[1]).filter(cell => cell.includes("`"));
  const selectors = cells.flatMap(cell => [...cell.matchAll(/`([^`]+)`/g)].map(m => m[1]))
    .filter(sel => !sel.startsWith("--") && !sel.startsWith("html."))
    .map(sel => sel.replace(/^data-author="<name>"$/, "[data-author]"));
  expect(selectors.length).toBeGreaterThanOrEqual(7);
  for (const selector of selectors) expect(root.querySelector(selector), `${selector} is a public authorship hook`).not.toBeNull();
  // The two names that are not elements: the window class is in the stylesheet, and `--author-color` is defined
  // for every tint class an authored row carries (jsdom loads no stylesheet, so read the rule).
  const css = readFileSync("src/styles/comments.css", "utf8");
  expect(css).toContain(".tine-author-marks-thread {");
  for (const id of ["a", "b", "c"]) {
    const tint = [...row(root, id).classList].find(name => /^author-tint-\d$/.test(name));
    expect(tint, `${id} carries a tint class`).toBeDefined();
    expect(css).toContain(`.${tint} { --author-color:`);
  }
});

// GH #892 review F1: the previous sibling's last drawn row is found from the covering comment index (the one
// owner of comment membership), never by screening the hidden comments again. Typing costs the same however
// many hidden comments follow the last visible child.
const quote = (id: string, parent: string) => b(id, "Bo", [], `note ${id}\ntine.quote:: ${parent}`);
const screened = () => vi.spyOn(commentChildren, "commentChildOf");
it.each([8, 32])("typing beside %i hidden comments screens a constant number of blocks for membership", count => {
  const comments = Array.from({ length: count }, (_, i) => quote(`c${i}`, "p"));
  const costs = ["p", "last", "c1"].map(edited => {
    const root = mount([b("p", "Ada", [b("first"), b("last"), ...comments]), b("n")]);
    // The thread runs from p's last visible row into n: that is the row the walk must find.
    expect(runOf(root, "n")).toBe("up");
    const calls = screened();
    setRaw(edited, edited === "c1" ? "other note\ntine.quote:: p\ntine.author:: Bo" : `${edited} edited\ntine.author:: Ada`);
    const cost = calls.mock.calls.length;
    calls.mockRestore(); dispose?.(); resetStore(); document.body.innerHTML = "";
    return cost;
  });
  // The index screens the one edited block; a walk over the tail would add one screen per hidden comment.
  for (const cost of costs) expect(cost).toBeLessThanOrEqual(4);
});

// GH #892 review (round 4) F1: typing in any row of the previous sibling's last-row spine, or in its hidden
// tail, must not re-walk that spine (a walk inspects every row it visits and, at each, the children from the
// end). `lastRow` is the walk, so its calls are the inspections: none when no fold or membership changed.
const walks = () => vi.spyOn(runs, "lastRow");
it.each([8, 32])("typing in the parent or last child above %i hidden comments does not re-walk the previous row", count => {
  const comments = Array.from({ length: count }, (_, i) => quote(`c${i}`, "p"));
  for (const edited of ["p", "last", "c1"]) {
    const root = mount([b("p", "Ada", [b("first"), b("last"), ...comments]), b("n")]);
    expect(runOf(root, "n")).toBe("up");
    const calls = walks();
    setRaw(edited, edited === "c1" ? "other note\ntine.quote:: p\ntine.author:: Bo" : `${edited} edited\ntine.author:: Ada`);
    expect(calls.mock.calls.length, `editing ${edited}`).toBe(0);
    expect(runOf(root, "n")).toBe("up");
    calls.mockRestore(); dispose?.(); resetStore(); document.body.innerHTML = "";
  }
});

/** A last-child chain of `depth` rows, root first: `s0` > `s1` > ... > the leaf `s<depth-1>`. */
const chain = (depth: number, leafBy = "Ada"): BlockDto => {
  let node = b(`s${depth - 1}`, leafBy);
  for (let level = depth - 2; level >= 0; level--) node = b(`s${level}`, "Ada", [node]);
  return node;
};
it.each([8, 32])("typing anywhere in a last-child chain of depth %i does not re-walk it for the next sibling", depth => {
  for (const edited of ["s0", `s${depth >> 1}`, `s${depth - 1}`]) {
    const root = mount([chain(depth), b("n")]);
    expect(runOf(root, "n")).toBe("up");
    const calls = walks();
    setRaw(edited, `${edited} edited\ntine.author:: Ada`);
    expect(calls.mock.calls.length, `editing ${edited}`).toBe(0);
    expect(runOf(root, "n")).toBe("up");
    calls.mockRestore(); dispose?.(); resetStore(); document.body.innerHTML = "";
  }
});

it.each([8, 32])("a real fold change on the spine (depth %i) still moves the thread's row above the next sibling", depth => {
  const mid = `s${depth >> 1}`;
  // Ada's chain ends in a leaf of Bo's, and so does `n`: the run joins only while the leaf is the row above it.
  const root = mount([chain(depth, "Bo"), b("n", "Bo")]);
  expect(runOf(root, "n")).toBe("up");
  setCollapsed(mid, true);
  expect(runOf(root, "n")).toBe("");
  setCollapsed(mid, false);
  expect(runOf(root, "n")).toBe("up");
});

// GH #892 review F3: the hits of a results list are independent. No thread link crosses from one hit to the
// next, so every hit starts with its own chip; the tint's card may still join adjacent leaf hits.
function mountResults(blocks: BlockDto[], hits: string[]) {
  loadSingle({ name: "Runs", title: "Runs", kind: "page", format: "md", pre_block: null, blocks });
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <BlockList ids={hits} results />, root);
  return root;
}
const chipOf = (root: ParentNode, id: string) => row(root, id).querySelector(":scope > .block-main .author-chip");
it.each([
  ["sibling leaf hits", () => [b("a"), b("b")]],
  ["leaf hits under different parents", () => [b("p1", "Ada", [b("a")]), b("p2", "Ada", [b("b")])]],
  ["a leaf hit followed by an expanded hit", () => [b("a"), b("b", "Ada", [b("k")])]],
  ["a leaf hit followed by a folded hit", () => [b("a"), { ...b("b", "Ada", [b("k")]), collapsed: true }]],
])("results: %s are not threaded together and each starts with a chip", (_name, blocks) => {
  const hits = ["a", "b"];
  const root = mountResults(blocks(), hits);
  // The leaf hit has no run class at all; the next hit may continue into its own child, never up into the leaf.
  expect(runOf(root, "a")).toBe("");
  expect(runOf(root, "b")).not.toContain("up");
  for (const id of hits) expect(chipOf(root, id), `${id} starts with its own chip`).not.toBeNull();
});

it("results: the outline rules still apply inside one hit, and the tint card may join leaf hits", () => {
  const root = mountResults([b("a", "Ada", [b("k")]), b("b"), b("c")], ["a", "b", "c"]);
  expect(["a", "k", "b", "c"].map(id => runOf(root, id))).toEqual(["down", "up", "", ""]);
  expect(["b", "c"].map(id => cardOf(root, id))).toEqual(["down", "up"]);
});

it("the tint hides the chip of a card-joined hit with the stylesheet, so it keeps its look", () => {
  const css = readFileSync("src/styles/comments.css", "utf8");
  expect(css).toMatch(/:where\(:root:not\(\.tine-author-marks-thread\)\) \.ls-block\.author-card-continued > \.block-main > \.block-content-wrapper > \.block-content > \.author-chip/);
});

it("a block that becomes a comment (or stops being one) moves the row above its next sibling, live", () => {
  const root = mount([b("p", "Ada", [b("first"), b("last", "Bo")]), b("n")]);
  expect(runOf(root, "n")).toBe("");   // p's last drawn row is by Bo
  setRaw("last", "last\ntine.author:: Bo\ntine.quote:: p");
  expect(runOf(root, "n")).toBe("up");   // a comment is not a row: first (Ada) is
  setRaw("last", "last\ntine.author:: Bo");
  expect(runOf(root, "n")).toBe("");
});
