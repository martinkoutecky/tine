import { describe, expect, it } from "vitest";
import { publishedBackend, validateSnapshot, type PublishedSnapshot } from "./publishedBackend";
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
    await expect(api.savePages([])).rejects.toThrow("read-only published export");
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
