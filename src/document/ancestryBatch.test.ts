import { afterEach, expect, it, vi } from "vitest";
import { batch, createComputed, createRoot, getOwner } from "solid-js";
import { loadFeed, resetStore } from "./workingSet";
import { depthOf, depthsOf } from "./tree";
import { isCommentId, threadRootOf, threadRootsOf } from "./commentThreads";
import { selectBlock, moveSelection, topSelected, clearSelection } from "./edits/selection";
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
