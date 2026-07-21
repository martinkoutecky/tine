import { afterEach, describe, expect, it } from "vitest";
import {
  buildClipboardPayload,
  ensurePageLoaded,
  forgetPage,
  loadSingle,
  pageInstanceGeneration,
  reloadPage,
  resetStore,
} from "./store";
import type { BlockDto, PageDto } from "./types";

const page = (name: string, blocks: BlockDto[], path = `pages/${name}.md`): PageDto => ({
  name, kind: "page", title: name, pre_block: null, blocks, format: "md", path,
});

afterEach(() => resetStore());

describe("clipboard payload builder", () => {
  it("retains exact raws and full descendants independently of public copy shaping", () => {
    loadSingle(page("Page", [{
      id: "parent",
      raw: "Parent\ncollapsed:: true\nid:: 11111111-1111-1111-1111-111111111111",
      collapsed: true,
      children: [{
        id: "child",
        raw: "Child\nhidden:: yes\nid:: 22222222-2222-2222-2222-222222222222",
        collapsed: false,
        children: [],
      }],
    }]));

    expect(buildClipboardPayload(["parent"])).toMatchObject({
      blocks: [{
        raw: "Parent\ncollapsed:: true\nid:: 11111111-1111-1111-1111-111111111111",
        sourceFormat: "md",
        children: [{ raw: "Child\nhidden:: yes\nid:: 22222222-2222-2222-2222-222222222222" }],
      }],
      sourcePages: [{ name: "Page", path: "pages/Page.md", generation: expect.any(Number) }],
    });
  });

  it("rejects a forest over 10,000 blocks", () => {
    const blocks = Array.from({ length: 10_001 }, (_, i): BlockDto => ({
      id: `b-${i}`, raw: `block ${i}`, collapsed: false, children: [],
    }));
    loadSingle(page("Large", blocks));
    expect(buildClipboardPayload(blocks.map((block) => block.id))).toBeNull();
  });

  it("rejects raw content over 4 MiB", () => {
    loadSingle(page("Large", [{ id: "huge", raw: "x".repeat(4 * 1024 * 1024 + 1), collapsed: false, children: [] }]));
    expect(buildClipboardPayload(["huge"])).toBeNull();
  });
});

describe("page-instance generations", () => {
  it("changes across reload/rebind/forget and never reuses a retired instance", () => {
    loadSingle(page("Page", [{ id: "a", raw: "a", collapsed: false, children: [] }]));
    const loaded = pageInstanceGeneration("Page")!;
    reloadPage(page("Page", [{ id: "a", raw: "changed", collapsed: false, children: [] }]));
    const reloaded = pageInstanceGeneration("Page")!;
    ensurePageLoaded(page("Page", [{ id: "b", raw: "rebound", collapsed: false, children: [] }], "pages/other.md"));
    const rebound = pageInstanceGeneration("Page")!;
    forgetPage("Page");
    expect(pageInstanceGeneration("Page")).toBeNull();
    ensurePageLoaded(page("Page", [{ id: "c", raw: "new", collapsed: false, children: [] }]));
    const recreated = pageInstanceGeneration("Page")!;

    expect(reloaded).toBeGreaterThan(loaded);
    expect(rebound).toBeGreaterThan(reloaded);
    expect(recreated).toBeGreaterThan(rebound);
  });

  it("retires an evicted page generation", () => {
    loadSingle(page("Main", [{ id: "main", raw: "main", collapsed: false, children: [] }]));
    ensurePageLoaded(page("P0", [{ id: "p0", raw: "zero", collapsed: false, children: [] }]));
    const original = pageInstanceGeneration("P0")!;
    for (let i = 1; i <= 80; i++) {
      ensurePageLoaded(page(`P${i}`, [{ id: `p${i}`, raw: String(i), collapsed: false, children: [] }]));
    }
    expect(pageInstanceGeneration("P0")).toBeNull();
    ensurePageLoaded(page("P0", [{ id: "p0-new", raw: "new", collapsed: false, children: [] }]));
    expect(pageInstanceGeneration("P0")!).toBeGreaterThan(original);
  });
});
