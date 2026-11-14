import { afterEach, expect, it } from "vitest";
import { batch } from "solid-js";
import { loadFeed, resetStore } from "./workingSet";
import { depthOf, depthsOf } from "./tree";
import { threadRootOf, threadRootsOf } from "./commentThreads";
import { selectBlock, moveSelection, topSelected, clearSelection } from "./edits/selection";
import { setDoc } from "./model";
import { blockRegions } from "../render/parse";
import type { BlockDto, Format } from "../types";

afterEach(resetStore);

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
