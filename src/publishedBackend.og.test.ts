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
