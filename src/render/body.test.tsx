import { describe, it, expect, beforeAll } from "vitest";
import { render } from "solid-js/web";
import { AstBody, renderBlocks, estimateBodyReserve } from "./body";
import { initParser, parseBlock } from "./parse";
import { renderedBlocks } from "../lazyObserve";

// AstBody gates the parse + AST→DOM build behind a "near the viewport"
// IntersectionObserver (P1 block-render virtualization). jsdom has no
// IntersectionObserver, so observeNear fires synchronously and AstBody renders
// EAGERLY — i.e. these tests exercise the no-IO degrade path, which must produce
// exactly today's output (the gate is WHEN we parse, not WHAT). The real
// deferred-placeholder / scroll behavior is a browser concern, verified in the
// Playwright harness (scripts/e2e-virtualize.mjs), not here.
beforeAll(async () => {
  await initParser();
});

async function htmlOf(el: () => unknown): Promise<string> {
  const div = document.createElement("div");
  const dispose = render(() => el() as never, div);
  // Let the ref→observeNear(synchronous fire)→setNear(true) swap flush.
  await Promise.resolve();
  const out = div.innerHTML;
  dispose();
  return out;
}

describe("AstBody (no-IO degrade path)", () => {
  it("renders the parsed body, not a deferred placeholder", async () => {
    const h = await htmlOf(() => AstBody({ raw: "a **bold** b", blockId: "blk-1" }));
    expect(h).toContain("<strong>");
    expect(h).not.toContain("ast-deferred");
  });

  it("output equals direct renderBlocks(parseBlock(...)) — gate is WHEN not WHAT", async () => {
    // AstBody parses the WHOLE block raw; `parseBlock` re-bullets internally (wasm
    // parse_block_json), so for property/planning-free bodies it equals a direct call.
    for (const text of ["plain text", "a **b** _c_ `d`", "| A | B |\n| --- | --- |\n| 1 | 2 |", "$$E=mc^2$$"]) {
      const viaAst = await htmlOf(() => AstBody({ raw: text, blockId: "x" }));
      const direct = await htmlOf(() => renderBlocks(parseBlock(text, false), "x", null));
      expect(viaAst).toBe(direct);
    }
  });

  it("latches the block id after it has rendered once", async () => {
    expect(renderedBlocks.has("blk-latch")).toBe(false);
    await htmlOf(() => AstBody({ raw: "hello", blockId: "blk-latch" }));
    expect(renderedBlocks.has("blk-latch")).toBe(true);
  });
});

describe("estimateBodyReserve (placeholder height proxy)", () => {
  it("reserves a min-height for headings (larger when shallower)", () => {
    const h1 = estimateBodyReserve(["Title"], 1)?.["min-height"];
    const h6 = estimateBodyReserve(["Title"], 6)?.["min-height"];
    expect(h1).toBeTruthy();
    expect(h6).toBeTruthy();
    expect(parseFloat(h1 as string)).toBeGreaterThan(parseFloat(h6 as string));
  });

  it("reserves for display math and media embeds", () => {
    expect(estimateBodyReserve(["$$x^2$$"], null)?.["min-height"]).toBe("2.4em");
    expect(estimateBodyReserve(["![](a.png)"], null)?.["min-height"]).toBe("6em");
  });

  it("reserves nothing for ordinary prose / code / tables", () => {
    expect(estimateBodyReserve(["just a paragraph"], null)).toBeUndefined();
    expect(estimateBodyReserve(["```js", "x", "```"], null)).toBeUndefined();
    expect(estimateBodyReserve(["| a | b |"], null)).toBeUndefined();
  });
});
