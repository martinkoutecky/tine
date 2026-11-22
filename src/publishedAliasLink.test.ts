import { beforeEach, describe, expect, it, vi } from "vitest";


const files = {
  "pages/Public.md": "alias:: Alias\n\n- Owner body\n- see [[launch day]]\n",
  "journals/2026_10_26.md": "alias:: launch day\n\n- Launch journal body\n",
};

const holder = vi.hoisted(() => ({ api: null as unknown }));
vi.mock("./backend", () => ({ backend: () => holder.api }));

async function load(source: Record<string, string> = files) {
  await (await import("./render/parse")).initParser();
  const { testPublishedBackend } = await import("./publishedBackend.methods.test.support");
  const api = testPublishedBackend(source);
  holder.api = api;
  const index = await import("./pageIndex");
  const { setJournalFormats } = await import("./journal");
  const loaded = await api.loadGraph("");
  if (loaded.kind !== "loaded") throw new Error("Publication did not open");
  setJournalFormats(loaded.meta.journal_page_title_format, loaded.meta.journal_file_name_format);
  index.installPageIndex();
  await vi.waitFor(() => expect(index.allPageNames()).toContain("Public"));
  const open = (name: string) => {
    const link = index.linkTarget(name);
    const route = link.kind === "page" ? index.navigationTarget(link.name) : link;
    return api.getPage(route.name, route.kind);
  };
  return { api, open, ...index };
}

beforeEach(() => {
  vi.resetModules();
});

describe("published site: alias links use the core inventory", () => {
  it("opens the owner of an ordinary page's alias, as on develop before CM step 2", async () => {
    const { open, linkTarget } = await load();
    expect(linkTarget("Alias").kind).toBe("page");
    const page = await open("Alias");
    expect(page?.id).toBe("pages/Public.md");
    expect(page?.blocks[0]?.raw).toBe("Owner body");
  });

  it("opens the journal of a journal's alias on a published site", async () => {
    const { api, open, linkTarget } = await load();
    const link = linkTarget("launch day");
    expect(link).toEqual({ name: "Oct 26th, 2026", kind: "journal" });
    const page = await open("launch day");
    expect(page?.id).toBe("journals/2026_10_26.md");
    expect(page?.blocks[0]?.raw).toBe("Launch journal body");
    // The route names the owner, so its linked references are the journal's own.
    expect((await api.getBacklinks(link.name)).map((group) => group.page)).toEqual(["Public"]);
  });

  it("opens the first owner of an alias declared by two pages", async () => {
    const { open } = await load({
      ...files,
      "pages/Public.md": "alias:: Alias, Shared\n\n- Owner body\n",
      "pages/Z.md": "alias:: Shared\n\n- Second owner\n",
    });
    expect(await open("Shared")).toMatchObject({ id: "pages/Public.md", blocks: [{ raw: "Owner body" }] });
  });

  it("answers the related pages shown for a published page and journal", async () => {
    const { api } = await load({ ...files, "pages/Group.md": "alias:: Public\n\n- Group body\n" });
    expect(await api.pageAliasGroup("Public")).toEqual({ aliases: ["Alias"], pages: ["Group"] });
    expect(await api.pageAliasGroup("Oct 26th, 2026")).toEqual({ aliases: ["launch day"], pages: [] });
  });

  it("opens date-spelled links in the published graph's journal formats", async () => {
    const { api, open, linkTarget } = await load({
      "logseq/config.edn": '{:journal/page-title-format "dd.MM.yyyy" :journal/file-name-format "yyyy-dd-MM"}',
      "pages/Public.md": "- [[2026-10-26]] [[26.10.2026]]\n",
      "journals/2026-26-10.md": "- Launch journal body\n",
    });
    for (const spelling of ["2026-10-26", "26.10.2026"]) {
      expect(linkTarget(spelling)).toEqual({ name: "26.10.2026", kind: "journal" });
      expect(await open(spelling)).toMatchObject({ id: "journals/2026-26-10.md", title: "26.10.2026" });
    }
    expect(linkTarget("27.10.2026")).toEqual({ name: "27.10.2026", kind: "journal" });
    expect(await api.resolvePage("27.10.2026", "journal")).toEqual({ kind: "absent", id: "journals/2026-27-10.md" });
  });
});
