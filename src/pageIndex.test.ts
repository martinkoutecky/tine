import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PageEntry, PageInventory, PageInventoryEntry, ResolvedPage } from "./types";

const backendMock = vi.hoisted(() => ({ pageInventory: vi.fn() }));
vi.mock("./backend", () => ({ backend: () => backendMock }));

function entry(name: string, target: ResolvedPage, key = name.toLowerCase().normalize("NFC")): PageInventoryEntry {
  return { key, name, is_journal: false, day: null, target };
}
const file = (name: string, id = `pages/${name.replaceAll("/", "___")}.md`, others: string[] = []) =>
  entry(name, { kind: "existing", id, others });
const alias = (name: string, ...owners: string[]) => entry(name, { kind: "alias", owners });
const absent = (name: string) => entry(name, { kind: "absent", id: `pages/${name.replaceAll("/", "___")}.md` });
const inventory = (rev: number, ...entries: PageInventoryEntry[]): PageInventory => ({ rev: String(rev), entries });
const page = (name: string): PageEntry => ({
  name,
  kind: "page",
  date_key: null,
  path: `pages/${name.replaceAll("/", "___")}.md`,
});

async function load() {
  const ui = await import("./ui");
  const index = await import("./pageIndex");
  const pages = await import("./pages");
  return { ...ui, ...index, ...pages };
}

beforeEach(() => {
  vi.resetModules();
  backendMock.pageInventory.mockReset();
});

describe("page index: the one frontend name answerer", () => {
  // Ported from graph.test "loads real page identities once and lets them win
  // colliding aliases": the backend's target is returned verbatim, and each
  // trigger costs one page_inventory IPC (bind, content save, create/delete).
  it("answers from the backend target and refetches once per trigger", async () => {
    backendMock.pageInventory.mockResolvedValue(inventory(1,
      file("page1"),
      alias("shortcut", "pages/other.md"),
      file("other"),
    ));
    const { installPageIndex, resolvedTarget, navigationName, bumpDataRev, bumpPageInventoryRev } = await load();
    installPageIndex();
    await vi.waitFor(() => expect(navigationName("Shortcut")).toBe("other"));
    expect(backendMock.pageInventory).toHaveBeenCalledTimes(1);
    expect(resolvedTarget("PAGE1")).toEqual({ kind: "existing", id: "pages/page1.md", others: [] });
    expect(navigationName("PAGE1")).toBe("page1");
    expect(navigationName("Unknown")).toBe("Unknown");

    bumpDataRev();
    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(2));
    bumpPageInventoryRev();
    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(3));
    // A save that bumps both in one tick costs one IPC, not two.
    bumpDataRev();
    bumpPageInventoryRev();
    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(4));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(backendMock.pageInventory).toHaveBeenCalledTimes(4);
  });

  // Ported from graph.test "refreshes real-page precedence after a same-session
  // page creation".
  it("follows a same-session page creation that takes over an alias name", async () => {
    backendMock.pageInventory.mockResolvedValue(inventory(1,
      alias("new page", "pages/Alias target.md"),
      file("Alias target"),
      file("page1"),
    ));
    const { installPageIndex, navigationName, bumpPageInventoryRev } = await load();
    installPageIndex();
    await vi.waitFor(() => expect(navigationName("New Page")).toBe("Alias target"));

    backendMock.pageInventory.mockResolvedValue(inventory(2,
      file("Alias target"),
      file("New Page", "pages/New Page.md"),
      file("page1"),
    ));
    bumpPageInventoryRev();
    await vi.waitFor(() => expect(navigationName("new page")).toBe("New Page"));
  });

  // Ported from graph.test "folds NFD alias keys before real-page precedence is
  // applied": the lookup folds with pageIdentityKey (NFC, case) onto the
  // backend key; which target wins is the backend's decision.
  it("folds NFD and case onto the backend key", async () => {
    backendMock.pageInventory.mockResolvedValue(inventory(1,
      entry("Café", { kind: "existing", id: "pages/Café.md", others: [] }, "café"),
      entry("ΟΣ", { kind: "alias", owners: ["pages/Owner.md"] }, "ος"),
      file("Owner"),
    ));
    const { installPageIndex, navigationName } = await load();
    installPageIndex();
    await vi.waitFor(() => expect(navigationName("CAFE\u{301}")).toBe("Café"));
    expect(navigationName(" ΟΣ ")).toBe("Owner");
    expect(navigationName("/café/")).toBe("Café");
  });

  // Ported from graph.test "discards an older same-epoch page-inventory response".
  it("drops a response older than the one it holds", async () => {
    let releaseStale!: (value: PageInventory) => void;
    backendMock.pageInventory.mockResolvedValueOnce(inventory(1, file("Base")));
    const { installPageIndex, refreshPageIndex, navigationName, allPages } = await load();
    installPageIndex();
    await vi.waitFor(() => expect(navigationName("base")).toBe("Base"));

    backendMock.pageInventory
      .mockImplementationOnce(() => new Promise<PageInventory>((resolve) => { releaseStale = resolve; }))
      .mockResolvedValueOnce(inventory(3, file("Newest")));
    const older = refreshPageIndex();
    const newer = refreshPageIndex();
    await newer;
    releaseStale(inventory(2, file("Stale")));
    await older;

    expect(navigationName("newest")).toBe("Newest");
    expect(navigationName("stale")).toBe("stale");
    expect(allPages()).toEqual([page("Newest")]);
  });

  it("drops a response from before a graph switch", async () => {
    let releaseStale!: (value: PageInventory) => void;
    backendMock.pageInventory.mockImplementationOnce(
      () => new Promise<PageInventory>((resolve) => { releaseStale = resolve; }),
    );
    const { installPageIndex, resetPageIndex, bumpGraphEpoch, navigationName, allPages } = await load();
    installPageIndex();
    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(1));

    backendMock.pageInventory.mockResolvedValueOnce(inventory(1, file("Fresh")));
    resetPageIndex();
    bumpGraphEpoch();
    await vi.waitFor(() => expect(navigationName("fresh")).toBe("Fresh"));
    // The old graph had a higher rev; its late response still loses.
    releaseStale(inventory(99, file("Old graph")));
    await Promise.resolve();
    await Promise.resolve();
    expect(navigationName("old graph")).toBe("old graph");
    expect(allPages()).toEqual([page("Fresh")]);
  });

  it("navigates an alias with several owners to the first one", async () => {
    backendMock.pageInventory.mockResolvedValue(inventory(1,
      file("Alpha", "pages/Alpha.md", ["pages/nested/Alpha.org"]),
      file("Beta", "pages/Beta.org"),
      alias("Shared", "pages/Alpha.md", "pages/Beta.org"),
    ));
    const { installPageIndex, navigationName } = await load();
    installPageIndex();
    await vi.waitFor(() => expect(navigationName("shared")).toBe("Alpha"));
  });

  // The frontend half of the Rust `page_inventory_wire_matches_legacy_fixture`
  // (formerly the list_pages / page_aliases / referenced_page_names adapters):
  // All Pages lists every physical file (a duplicate-day journal only its
  // canonical file), and the name list adds alias and reference-only names.
  it("derives All Pages and the name list from the legacy fixture wire", async () => {
    backendMock.pageInventory.mockResolvedValue(inventory(7,
      file("Alpha", "pages/Alpha.md", ["pages/nested/Alpha.org"]),
      absent("Another Ref"),
      file("Beta", "pages/Beta.org"),
      {
        key: "jun 26th, 2026",
        name: "Jun 26th, 2026",
        is_journal: true,
        day: 20260626,
        target: { kind: "existing", id: "journals/2026_06_26.md", others: ["journals/Jun 26th, 2026.org"] },
      },
      absent("Only Linked"),
      alias("Shared", "pages/Alpha.md", "pages/Beta.org"),
      file("Team/Child", "pages/Team%2FChild.md"),
    ));
    const { allPages, allPageNames } = await load();
    await vi.waitFor(() => expect(allPages()).toEqual([
      { name: "Alpha", kind: "page", date_key: null, path: "pages/Alpha.md" },
      { name: "Alpha", kind: "page", date_key: null, path: "pages/nested/Alpha.org" },
      { name: "Beta", kind: "page", date_key: null, path: "pages/Beta.org" },
      { name: "Jun 26th, 2026", kind: "journal", date_key: 20260626, path: "journals/2026_06_26.md" },
      { name: "Team/Child", kind: "page", date_key: null, path: "pages/Team%2FChild.md" },
    ]));
    expect([...allPageNames()].sort()).toEqual([
      "Alpha", "Another Ref", "Beta", "Jun 26th, 2026", "Only Linked", "Shared", "Team/Child",
    ]);
  });
});
