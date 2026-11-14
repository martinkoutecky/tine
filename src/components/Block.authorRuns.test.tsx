import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { openCommentThread } from "../tests/commentEntry";
import { render } from "solid-js/web";
import { BlockList } from "./BlockList";
import { Block, SurfaceContext } from "./Block";
import { blockRegions, initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { pageByName, resetStore, setRaw, setBlockProperty, deleteBlock, undo } from "../document";
import { endEdit, startEditing } from "../editorController";
import { bindTestHost } from "../document/host/wiring.test.support";
import { editorIn } from "../tests/editorDriver";
import { backend } from "../backend";
import type { BlockDto } from "../types";
import * as runs from "./authorRuns";
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
  // A run is one card: its members carry the join classes the tint's corners and overlap follow.
  expect([...row(root, "b").classList].filter(name => name.startsWith("author-run-"))).toEqual(["author-run-continued", "author-run-continues"]);
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
  expect(css).toMatch(/\.ls-block\.author-tinted > \.block-main > \.block-content-wrapper \{[^}]*padding-inline: var\(--author-tint-bleed\);[^}]*margin-inline: calc\(-1 \* var\(--author-tint-bleed\)\)/);
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
