import { readFileSync } from "node:fs";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import type { BlockDto, PageDto } from "../types";
import { OUTLINE_MAX_DEPTH, OUTLINE_MAX_SOURCE_CHARS, outlineDepth, parseOutline, type OutlineNode } from "../editor/outline";
import { exportOutline, DEFAULT_EXPORT_OPTIONS } from "../editor/exportText";
import { exportHtml } from "../editor/exportHtml";
import { exportOpml } from "../editor/exportOpml";
import { initParser } from "../render/parse";
import { doc } from "./model";
import { pageToDto } from "./convert";
import { loadFeed, pageByName, insertOutlineAfter, insertOutlineChildren, insertEmptyChildBlock, replaceEmptyBlockWithOutline, replaceChildOrders, splitBlock, captureToPage, blockSubtreeMarkdown, buildClipboardPayload, exportNodesFor, indentBlock, moveBlock, resetStore } from ".";

// og 15b (I-22 / I-4): the frontend's one outline ceiling is the backend's
// admission cap. A benign page AT the cap loads, serializes, exports and
// re-pastes; every outline inserter refuses a result one level deeper, so the
// recursive consumers never see more than the cap.

beforeAll(() => initParser());
afterEach(() => {
  resetStore();
  vi.restoreAllMocks();
});

const id = (n: number) => `00000000-0000-4000-8000-${n.toString(16).padStart(12, "0")}`;

function deepDto(levels: number): BlockDto[] {
  let node: BlockDto = { id: id(levels), raw: `level ${levels} [[Tag]]`, collapsed: false, children: [] };
  for (let level = levels - 1; level >= 1; level--) {
    node = { id: id(level), raw: `level ${level}\nsecond line`, collapsed: false, children: [node] };
  }
  return [node];
}

function deepOutline(levels: number): OutlineNode[] {
  let node: OutlineNode = { raw: `leaf ${levels}`, children: [] };
  for (let level = levels - 1; level >= 1; level--) node = { raw: `node ${level}`, children: [node] };
  return [node];
}

const page = (name: string, blocks: BlockDto[]): PageDto => ({ name, kind: "page", title: name, pre_block: null, blocks, format: "md" });

function subtreeSize(): number {
  return Object.keys(doc.byId).length;
}

describe("outline depth cap", () => {
  it("matches the 128-level master ceiling", () => {
    expect(OUTLINE_MAX_DEPTH).toBe(128);
  });

  it("mirrors the Rust admission cap", async () => {
    const { readFileSync } = await import("node:fs");
    const rust = readFileSync(new URL("../../crates/tine-store/src/model.rs", import.meta.url), "utf8");
    expect(rust).toContain(`pub(crate) const PARSE_INPUT_MAX_DEPTH: usize = ${OUTLINE_MAX_DEPTH};`);
  });

  it("loads, serializes, exports and re-pastes a benign page exactly at the cap", () => {
    loadFeed([page("Deep", deepDto(OUTLINE_MAX_DEPTH)), page("Target", [{ id: id(9000), raw: "anchor", collapsed: false, children: [] }])]);
    const dto = pageToDto("Deep")!;
    let depth = 0;
    for (let level: BlockDto[] = dto.blocks; level.length; level = level[0].children) depth++;
    expect(depth).toBe(OUTLINE_MAX_DEPTH);

    const root = pageByName("Deep")!.roots[0];
    const nodes = exportNodesFor([root]);
    expect(exportOutline(nodes, DEFAULT_EXPORT_OPTIONS)).toContain(`level ${OUTLINE_MAX_DEPTH}`);
    expect(exportHtml(nodes, { stripLinks: false, removeEmphasis: false, removeTags: false })).toContain(`level ${OUTLINE_MAX_DEPTH}`);
    expect(exportOpml(nodes, { stripLinks: false, removeEmphasis: false, removeTags: false })).toContain(`level ${OUTLINE_MAX_DEPTH}`);
    expect(buildClipboardPayload([root])).not.toBeNull();

    const markdown = blockSubtreeMarkdown(root);
    const reparsed = parseOutline(markdown);
    expect(outlineDepth(reparsed)).toBe(OUTLINE_MAX_DEPTH);
    const anchor = pageByName("Target")!.roots[0];
    expect(insertOutlineAfter(anchor, reparsed)).not.toBe(anchor);
    const pasted = pageToDto("Target")!.blocks[1];
    let pastedDepth = 0;
    for (let level: BlockDto[] = [pasted]; level.length; level = level[0].children) pastedDepth++;
    expect(pastedDepth).toBe(OUTLINE_MAX_DEPTH);
  });

  it("every inserter refuses a result one level past the cap and accepts one at it", () => {
    loadFeed([page("Host", [{ id: id(1), raw: "host", collapsed: false, children: [] }, { id: id(2), raw: "", collapsed: false, children: [] }])]);
    const [host, empty] = pageByName("Host")!.roots;
    const before = subtreeSize();

    // Children of a root land at depth 2.
    expect(insertOutlineChildren(host, deepOutline(OUTLINE_MAX_DEPTH))).toBeNull();
    expect(insertOutlineAfter(host, deepOutline(OUTLINE_MAX_DEPTH + 1))).toBeNull();
    expect(replaceEmptyBlockWithOutline(empty, deepOutline(OUTLINE_MAX_DEPTH + 1))).toBeNull();
    expect(subtreeSize()).toBe(before);

    expect(insertOutlineChildren(host, deepOutline(OUTLINE_MAX_DEPTH - 1))).not.toBeNull();
    replaceEmptyBlockWithOutline(empty, deepOutline(OUTLINE_MAX_DEPTH));
    expect(doc.byId[empty].raw).toBe("node 1");
    expect(doc.byId[empty].children).toHaveLength(1);
  });

  it("a hostile, very deep pasted outline is refused without overflowing the stack", () => {
    loadFeed([page("Host", [{ id: id(1), raw: "host", collapsed: false, children: [] }])]);
    const host = pageByName("Host")!.roots[0];
    const before = subtreeSize();
    const nodes = deepOutline(50_000);
    expect(() => insertOutlineAfter(host, nodes)).not.toThrow();
    expect(insertOutlineChildren(host, nodes)).toBeNull();
    expect(subtreeSize()).toBe(before);
  });

  it("indent and reparent refuse a subtree that would cross the cap", async () => {
    loadFeed([page("Deep", [
      { id: id(900), raw: "host", collapsed: false, children: [] },
      ...deepDto(OUTLINE_MAX_DEPTH),
    ])]);
    const [host, deep] = pageByName("Deep")!.roots;
    const before = pageToDto("Deep");
    expect(indentBlock(deep, 0)).toBe(false);
    expect(await moveBlock(deep, host, 0)).toBe(false);
    expect(pageToDto("Deep")).toEqual(before);
  });

  it("child creation and sheet reparenting refuse a 129th outline level", () => {
    loadFeed([page("Deep", [
      ...deepDto(OUTLINE_MAX_DEPTH),
      { id: id(900), raw: "spare", collapsed: false, children: [] },
    ])]);
    const leaf = id(OUTLINE_MAX_DEPTH);
    const before = pageToDto("Deep");
    expect(insertEmptyChildBlock(leaf, 0)).toBeNull();
    expect(splitBlock(leaf, 0, true, true)).toBe(false);
    expect(replaceChildOrders({ [leaf]: [id(900)] })).toBe(false);
    expect(pageToDto("Deep")).toEqual(before);
  });

  it("parses no outline from text past the source ceiling", () => {
    expect(parseOutline("- a\n- b".padEnd(OUTLINE_MAX_SOURCE_CHARS + 1, "x"))).toEqual([]);
    expect(parseOutline("- a\n- b")).toHaveLength(2);
  });

  it("quick capture refuses an over-deep outline instead of reporting a capture", async () => {
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    const text = Array.from({ length: OUTLINE_MAX_DEPTH + 1 }, (_, level) => `${"  ".repeat(level)}- n${level}`).join("\n");
    expect(await captureToPage("Captured", text)).toBe(false);
    expect(pageByName("Captured")?.roots.length ?? 0).toBe(0);
  });
});

// C5 I-12 twin pin: the TypeScript ceiling and the Rust admission cap are one number. A guard, not a
// shared constant, because the wasm crossing per insert is too hot; this fails on drift in either file.
describe("outline depth ceiling twin", () => {
  it("OUTLINE_MAX_DEPTH equals tine-store PARSE_INPUT_MAX_DEPTH (crates/tine-store/src/model.rs)", () => {
    const rust = readFileSync("crates/tine-store/src/model.rs", "utf8");
    const match = /const PARSE_INPUT_MAX_DEPTH: usize = (\d+);/.exec(rust);
    expect(match, "I-12: the Rust cap constant moved; update this pin and OUTLINE_MAX_DEPTH together").not.toBeNull();
    expect(OUTLINE_MAX_DEPTH).toBe(Number(match![1]));
  });
});
