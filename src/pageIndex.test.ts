import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PageEntry, PageInventory, PageInventoryEntry, ResolvedPage } from "./types";

const backendMock = vi.hoisted(() => ({ pageInventory: vi.fn(), graphBindingGeneration: vi.fn(() => 1) }));
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
  await (await import("./render/parse")).initParser();
  const ui = await import("./graphSession");
  const index = await import("./pageIndex");
  const pages = await import("./pages");
  return { ...ui, ...index, ...pages };
}

beforeEach(() => {
  vi.resetModules();
  backendMock.pageInventory.mockReset();
  backendMock.graphBindingGeneration.mockReset().mockReturnValue(1);
});

describe("page index: the one frontend name answerer", () => {
  // Ported from graph.test "loads real page identities once and lets them win
  // colliding aliases": the backend's target is returned verbatim, and each
  // inventory trigger costs one IPC; content-only saves cost none (I-25).
  it("answers from the backend target and refetches only inventory changes", async () => {
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
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(backendMock.pageInventory).toHaveBeenCalledTimes(1);
    bumpPageInventoryRev();
    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(2));
    // A save that bumps both in one tick costs one IPC, not two.
    bumpDataRev();
    bumpPageInventoryRev();
    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(3));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(backendMock.pageInventory).toHaveBeenCalledTimes(3);
  });

  // PERF-FIXES D2: a rename publishes each referrer separately, and each
  // publication bumps the inventory signal. While one read is in flight the
  // rest coalesce into one trailing read, which sees the burst's last state.
  it("coalesces an inventory burst during an in-flight read into one trailing read", async () => {
    let release: (value: PageInventory) => void = () => {};
    backendMock.pageInventory.mockResolvedValueOnce(inventory(1, file("old")));
    const { installPageIndex, navigationName, bumpPageInventoryRev } = await load();
    installPageIndex();
    await vi.waitFor(() => expect(navigationName("OLD")).toBe("old"));
    backendMock.pageInventory.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    backendMock.pageInventory.mockResolvedValueOnce(inventory(3, file("new")));
    bumpPageInventoryRev();
    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(2));
    for (let i = 0; i < 50; i++) {
      bumpPageInventoryRev();
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    expect(backendMock.pageInventory).toHaveBeenCalledTimes(2);
    release(inventory(2, file("mid")));
    await vi.waitFor(() => expect(navigationName("NEW")).toBe("new"));
    expect(backendMock.pageInventory).toHaveBeenCalledTimes(3);
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

  it("resolves Unicode-trimmed file and alias names with native inventory keys", async () => {
    backendMock.pageInventory.mockResolvedValue(inventory(1,
      entry("Café", { kind: "existing", id: "pages/Café.md", others: [] }, "café"),
      entry("Shortcut", { kind: "alias", owners: ["pages/Café.md"] }, "shortcut"),
      entry("\uFEFFFoo\uFEFF", { kind: "existing", id: "pages/BOM.md", others: [] }, "\uFEFFfoo\uFEFF"),
    ));
    const { installPageIndex, navigationName, resolvedTarget } = await load();
    installPageIndex();
    await vi.waitFor(() => expect(navigationName("\u0085/CAFÉ/\u0085")).toBe("Café"));
    expect(navigationName("\u0085Shortcut\u0085")).toBe("Café");
    expect(resolvedTarget("\uFEFFFoo\uFEFF")).toEqual({ kind: "existing", id: "pages/BOM.md", others: [] });
    expect(resolvedTarget("Foo")).toBeUndefined();
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

  // EMPTY-ALIAS (D-20261103-CMA, as Logseq): an empty `ML.md` beside
  // `Machine Learning` (`alias:: ML`) yields its name. The store marks the
  // entry; a link and a named route open the owner, while the file is still
  // the page that owns the name (All Pages, writes, the exact-path route).
  it("routes an empty alias page's name to the alias owner", async () => {
    backendMock.pageInventory.mockResolvedValue(inventory(1,
      file("Machine Learning"),
      { ...file("ML"), link: { name: "Machine Learning", kind: "page" }, empty_alias: true },
      { ...file("2026-10-26"), link: { name: "Oct 26th, 2026", kind: "journal" } },
    ));
    const { installPageIndex, linkTarget, navigationTarget, navigationName, resolvedTarget, ownerPath } = await load();
    installPageIndex();
    await vi.waitFor(() => expect(linkTarget("ML")).toEqual({ name: "Machine Learning", kind: "page" }));
    expect(navigationTarget("ml")).toEqual({ name: "Machine Learning", kind: "page" });
    expect(navigationName("ML")).toBe("ML");
    expect(ownerPath(resolvedTarget("ML"))).toBe("pages/ML.md");
    // A date-named page with content keeps its named route (its link moves
    // for the day, not for an alias).
    expect(navigationTarget("2026-10-26")).toEqual({ name: "2026-10-26", kind: "page" });
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


it("reports a failed inventory read and keeps the last good page list", async () => {
  const { refreshPageIndex, allPages } = await load();
  const { toasts, setToasts } = await import("./toasts");
  setToasts([]);
  backendMock.pageInventory.mockResolvedValue(inventory(1, file("Kept")));
  await refreshPageIndex();
  const previous = allPages();
  backendMock.pageInventory.mockRejectedValue(new Error("io:PermissionDenied"));
  await refreshPageIndex();
  expect(allPages()).toEqual(previous);
  expect(toasts().some((t) => t.kind === "error" && t.message.includes("page list"))).toBe(true);
});

// Checkpoint-4 B-FAIL regression: at launch the webview asks for the page list
// before the startup load has bound this window (and Quick Capture is never
// bound). The backend refuses with `no graph loaded for window …` or
// `missing-graph-binding`: a transient state, not a failed read, so it raises no
// sticky error toast (Martin 2026-09-29: transient states raise none).
it("raises no failure toast for an inventory read issued before the window is bound", async () => {
  const { refreshPageIndex } = await load();
  const { toasts, setToasts } = await import("./toasts");
  setToasts([]);
  backendMock.graphBindingGeneration.mockReturnValue(0);
  for (const refusal of ["no graph loaded for window main", "missing-graph-binding"]) {
    backendMock.pageInventory.mockRejectedValue(new Error(refusal));
    await refreshPageIndex();
  }
  expect(toasts().filter((t) => t.kind === "error")).toEqual([]);
});

it("keeps readable names and reports unreadable files once when a file name can't be read", async () => {
  const { refreshPageIndex, resolvedTarget } = await load();
  const { toasts, setToasts } = await import("./toasts");
  setToasts([]);
  backendMock.pageInventory.mockResolvedValue({ ...inventory(1, file("Good")), unreadable: ["pages/Bad.md"] });
  await refreshPageIndex();
  await refreshPageIndex();
  expect(resolvedTarget("Good")).toEqual({ kind: "existing", id: "pages/Good.md", others: [] });
  const errors = toasts().filter((t) => t.kind === "error");
  expect(errors).toHaveLength(1);
  expect(errors[0].message).toContain("couldn't be read");
  // GH #727: the toast itself names the file, and does not point at the opt-in debug log.
  expect(errors[0].message).toContain("pages/Bad.md");
  expect(errors[0].message).not.toContain("debug log");
});

// Cost-model audit 2026-11-02 row 1: typing `[[Existing Page]]` (or editing
// `alias::`) moves only alias and reference-only names. The publication
// carries them; the index patches itself with no `page_inventory` IPC and
// answers exactly as a fresh read would.
describe("page index: name-only inventory deltas", () => {
  it.each([8, 32])("patches one name without visiting or notifying %i unrelated names", async (size) => {
    const { createRoot, createComputed } = await import("solid-js");
    let visits = 0;
    const entries = Array.from({ length: size }, (_, i) => {
      const row = absent(`Name${i}`);
      const target = row.target;
      Object.defineProperty(row, "target", { get: () => { visits++; return target; } });
      return row;
    });
    const index = await answers(inventory(1, ...entries));
    let runs = 0;
    const dispose = createRoot(dispose => {
      for (const row of entries) createComputed(() => { index.resolvedTarget(row.name); runs++; });
      return dispose;
    });
    visits = runs = 0;
    expect(index.patchPageIndex(2n, { keys: ["fresh"], entries: [absent("Fresh")] })).toEqual({ aliases: false });
    expect({ runs, visits }).toEqual({ runs: 0, visits: 0 });
    expect(index.resolvedTarget("Fresh")?.kind).toBe("absent");
    expect(index.allPageNames()).toContain("Fresh");
    runs = 0;
    backendMock.pageInventory.mockResolvedValue(inventory(3, ...entries, absent("Fresh")));
    await index.refreshPageIndex();
    expect(runs).toBe(0);
    dispose();
  });

  it("keeps spelling twins, kind-qualified names and owner-path dependencies through patches and refresh", async () => {
    const twins = [file("Foo", "pages/Foo.md", ["pages/foo.md"]), file("foo", "pages/Foo.md", ["pages/foo.md"])];
    const journal = { ...absent("Same"), is_journal: true, day: 20261107 };
    const shortcut = alias("Shortcut", "pages/Foo.md");
    const index = await answers(inventory(1, ...twins, absent("Same"), journal, shortcut));
    const files = index.allPages();
    index.patchPageIndex(2n, { keys: ["typed"], entries: [absent("Typed")] });
    expect(index.allPages()).toBe(files);
    expect(index.navigationName("FOO")).toBe("Foo");
    expect(index.resolvedTarget("Same", "journal")).toEqual(journal.target);
    const { createRoot, createComputed } = await import("solid-js");
    const names: string[] = [];
    const dispose = createRoot(dispose => {
      createComputed(() => names.push(index.navigationName("Shortcut")));
      return dispose;
    });
    backendMock.pageInventory.mockResolvedValue(inventory(3, file("Renamed", "pages/Foo.md"), shortcut));
    await index.refreshPageIndex();
    expect(names).toEqual(["Foo", "Renamed"]);
    index.resetPageIndex();
    expect(names.at(-1)).toBe("Shortcut");
    backendMock.pageInventory.mockResolvedValue(inventory(4, file("Again", "pages/Foo.md"), shortcut));
    await index.refreshPageIndex();
    expect(names.at(-1)).toBe("Again");
    dispose();
  });

  it("keeps the backend order through growing, shrinking and replacing name buckets", async () => {
    const index = await answers(inventory(1, file("Physical")));
    const names = Array.from({ length: 96 }, (_, i) => `Name${String(i).padStart(3, "0")}`);
    const expected = new Set<string>();
    let rev = 1n;
    const check = () => expect(index.allPageNames()).toEqual(["Physical", ...[...expected].sort()]);
    for (const name of names.toReversed()) {
      index.patchPageIndex(++rev, { keys: [name.toLowerCase()], entries: [absent(name)] });
      expected.add(name); check();
    }
    for (const name of names.filter((_, i) => i % 2 === 0)) {
      index.patchPageIndex(++rev, { keys: [name.toLowerCase()], entries: [] });
      expected.delete(name); check();
    }
    for (const name of names) {
      index.patchPageIndex(++rev, { keys: [name.toLowerCase()], entries: [alias(name, "pages/Physical.md")] });
      expected.add(name); check();
    }
    for (const name of names.toReversed()) {
      index.patchPageIndex(++rev, { keys: [name.toLowerCase()], entries: [] });
      expected.delete(name); check();
    }
  });
  const sorted = (...entries: PageInventoryEntry[]) =>
    [...entries].sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  const change = (rev: number, keys: string[], entries: PageInventoryEntry[]) =>
    ({ rev: String(rev), inventoryChanged: true, blockRefCounts: {}, inventoryDelta: { keys, entries } });

  async function answers(fresh: PageInventory) {
    const index = await load();
    const graphAnswers = await import("./graphAnswers");
    backendMock.pageInventory.mockResolvedValue(fresh);
    index.installPageIndex();
    await vi.waitFor(() => expect(index.allPages()).toBeDefined());
    backendMock.pageInventory.mockClear();
    return { ...index, ...graphAnswers };
  }

  /** What the index answers, for comparing a patched index with a fresh one. */
  function snapshot(index: Awaited<ReturnType<typeof answers>>, names: string[]) {
    return {
      names: [...index.allPageNames()],
      pages: index.allPages(),
      targets: names.map((name) => [index.resolvedTarget(name), index.resolvedTarget(name, "journal"), index.navigationName(name)]),
    };
  }

  it("patches typed link names in place: no page_inventory call, the same answers as a fresh read", async () => {
    const base = [file("Existing Page"), file("b"), alias("Nick", "pages/b.md"), absent("Ex"), absent("Team/Child")];
    const index = await answers(inventory(1, ...sorted(...base)));
    const aliasRev = index.pageAliasRev();
    // `[[E]]`, `[[Ex]]` (already referenced elsewhere), `[[Exi]]`, ... `[[Existing Page]]`.
    index.applyGraphAnswers(change(2, ["e"], [absent("E")]));
    index.applyGraphAnswers(change(3, ["e"], []));
    index.applyGraphAnswers(change(4, ["exi"], [absent("Exi")]));
    index.applyGraphAnswers(change(5, ["exi", "zz/ns"], [absent("ZZ/ns")]));
    index.applyGraphAnswers(change(6, ["zz/ns", "😀", "￿"], [absent("😀"), absent("￿")]));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(backendMock.pageInventory).not.toHaveBeenCalled();
    expect(index.pageAliasRev()).toBe(aliasRev);
    const names = ["Existing Page", "b", "Nick", "Ex", "E", "Exi", "ZZ/ns", "😀", "￿"];
    const patched = snapshot(index, names);
    // Rust orders keys by code point: U+FFFF before U+1F600 (JS `<` would not).
    const fresh = await answers(inventory(6, ...sorted(...base), absent("￿"), absent("😀")));
    expect(patched).toEqual(snapshot(fresh, names));
  });

  it("an alias edit patches the alias entry and re-asks alias groups, still with no inventory IPC", async () => {
    const base = [file("a"), file("b"), absent("Later Alias")];
    const index = await answers(inventory(1, ...base));
    const aliasRev = index.pageAliasRev();
    index.applyGraphAnswers(change(2, ["later alias"], [alias("Later Alias", "pages/a.md")]));
    index.applyGraphAnswers(change(3, ["nick"], [alias("nick", "pages/a.md", "pages/b.md")]));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(backendMock.pageInventory).not.toHaveBeenCalled();
    expect(index.pageAliasRev()).toBe(aliasRev + 2);
    expect(index.navigationName("LATER ALIAS")).toBe("a");
    expect(index.resolvedTarget("Nick")).toEqual({ kind: "alias", owners: ["pages/a.md", "pages/b.md"] });
  });

  it("refetches when the change cannot be patched in order", async () => {
    const index = await answers(inventory(1, file("a")));
    // No delta (a page file claimed or released a name): refetch.
    backendMock.pageInventory.mockResolvedValue(inventory(2, file("a"), file("new")));
    index.applyGraphAnswers({ rev: "2", inventoryChanged: true, blockRefCounts: {} });
    await vi.waitFor(() => expect(index.resolvedTarget("new")?.kind).toBe("existing"));
    expect(backendMock.pageInventory).toHaveBeenCalledTimes(1);
    // A delta while a read is in flight: the trailing read answers instead.
    let release: (value: PageInventory) => void = () => {};
    backendMock.pageInventory.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    index.applyGraphAnswers({ rev: "3", inventoryChanged: true, blockRefCounts: {} });
    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(2));
    backendMock.pageInventory.mockResolvedValue(inventory(4, file("a"), file("new"), absent("typed")));
    index.applyGraphAnswers(change(4, ["typed"], [absent("typed")]));
    release(inventory(3, file("a"), file("new")));
    await vi.waitFor(() => expect(index.resolvedTarget("typed")?.kind).toBe("absent"));
    expect(backendMock.pageInventory).toHaveBeenCalledTimes(3);
    // A rebind forgets the held answer: the next delta refetches.
    index.resetPageIndex();
    backendMock.pageInventory.mockResolvedValue(inventory(5, file("a")));
    index.applyGraphAnswers(change(5, ["typed"], []));
    await vi.waitFor(() => expect(backendMock.pageInventory).toHaveBeenCalledTimes(4));
  });
});
