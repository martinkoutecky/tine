// CM step 2, census Q13: a published site answers names as the app does, by
// looking up the `answers` Rust wrote at publish time
// (docs/contracts/quick-share.md, "Published answers"). Each case reads a
// snapshot the static export wrote (`tests/fixtures/published-answers/`,
// pinned by `crates/tine-graph-features/tests/published_answers.rs`) through
// the same frontend path the app uses: `linkTarget` (PageRef), the router's
// `navigationTarget` for an ordinary-page route, then the backend's `getPage`.
// The D-numbers are the disagreements of the census map (cm-q13-published-map).
import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PublishedSnapshot } from "./publishedBackend";

const FIXTURES = new URL("../tests/fixtures/published-answers/", import.meta.url);
const fixture = (name: string): PublishedSnapshot =>
  JSON.parse(readFileSync(new URL(`${name}.json`, FIXTURES), "utf8")) as PublishedSnapshot;

const holder = vi.hoisted(() => ({ api: null as unknown }));
vi.mock("./backend", () => ({ backend: () => holder.api }));

async function load(snapshot: PublishedSnapshot) {
  await (await import("./render/parse")).initParser();
  const published = await import("./publishedBackend");
  vi.spyOn(published, "isPublishedExport").mockReturnValue(true);
  const { publishedBackend } = published;
  holder.api = publishedBackend(async () => snapshot);
  const api = holder.api as ReturnType<typeof publishedBackend>;
  const { setJournalFormats } = await import("./journal");
  const loaded = await api.loadGraph("");
  if (loaded.kind === "loaded") setJournalFormats(loaded.meta.journal_page_title_format, loaded.meta.journal_file_name_format);
  const index = await import("./pageIndex");
  index.installPageIndex();
  await vi.waitFor(() => expect(index.allPages()?.length).toBeGreaterThan(0));
  /** The page a click on `[[name]]` shows: PageRef, router, getPage. */
  const open = async (name: string) => {
    const link = index.linkTarget(name);
    const route = link.kind === "page" ? index.navigationTarget(link.name) : link;
    return api.getPage(route.name, route.kind);
  };
  return { api, open, ...index };
}

beforeEach(() => {
  vi.resetModules();
});

describe("published answers: links open what they open in the app", () => {
  it("new publications open an empty real file while frozen v1 answers keep their stored redirect", async () => {
    const { open } = await load(fixture("cases-real-file-first"));
    expect((await open("ML"))?.id).toBe("pages/ML.md");
  });
  it("D1: a link to an empty page named like an alias opens the alias owner", async () => {
    const { open } = await load(fixture("cases"));
    const page = await open("ML");
    expect(page?.id).toBe("pages/Machine Learning.md");
    expect(page?.blocks[0]?.raw).toBe("ML body");
  });

  it("D2: a date-spelled link opens the day's journal", async () => {
    const { api, open, linkTarget } = await load(fixture("cases"));
    expect(linkTarget("2026-10-26")).toEqual({ name: "Oct 26th, 2026", kind: "journal" });
    const page = await open("2026-10-26");
    expect(page?.id).toBe("journals/2026_10_26.md");
    expect(await api.resolvePage("2026-10-26", "journal")).toEqual({ kind: "existing", id: "journals/2026_10_26.md", others: [] });
  });

  it("D3: a namespace parent is a listed page", async () => {
    const { allPageNames, linkTarget, api } = await load(fixture("cases"));
    expect(allPageNames()).toContain("n");
    expect(linkTarget("n")).toEqual({ name: "n", kind: "page" });
    const rows = (await api.pageInventory()).entries.filter((row) => row.name === "n");
    expect(rows).toEqual([expect.objectContaining({ namespace_parent: true, target: { kind: "absent", id: "pages/n.md" } })]);
  });

  it("D4: an alias two pages declare opens the first owner", async () => {
    const { open } = await load(fixture("cases"));
    expect((await open("Shared"))?.id).toBe("pages/Alpha.md");
  });

  it("D4: a journal's alias opens the journal", async () => {
    const { open, linkTarget } = await load(fixture("cases"));
    expect(linkTarget("launch day")).toEqual({ name: "Oct 26th, 2026", kind: "journal" });
    expect((await open("launch day"))?.id).toBe("journals/2026_10_26.md");
  });

  it("D5: a page's header lists the pages that declare its name", async () => {
    const { api } = await load(fixture("cases"));
    expect(await api.pageAliasGroup("Group A")).toEqual({ aliases: [], pages: ["Group B"] });
    expect(await api.pageAliasGroup("Oct 26th, 2026")).toEqual({ aliases: ["launch day"], pages: [] });
  });

  it("D6: the backlinks of an alias are its owner's", async () => {
    const { api } = await load(fixture("cases"));
    const owner = await api.getBacklinks("Oct 26th, 2026");
    expect(owner.length).toBeGreaterThan(0);
    expect(await api.getBacklinks("launch day")).toEqual(owner);
  });

  it("D7b: an empty journal is not a day with content", async () => {
    const { api } = await load(fixture("cases"));
    expect(await api.journalContentDays()).toEqual([20261026]);
  });

  it("D9: the graph's journal formats reach the viewer's date parser", async () => {
    const { api, open, linkTarget } = await load(fixture("formats"));
    const loaded = await api.loadGraph("");
    expect(loaded.kind === "loaded" && loaded.meta.journal_page_title_format).toBe("dd.MM.yyyy");
    expect((await open("26.10.2026"))?.id).toBe("journals/2026_10_26.md");
    // A day no file holds still links as a journal under the configured format.
    expect(linkTarget("27.10.2026")).toEqual({ name: "27.10.2026", kind: "journal" });
  });
});

describe("published answers: permalinks open what the link opens, on open and refresh", () => {
  const cases: [string, string][] = [
    ["ML", "pages/Machine Learning.md"],
    ["Shared", "pages/Alpha.md"],
    ["2026-10-26", "journals/2026_10_26.md"],
    ["launch day", "journals/2026_10_26.md"],
  ];
  it.each(cases)("#/page/%s", async (name, path) => {
    const snapshot = fixture("cases");
    await load(snapshot);
    const { openPublishedPermalink, publishedPermalinkHash, publishedPermalinkForWorkspace } = await import("./publishedPermalink");
    const { createPaneRouter } = await import("./router");
    const router = createPaneRouter();
    const opened = openPublishedPermalink(snapshot, publishedPermalinkHash({ kind: "page", page: name }), router);
    expect(opened.status).toBe("opened");
    const route = router.route();
    expect(route).toMatchObject({ kind: "page", path });
    // Refresh: the address the workspace writes back opens the same page.
    const written = publishedPermalinkForWorkspace(1, 1, route);
    expect(written).toBeTruthy();
    const again = createPaneRouter();
    expect(openPublishedPermalink(snapshot, publishedPermalinkHash(written!), again).status).toBe("opened");
    expect(again.route()).toMatchObject({ kind: "page", path });
  });

  it("keeps the quick share prefix and opens a multi-owner alias", async () => {
    const snapshot = fixture("cases");
    await load(snapshot);
    const { openPublishedPermalink, setQuickSharePrefix } = await import("./publishedPermalink");
    const { createPaneRouter } = await import("./router");
    setQuickSharePrefix("tine-fixture/0123456789abcdef0123/key");
    try {
      const router = createPaneRouter();
      expect(openPublishedPermalink(snapshot, "#tine-fixture/0123456789abcdef0123/key/page/Shared", router).status).toBe("opened");
      expect(router.route()).toMatchObject({ kind: "page", name: "Alpha", path: "pages/Alpha.md" });
    } finally {
      setQuickSharePrefix(null);
    }
  });
});

describe("published answers: the static loader admits answers like a share", () => {
  it("refuses damaged recognized answers and reads absent or unknown versions as before", async () => {
    const { validateSnapshot } = await import("./publishedBackend");
    const { DamagedAnswersError } = await import("./publishedAnswers");
    const snapshot = fixture("cases");
    expect(() => validateSnapshot(snapshot)).not.toThrow();
    expect(() => validateSnapshot({ ...snapshot, answers: undefined })).not.toThrow();
    expect(() => validateSnapshot({ ...snapshot, answers: { v: 2 } })).not.toThrow();
    const damaged = structuredClone(snapshot) as PublishedSnapshot & { answers: { inventory: unknown[] } };
    damaged.answers.inventory.push({ key: "x", name: "x", is_journal: false, day: null, target: { kind: "existing", id: "pages/private.md", others: [] } });
    expect(() => validateSnapshot(damaged)).toThrow(DamagedAnswersError);
  });
});
