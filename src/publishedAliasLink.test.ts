// CM step 2 (Q8): the published site has no inventory `link` field (quick
// share generation 1 is frozen; docs/storage-contract.md). A rendered
// `[[alias]]` of an ordinary page must still open the owner there, through the
// same frontend path the app uses: `linkTarget` (PageRef), then the router's
// `navigationTarget` for an ordinary-page route, then the backend `getPage`.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PublishedSnapshot } from "./publishedBackend";

const snapshot: PublishedSnapshot = {
  schema: 1, name: "Example", exported_at: "", home: "Public",
  pages: [
    { path: "pages/Public.md", name: "Public", title: "Public", kind: "page", pre_block: null,
      blocks: [{ id: "one", raw: "Owner body", collapsed: false, children: [] }], read_only: true },
    { path: "journals/2026_10_26.md", name: "Oct 26th, 2026", title: "Oct 26th, 2026", kind: "journal", pre_block: null,
      blocks: [{ id: "two", raw: "Launch journal body", collapsed: false, children: [] }], read_only: true },
  ],
  entries: [
    { name: "Public", kind: "page", date_key: null, path: "pages/Public.md" },
    { name: "Oct 26th, 2026", kind: "journal", date_key: 20261026, path: "journals/2026_10_26.md" },
  ],
  backlinks: {}, block_ref_counts: {}, aliases: [["Alias", "Public"], ["launch day", "Oct 26th, 2026"]], icons: {},
  queries: [],
};

const holder = vi.hoisted(() => ({ api: null as unknown }));
vi.mock("./backend", () => ({ backend: () => holder.api }));

async function load() {
  await (await import("./render/parse")).initParser();
  const { publishedBackend } = await import("./publishedBackend");
  holder.api = publishedBackend(async () => snapshot);
  const index = await import("./pageIndex");
  return { api: holder.api as ReturnType<typeof publishedBackend>, ...index };
}

beforeEach(() => {
  vi.resetModules();
});

describe("published site: alias links without the inventory link field", () => {
  it("opens the owner of an ordinary page's alias, as on develop before CM step 2", async () => {
    const { api, installPageIndex, allPageNames, linkTarget, navigationTarget } = await load();
    installPageIndex();
    await vi.waitFor(() => expect(allPageNames()).toContain("Public"));
    // PageRef: what the link opens.
    const link = linkTarget("Alias");
    expect(link.kind).toBe("page");
    // Router: an ordinary-page route without a path goes through navigationTarget.
    const route = link.kind === "page" ? navigationTarget(link.name) : link;
    const page = await api.getPage(route.name, route.kind);
    expect(page?.id).toBe("pages/Public.md");
    expect(page?.blocks[0]?.raw).toBe("Owner body");
  });
});
