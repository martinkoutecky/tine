// The margin's "does this page hold a comment" question must follow the page's
// structure, never the text of other blocks: a keystroke anywhere would
// otherwise make the margin read every block (the GH #248 shape, caught by
// keystrokeScaling.guard). A comment arrives as a new child block.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { createMemo, createRoot, getOwner } from "solid-js";
import { initParser, blockRegions } from "./render/parse";
import { resetStore, setRaw, insertEmptyChildBlock, indentBlock } from "./document";
import { loadSingle } from "./document/workingSet";
import { pageByName } from "./document/model";
import { commentsWritten, pageHasComment, watchCommentsWritten } from "./margin";
import type { BlockDto } from "./types";

function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  const properties = blockRegions(raw, "md").properties.filter((p) => p.primary).map((p): [string, string] => [p.key, p.value]);
  return { id, raw, collapsed: false, children, ...(properties.length ? { properties } : {}) };
}

beforeAll(() => initParser());
afterEach(() => resetStore());

function watch(): { runs: () => number; has: () => boolean; dispose: () => void } {
  const page = pageByName("M")!;
  let runs = 0;
  let has!: () => boolean;
  const dispose = createRoot((d) => {
    has = createMemo(() => { runs++; watchCommentsWritten(); return pageHasComment(page.roots, "md"); });
    return d;
  });
  return { runs: () => runs, has: () => has(), dispose };
}

describe("pageHasComment is structural", () => {
  it("does not re-run when another block's text changes", () => {
    loadSingle({ name: "M", kind: "page", title: "M", pre_block: null, blocks: [block("a", "first", [block("b", "second"), block("c", "third")])] });
    const w = watch();
    expect(w.has()).toBe(false);
    const before = w.runs();
    setRaw("b", "second, edited");
    setRaw("c", "third, edited");
    expect(w.has()).toBe(false);
    expect(w.runs()).toBe(before);
    w.dispose();
  });

  it("finds a comment that arrives as a new child once it is announced", () => {
    loadSingle({ name: "M", kind: "page", title: "M", pre_block: null, blocks: [block("a", "first words"), block("b", "second")] });
    const w = watch();
    expect(w.has()).toBe(false);
    const child = insertEmptyChildBlock("a", 0)!;
    setRaw(child, "\nquote:: first");
    commentsWritten();
    expect(w.has()).toBe(true);
    w.dispose();
  });

  it("still finds a comment already on the page when it loads", () => {
    loadSingle({ name: "M", kind: "page", title: "M", pre_block: null, blocks: [block("a", "first words", [block("k", "\nquote:: first")])] });
    const w = watch();
    expect(w.has()).toBe(true);
    w.dispose();
  });
});

describe("pageHasComment at open (BIGLOAD-090b)", () => {
  it("subscribes to O(1) sources on a 2,000-block page, not one per block", () => {
    const blocks = Array.from({ length: 1000 }, (_, i) => block(`p${i}`, `parent ${i}`, [block(`c${i}`, `child ${i}`)]));
    loadSingle({ name: "M", kind: "page", title: "M", pre_block: null, blocks });
    const page = pageByName("M")!;
    let owner!: { sources: unknown[] | null };
    createRoot((dispose) => {
      const has = createMemo(() => {
        owner = getOwner() as unknown as { sources: unknown[] | null };
        watchCommentsWritten();
        return pageHasComment(page.roots, "md");
      });
      expect(has()).toBe(false);
      expect(owner.sources?.length ?? 0).toBeLessThan(10);
      dispose();
    });
  });

  it("re-asks when the page's structure changes", () => {
    loadSingle({ name: "M", kind: "page", title: "M", pre_block: null, blocks: [block("a", "first"), block("k", "\nquote:: first")] });
    const w = watch();
    expect(w.has()).toBe(false);
    indentBlock("k", 0);
    expect(w.has()).toBe(true);
    w.dispose();
  });
});
