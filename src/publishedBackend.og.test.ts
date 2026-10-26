import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import {
  __resetPublishedSnapshotForTest, loadPublishedSnapshot, publishedBackend, readBounded, validateSnapshot,
  type PublishedSnapshot,
} from "./publishedBackend";
import { initParser } from "./render/parse";
import { openPublishedPermalink, parsePublishedPermalinkHash, publishedPermalinkHash } from "./publishedPermalink";
import type { ParsedQuery, QueryResult } from "./editor/queryIr";

const parsed = {
  query: { anchor: "block", source: { kind: "og", original: "(task TODO)" }, filter: { kind: "true" } },
  view: {},
} as unknown as ParsedQuery;
const result = {
  anchor: "block", groups: [{ page: "Public", kind: "page", blocks: [{ id: "one", raw: "TODO one", collapsed: false, children: [] }] }],
  diagnostics: [], report: { supported: true, ran: [], ignored: [] }, total: 1, matched_total: 1, exceeded: false,
} as QueryResult;

const snapshot: PublishedSnapshot = {
  schema: 1, name: "Example", exported_at: "", home: "Public",
  pages: [{ path: "pages/Public.md", name: "Public", title: "Public", kind: "page", pre_block: null,
    blocks: [{ id: "one", raw: "TODO one", collapsed: false, children: [] }], read_only: true }],
  entries: [{ name: "Public", kind: "page", date_key: null, path: "pages/Public.md" }],
  backlinks: {}, block_ref_counts: {}, aliases: [["Alias", "Public"]], icons: {},
  queries: [{ host: "Public", argument: "(task TODO)", dialect: "macro_query", properties: [], parsed,
    context: { current_page: "Public" }, executed_context: { current_page: "Public" }, view: {}, result }],
};

describe("read-only published snapshot", () => {
  it("answers page and baked query reads from the closed snapshot", async () => {
    validateSnapshot(snapshot);
    const api = publishedBackend(async () => snapshot);
    expect(api.graphBindingGeneration()).toBe(1);
    const inventory = await api.pageInventory();
    expect(BigInt(inventory.rev)).toBe(0n);
    expect(inventory.entries.map((entry) => entry.name)).toEqual(["Public"]);
    expect((await api.getPage("Alias", "page"))?.id).toBe("pages/Public.md");
    expect(await api.parseQuery("(task TODO)", "macro_query")).toEqual(parsed);
    expect(await api.queryRun(parsed.query, {}, { current_page: "Public" })).toEqual(result);
    await expect(api.parseQuery("(task DONE)", "macro_query")).rejects.toMatchObject({ reasonCode: "published_export_static" });
  });

  it("refuses a write while keeping public permalink identity stable", async () => {
    const api = publishedBackend(async () => snapshot);
    const dto = { name: "Public", kind: "page" as const, title: "Public", pre_block: null, blocks: [] };
    await expect(api.pageSubmit(1, 1, "pages/Public.md", dto, 1, null, ["replace-page"])).rejects.toThrow("read-only published export");
    await expect(api.pageMove(1, 2, ["pages/Public.md", dto, 1], ["pages/Other.md", dto, 1], ["move-blocks"])).rejects.toThrow("read-only published export");
    await expect(api.pageDelete(1, "Public", "page")).rejects.toThrow("read-only published export");
    const page = publishedPermalinkHash({ kind: "page", page: "Public" });
    const block = publishedPermalinkHash({ kind: "block", block: "one" });
    expect(parsePublishedPermalinkHash(page)).toEqual({ kind: "page", page: "Public" });
    expect(parsePublishedPermalinkHash(block)).toEqual({ kind: "block", block: "one" });
    expect(openPublishedPermalink).toBeTypeOf("function");
  });
});


describe("published semantic answers and admission (OG-B-FRONT)", () => {
  it("resolves NFC and boundary slash page identities", async () => {
    const s = structuredClone(snapshot); s.pages[0].name = "Cafe\u0301";
    const api = publishedBackend(async () => s);
    expect((await api.getPage("/Café/", "page"))?.name).toBe("Cafe\u0301");
  });
  it("maps folded evidence back to authored UTF-16 text", async () => {
    const s = structuredClone(snapshot); s.pages[0].blocks[0].raw = "a\u0301b";
    const hits = await publishedBackend(async () => s).runGraphSearch("b", 0, 10, "quick-switch");
    expect(hits.hits[0].evidence?.[0].spans).toEqual([{start: 2, end: 3}]);
  });
  it("resolves the authored ID instead of a fenced example", async () => {
    const s = structuredClone(snapshot);
    s.pages[0].blocks = [
      {id: "example", raw: "```\nid:: wanted\n```", collapsed: false, children: []},
      {id: "runtime", raw: "Real\nid:: wanted", collapsed: false, children: []},
    ];
    const api = publishedBackend(async () => s);
    expect((await api.resolveBlock("wanted"))?.blocks[0].id).toBe("runtime");
    expect((await api.previewBlock("wanted", 10))?.group.blocks[0].id).toBe("runtime");
  });
  it("refuses hostile served depth but accepts a broad ordinary export", () => {
    const s = structuredClone(snapshot); let child = s.pages[0].blocks[0];
    for (let i = 0; i < 2000; i++) {
      const next = {id: String(i), raw: "x", collapsed: false, children: []};
      child.children.push(next); child = next;
    }
    expect(() => validateSnapshot(s)).toThrow(/depth/);
    const broad = structuredClone(snapshot);
    broad.pages[0].blocks = Array.from({length: 20000}, (_, i) => ({id: String(i), raw: "x", collapsed: false, children: []}));
    expect(() => validateSnapshot(broad)).not.toThrow();
  });
});


it("accepts the native maximum block depth and previews it with a node budget", async () => {
  const s = structuredClone(snapshot); let child = s.pages[0].blocks[0];
  for (let i = 1; i < 128; i++) {
    const next = {id: String(i), raw: "x", collapsed: false, children: []};
    child.children.push(next); child = next;
  }
  expect(() => validateSnapshot(s)).not.toThrow();
  const preview = await publishedBackend(async () => s).previewBlock("one", 1);
  expect(preview?.group.blocks[0].children).toEqual([]);
  expect(preview?.truncated).toBe(127);
});


it("matches baked query contexts by canonical page identity", async () => {
  const s = structuredClone(snapshot); s.queries[0].context.current_page = "Cafe\u0301";
  const api = publishedBackend(async () => s);
  expect(await api.queryRun(parsed.query, {}, {current_page: "/Café/"})).toEqual(result);
});


it("bounds preview cloning before allocation and owns emitted metadata (OG-DUPF03)", async () => {
  const s = structuredClone(snapshot);
  const root = s.pages[0].blocks[0];
  root.tags = ["tag"]; root.properties = [["key", "value"]]; root.breadcrumb = ["ancestor"];
  root.children = Array.from({length: 20000}, (_, i) => ({id: `child-${i}`, raw: "x".repeat(1024), collapsed: false, children: []}));
  const clone = vi.spyOn(globalThis, "structuredClone");
  try {
    const start = performance.now();
    const preview = await publishedBackend(async () => s).previewBlock("one", 1);
    console.log(`OG-DUPF03 broad preview: ${(performance.now() - start).toFixed(2)} ms`);
    expect(preview?.truncated).toBe(20000);
    const copied = preview!.group.blocks[0];
    expect(copied.children).toEqual([]);
    const clonedDescendants = clone.mock.calls.reduce((n, [value]) => n + (value as {children?: unknown[]}).children!.length, 0);
    expect(clonedDescendants, "preview allocation must be bounded before cloning").toBe(0);
    copied.tags![0] = "changed"; copied.properties![0][1] = "changed"; copied.breadcrumb![0] = "changed";
    expect(root.tags).toEqual(["tag"]); expect(root.properties).toEqual([["key", "value"]]); expect(root.breadcrumb).toEqual(["ancestor"]);
  } finally { clone.mockRestore(); }
});


describe("published reads are parser-decided, visible on failure and bounded (OG-R)", () => {
  beforeAll(initParser);
  afterEach(() => {
    vi.unstubAllGlobals();
    __resetPublishedSnapshotForTest();
  });
  const bytesOf = (n: number) => new Uint8Array(n).fill(7);
  const TEST_ASSET_MAX_BYTES = 10;
  const TOO_SMALL_MAX_BYTES = TEST_ASSET_MAX_BYTES - 1;
  const respond = (body: Uint8Array, init: ResponseInit = {}) => new Response(body as unknown as BodyInit, init);

  it("lists only real ((uuid)) references as referrers, never a code lookalike", async () => {
    const id = "11111111-1111-4111-8111-111111111111";
    const s = structuredClone(snapshot);
    s.pages[0].blocks = [
      { id: "real", raw: `see ((${id})) here`, collapsed: false, children: [] },
      { id: "code", raw: `inline \`((${id}))\` only`, collapsed: false, children: [] },
      { id: "fence", raw: `\`\`\`\n((${id}))\n\`\`\``, collapsed: false, children: [] },
      { id: "other", raw: "no reference", collapsed: false, children: [] },
    ];
    const groups = await publishedBackend(async () => s).getBlockReferrers(id);
    expect(groups.flatMap((group) => group.blocks.map((block) => block.id))).toEqual(["real"]);
  });

  it("rejects a missing or refused asset instead of answering empty bytes", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response("nope", { status: 404 })));
    const api = publishedBackend(async () => snapshot);
    await expect(api.readAsset("gone.png", TEST_ASSET_MAX_BYTES)).rejects.toThrow(/gone\.png.*404/);
    await expect(api.readAsset("../escape.png", TEST_ASSET_MAX_BYTES)).rejects.toThrow(/outside|not a file/);
    await expect(api.streamAsset("/etc/passwd")).rejects.toThrow(/not a file/);
  });

  it("honours the caller's byte cap on an asset, by declared length and by streamed length", async () => {
    const api = publishedBackend(async () => snapshot);
    vi.stubGlobal("fetch", vi.fn(async () => respond(bytesOf(10), { headers: { "content-length": "10" } })));
    expect((await api.readAsset("ok.bin", TEST_ASSET_MAX_BYTES)).byteLength).toBe(10);
    await expect(api.readAsset("big.bin", TOO_SMALL_MAX_BYTES)).rejects.toThrow(/larger than 9/);
    // No declared length: the running total refuses and cancels the stream.
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        for (let i = 0; i < 5; i++) controller.enqueue(bytesOf(4));
        controller.close();
      },
    });
    await expect(readBounded(new Response(stream), 9, "asset s")).rejects.toThrow(/larger than 9/);
  });

  it("refuses an oversized snapshot document before parsing it", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => respond(new TextEncoder().encode("{}"), { headers: { "content-length": String(1 << 30) } })));
    await expect(loadPublishedSnapshot("snapshot.json")).rejects.toThrow(/larger than/);
  });
});
