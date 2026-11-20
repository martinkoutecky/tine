import { afterEach, expect, it, vi } from "vitest";
import { batch, createComputed, createRoot, getOwner } from "solid-js";
import { unwrap } from "solid-js/store";
import { loadFeed, resetStore } from "./workingSet";
import { depthOf, depthsOf } from "./tree";
import { isCommentId, threadRootOf, threadRootsOf } from "./commentThreads";
import { selectBlock, moveSelection, topSelected, clearSelection } from "./edits/selection";
import { exportNodesFor } from "./edits/serialize";
import { referenceExportAtDepth, referenceExportCandidates } from "../editor/exportProjection";
import { DEFAULT_EXPORT_OPTIONS, exportOutline } from "../editor/exportText";
import { doc, setDoc } from "./model";
import * as model from "./model";
import { blockRegions } from "../render/parse";
import type { BlockDto, Format } from "../types";

const subscriptions = vi.hoisted(() => ({ releases: [] as ReturnType<typeof vi.fn>[], detached: [] as boolean[] }));
vi.mock("./model", async importOriginal => {
  const actual = await importOriginal<typeof import("./model")>();
  return { ...actual, onBlockRawChanged: vi.fn((fn: (id: string) => void, ids: Iterable<string>) => {
    const release = vi.fn(actual.onBlockRawChanged(fn, ids));
    subscriptions.releases.push(release);
    subscriptions.detached.push(getOwner()?.owner === null);
    return release;
  }) };
});

afterEach(() => { resetStore(); vi.restoreAllMocks(); });

function page(format: Format) {
  const quote = format === "md" ? "\ntine.quote:: body" : "\n:PROPERTIES:\n:tine.quote: body\n:END:";
  const block = (name: string, children: BlockDto[] = [], comment = false): BlockDto => {
    const raw = name + (comment ? quote : "");
    const properties = blockRegions(raw, format).properties.filter(p => p.primary).map((p): [string, string] => [p.key, p.value]);
    return { id: `${format}-${name}`, raw, properties, children, collapsed: false };
  };
  return { name: format, title: format, kind: "page" as const, format, pre_block: null, blocks: [
    block("body", [block("comment", [block("nested", [block("reply")], true)], true), block("child")], true),
    block("tail"),
  ] };
}

function countParentReads(ids: string[]) {
  let reads = 0;
  for (const id of ids) {
    const node = unwrap(doc.byId[id]);
    const parent = node.parent;
    Object.defineProperty(node, "parent", { get() { reads++; return parent; }, configurable: true });
  }
  return () => reads;
}

it.each([false, true])("batch answers preserve outermost threads and source depths across pages (reverse=%s)", reverse => {
  loadFeed([page("md"), page("org")]);
  const names = ["body", "comment", "nested", "reply", "child", "tail"];
  const ids = ["md", "org"].flatMap(format => names.map(name => `${format}-${name}`));
  if (reverse) ids.reverse();
  const roots = threadRootsOf(ids);
  const depths = depthsOf(ids);
  const expectedDepths: Record<string, number> = { body: 0, comment: 1, nested: 2, reply: 3, child: 1, tail: 0 };
  for (let at = 0; at < ids.length; at++) {
    const id = ids[at], [format, name] = id.split("-");
    const expectedRoot = ["comment", "nested", "reply"].includes(name) ? `${format}-comment` : null;
    expect(roots.get(id)).toBe(expectedRoot);
    expect(threadRootOf(id)).toBe(expectedRoot);
    expect(depths[at]).toBe(expectedDepths[name]);
    expect(depthOf(id)).toBe(expectedDepths[name]);
  }
});

it("batch scopes include omitted ancestors and recompute after raw and parent edits", () => {
  loadFeed([page("md")]);
  const ids = ["md-reply", "md-child", "md-reply"];
  expect(depthsOf(ids)).toEqual([3, 1, 3]);
  expect(threadRootsOf(ids).get("md-reply")).toBe("md-comment");
  setDoc("byId", "md-comment", "raw", "ordinary");
  expect(threadRootsOf(ids).get("md-reply")).toBe("md-nested");
  batch(() => {
    setDoc("byId", "md-nested", "children", []);
    setDoc("byId", "md-body", "children", ids => [...ids, "md-reply"]);
    setDoc("byId", "md-reply", "parent", "md-body");
  });
  expect(depthsOf(ids)).toEqual([1, 1, 1]);
  expect(threadRootsOf(ids).get("md-reply")).toBeNull();
});

it.each([64, 128])("projects 50 loaded reference chains of depth %s with linear parent reads", depth => {
  const roots: BlockDto[] = [];
  const ids: string[] = [];
  for (let chain = 0; chain < 50; chain++) {
    let children: BlockDto[] = [];
    for (let level = depth; level > 0; level--) {
      const id = `${chain}-${level}`;
      ids.push(id);
      children = [{ id, raw: id, children, collapsed: false }];
    }
    roots.push(children[0]);
  }
  loadFeed([{ name: "Chains", title: "Chains", kind: "page", format: "md", pre_block: null, blocks: roots }]);
  const parentReads = countParentReads(ids);

  const forest = exportNodesFor(ids, true);
  expect(forest.map(root => root.raw)).toEqual(roots.map(root => root.raw));
  const levelOne = referenceExportAtDepth(referenceExportCandidates(forest), 1);
  expect(levelOne.map(node => node.raw)).toEqual(roots.flatMap((_, chain) =>
    Array.from({ length: depth }, (_, level) => `${chain}-${level + 1}`)));
  expect(parentReads()).toBeLessThanOrEqual(7 * ids.length);
});

it.each([false, true])("keeps parent reads linear for shared unchecked ancestors (checked root=%s)", checkedRoot => {
  const block = (id: string, children: BlockDto[] = []): BlockDto => ({ id, raw: id, children, collapsed: false });
  const leaves = Array.from({ length: 50 }, (_, index) => block(`leaf-${index}`));
  const ancestors = Array.from({ length: 127 }, (_, index) => `ancestor-${index}`);
  const roots = ancestors.reduceRight((children, id) => [block(id, children)], leaves);
  loadFeed([{ name: "Shared", title: "Shared", kind: "page", format: "md", pre_block: null, blocks: roots }]);
  const leafIds = leaves.map(leaf => leaf.id);
  const ids = checkedRoot ? [...leafIds, roots[0].id] : leafIds;
  const allIds = [...leafIds, ...ancestors];
  const parentReads = countParentReads(allIds);

  const forest = exportNodesFor(ids, true);
  expect(forest.map(root => root.raw)).toEqual(checkedRoot ? [roots[0].id] : leafIds);
  expect(parentReads()).toBeLessThanOrEqual(7 * allIds.length);
});

it.each(["md", "org"] as const)("keeps detached and promoted reference replies inside their source thread (%s)", format => {
  loadFeed([page(format)]);
  const options = { ...DEFAULT_EXPORT_OPTIONS, maxDepth: 1 };
  const exportFirstLevel = (ids: string[], includeComments: boolean) => {
    const forest = exportNodesFor(ids.map(id => `${format}-${id}`), true);
    const visible = referenceExportAtDepth(referenceExportCandidates(forest), 1);
    return exportOutline(visible, { ...options, includeComments });
  };
  expect(exportFirstLevel(["body", "reply"], true)).toContain("[Comment] reply");
  expect(exportFirstLevel(["body", "reply"], false)).toBe("- body");
  expect(exportFirstLevel(["reply"], false)).toBe("");
});

it.each(["md", "org"] as const)("thread readers follow membership deltas, including reparent and replacement (%s)", format => {
  loadFeed([page(format)]);
  const parent = `${format}-comment`, child = `${format}-reply`, nested = `${format}-nested`;
  const quote = format === "md" ? "\ntine.quote:: body" : "\n:PROPERTIES:\n:tine.quote: body\n:END:";
  let root: string | null = null, reads = 0;
  const dispose = createRoot(dispose => {
    createComputed(() => { root = threadRootOf(child); reads++; });
    return dispose;
  });
  try {
    expect(root).toBe(parent);
    const initial = reads;
    setDoc("byId", parent, "raw", `Edited comment${quote}`);
    setDoc("byId", `${format}-body`, "raw", "Edited outer body");
    expect(reads).toBe(initial);
    setDoc("byId", parent, "raw", "Ordinary parent");
    expect(root).toBe(nested);
    const ordinary = reads;
    setDoc("byId", parent, "raw", "Ordinary parent edited again");
    expect(reads).toBe(ordinary);
    setDoc("byId", parent, "raw", `Comment again${quote}`);
    expect(root).toBe(parent);
    setDoc("byId", parent, "parent", null);
    expect(root).toBe(nested);
    // Replacing a live node must not reuse its old membership derivation.
    const replacement = { ...doc.byId[parent], parent: `${format}-body`, raw: `Replacement${quote}` };
    batch(() => { setDoc("byId", parent, undefined!); setDoc("byId", parent, replacement); });
    expect(root).toBe(parent);
  } finally { dispose(); }
});

it("thread reads inside an edit batch see the raw change before reactive effects flush", () => {
  loadFeed([page("md")]);
  expect(threadRootOf("md-reply")).toBe("md-comment");
  batch(() => {
    setDoc("byId", "md-comment", "raw", "Ordinary parent");
    expect(threadRootOf("md-reply")).toBe("md-nested");
  });
});

it("shares only parent subscriptions and releases them on replacement and removal", () => {
  subscriptions.releases = [];
  const releases = subscriptions.releases;
  const watch = vi.mocked(model.onBlockRawChanged);
  watch.mockClear();
  loadFeed([page("md")]);
  const beforeLeaves = watch.mock.calls.length;
  expect(isCommentId("md-body")).toBe(false);
  expect(isCommentId("md-child")).toBe(false);
  expect(watch).toHaveBeenCalledTimes(beforeLeaves);
  expect(isCommentId("md-comment")).toBe(true);
  expect(isCommentId("md-comment")).toBe(true);
  expect(watch).toHaveBeenCalledOnce();
  const replacement = { ...doc.byId["md-comment"], raw: "Ordinary parent" };
  batch(() => { setDoc("byId", "md-comment", undefined!); setDoc("byId", "md-comment", replacement); });
  expect(releases[0]).toHaveBeenCalledOnce();
  expect(isCommentId("md-comment")).toBe(false);
  expect(watch).toHaveBeenCalledTimes(2);
  setDoc("byId", "md-comment", undefined!);
  expect(releases[1]).toHaveBeenCalledOnce();
});

it("source membership publishers do not retain the view owner that first queried them", () => {
  loadFeed([page("md")]);
  subscriptions.detached = [];
  createRoot(dispose => {
    expect(isCommentId("md-nested")).toBe(true);
    dispose();
  });
  expect(subscriptions.detached).toEqual([true]);
  setDoc("byId", "md-nested", "raw", "Ordinary parent");
  expect(isCommentId("md-nested")).toBe(false);
});

it("selection keeps body and comment navigation in their existing scopes", () => {
  loadFeed([page("md")]);
  const scope = { roots: ["md-body", "md-tail"] };
  selectBlock("md-child", scope);
  moveSelection(-1, false);
  expect(topSelected()).toEqual(["md-body"]);
  clearSelection();
  selectBlock("md-nested", { roots: ["md-comment", "md-nested"], collapsed: () => true, navOnly: true });
  moveSelection(-1, false);
  expect(topSelected()).toEqual(["md-comment"]);
});
