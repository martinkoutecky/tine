// Family 22 (master 5b3808ce2, d5bb17858, 7b162cb8d): the arrangement page as
// user data. Drives the real entry points against a fake disk behind the
// backend: page reads/writes through the document door, config through
// setFavorites.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "./graph"; // installs the Favorites page door, as at app start
import { backend, type SavePageEntry } from "./backend";
import { bumpDataRev, graphMeta, setGraphMeta } from "./graphSession";
import {
  addFavoriteGroup, deleteFavoriteGroup, favorites, favoritesLayout, moveFavoriteRow,
  renameFavoriteGroup, seedFavorites, setFavoriteRowCollapsed, toggleFavorite,
} from "./favorites";
import { layoutToMarkdown } from "./favoritesLayout";
import { renamePageInNavigation } from "./ui";
import { toasts, setToasts } from "./toasts";
import type { BlockDto, Format, GraphMeta, PageRead } from "./types";

type DiskPage = { pre_block: string | null; blocks: BlockDto[]; rev: number; format?: Format };
let disk: Map<string, DiskPage>;
let config: { names: string[]; page: string | null };
let writes: string[];

const b = (raw: string, children: BlockDto[] = []): BlockDto => ({ id: "", raw, collapsed: false, children });
// The real save path writes raw only; a read derives `collapsed` from the raw.
const toDisk = (bs: BlockDto[]): BlockDto[] => bs.map((x) => ({ ...x, collapsed: false, children: toDisk(x.children) }));
const COLLAPSED: Record<Format, RegExp> = { md: /^collapsed:: true$/m, org: /^:PROPERTIES:\n:collapsed: true\n:END:$/m };
const fromDisk = (bs: BlockDto[], f: Format = "md"): BlockDto[] =>
  bs.map((x) => ({ ...x, collapsed: COLLAPSED[f].test(x.raw), children: fromDisk(x.children, f) }));
// As the store: a new page takes the graph's preferred format (pages/<name>.org in an Org graph).
const preferred = (): Format => graphMeta()?.preferred_format ?? "md";
const settle = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); await new Promise((r) => setTimeout(r, 0)); };
const md = () => layoutToMarkdown(favoritesLayout());
const line = (x: BlockDto, d: number): string => `${"\t".repeat(d)}- ${x.raw}\n${x.children.map((c) => line(c, d + 1)).join("")}`;
const shape = (page: DiskPage) => ({ pre: page.pre_block, text: page.blocks.map((x) => line(x, 0)).join("") });

beforeEach(() => {
  disk = new Map();
  config = { names: [], page: null };
  writes = [];
  const api = backend();
  vi.spyOn(api, "getPage").mockImplementation(async (name: string) => {
    const page = disk.get(name);
    return page ? ({ name, kind: "page", title: name, pre_block: page.pre_block, blocks: fromDisk(page.blocks, page.format), rev: String(page.rev), format: page.format ?? "md", id: `pages/${name}.${page.format ?? "md"}` } as PageRead) : null;
  });
  vi.spyOn(api, "resolvePage").mockImplementation(async (name: string) =>
    disk.has(name) ? { kind: "existing", id: `pages/${name}.${disk.get(name)!.format ?? "md"}`, others: [] } : { kind: "absent", id: `pages/${name}.${preferred()}` });
  vi.spyOn(api, "savePages").mockImplementation(async (entries: SavePageEntry[]) => {
    const [entry] = entries;
    const current = disk.get(entry.page.name);
    if (String(current?.rev ?? null) !== String(entry.baseRev)) return { failed: { index: 0, family: "conflict", undoFailed: [] } };
    const rev = (current?.rev ?? 0) + 1;
    disk.set(entry.page.name, { pre_block: entry.page.pre_block, blocks: toDisk(entry.page.blocks), rev, format: current?.format ?? preferred() });
    writes.push(`page:${entry.page.name}:${entry.kinds.join(",")}`);
    return { ok: [String(rev)] };
  });
  vi.spyOn(api, "setFavorites").mockImplementation(async (names: string[], page?: string | null) => {
    config = { names: [...names], page: page ?? config.page };
    writes.push(`config:${names.join("|")}:${page ?? "-"}`);
  });
  seedFavorites([]);
});
afterEach(() => { vi.restoreAllMocks(); setToasts([]); setGraphMeta(null); });

describe("favorites arrangement page", () => {
  it("a flat list never grows a page; the first label creates it, page first, then config", async () => {
    toggleFavorite("A");
    toggleFavorite("B");
    await settle();
    expect(disk.size).toBe(0);
    expect(writes).toEqual(["config:A:-", "config:A|B:-"]);
    addFavoriteGroup();
    await settle();
    expect(shape(disk.get("Favorites")!)).toEqual({ pre: "tine/favorites:: true", text: "- [[A]]\n- [[B]]\n- New group\n" });
    expect(writes.slice(2)).toEqual(["page:Favorites:create-page", "config:A|B:Favorites"]);
    expect(config).toEqual({ names: ["A", "B"], page: "Favorites" });
  });

  it("nests, renames, collapses and deletes groups; membership is the pre-order projection", async () => {
    toggleFavorite("A");
    toggleFavorite("B");
    addFavoriteGroup("Work");
    await settle();
    moveFavoriteRow([0], [2], 0); // A into Work
    await settle();
    expect(md()).toBe("- [[B]]\n- Work\n\t- [[A]]\n");
    expect(config.names).toEqual(["B", "A"]);
    addFavoriteGroup("Work");
    renameFavoriteGroup([1], "work");
    await settle();
    expect(md()).toBe("- [[B]]\n- work\n\t- [[A]]\n- Work 2\n");
    setFavoriteRowCollapsed([1], true);
    await settle();
    expect(disk.get("Favorites")!.blocks[1].raw).toBe("work\ncollapsed:: true");
    seedFavorites(config.names, config.page); // reopen: collapse came back from disk
    await settle();
    expect(favoritesLayout()[1]).toMatchObject({ raw: "work", collapsed: true });
    deleteFavoriteGroup([1]);
    await settle();
    expect(md()).toBe("- [[B]]\n- [[A]]\n- Work 2\n");
    expect(favorites().map((f) => f.name)).toEqual(["B", "A"]);
    expect(writes.filter((w) => w.startsWith("page:")).every((w) => w.endsWith(":create-page") || w.endsWith(":replace-page"))).toBe(true);
  });

  it("never writes over a user's own page named Favorites", async () => {
    disk.set("Favorites", { pre_block: null, blocks: [b("my notes")], rev: 1 });
    toggleFavorite("A");
    addFavoriteGroup();
    await settle();
    expect(shape(disk.get("Favorites")!).text).toBe("- my notes\n");
    expect(config.page).toBe("Favorites 2");
    expect(shape(disk.get("Favorites 2")!).text).toBe("- [[A]]\n- New group\n");
  });

  it.each([
    ["md", "```md\ntine/favorites:: true\n```"],
    ["org", "#+BEGIN_SRC\n#+tine/favorites: true\n#+END_SRC"],
  ] as const)("a %s code example cannot claim a user's Favorites page", async (format, pre_block) => {
    disk.set("Favorites", { pre_block, blocks: [b("my notes")], rev: 1, format });
    toggleFavorite("A");
    addFavoriteGroup("Work");
    await settle();
    expect(config.page).toBe("Favorites 2");
    expect(shape(disk.get("Favorites")!)).toEqual({ pre: pre_block, text: "- my notes\n" });
    expect(shape(disk.get("Favorites 2")!).text).toBe("- [[A]]\n- Work\n");
  });

  it("opens with config.edn membership over the page, writing nothing", async () => {
    disk.set("Favs", { pre_block: "tine/favorites:: true", blocks: [b("Work", [b("[[A]]"), b("[[Gone]]", [b("[[C]]")])])], rev: 3 });
    seedFavorites(["A", "C", "New"], "Favs");
    await settle();
    expect(md()).toBe("- Work\n\t- [[A]]\n\t- [[C]]\n- [[New]]\n");
    expect(writes).toEqual([]);
  });

  it("adopts an edit to the page as membership, without writing the page back", async () => {
    disk.set("Favs", { pre_block: "tine/favorites:: true", blocks: [b("[[A]]"), b("[[B]]")], rev: 1 });
    seedFavorites(["A", "B"], "Favs");
    await settle();
    disk.set("Favs", { pre_block: "tine/favorites:: true", blocks: [b("Work", [b("[[B]]")]), b("[[D]]")], rev: 2 });
    bumpDataRev();
    await settle();
    expect(md()).toBe("- Work\n\t- [[B]]\n- [[D]]\n");
    expect(writes).toEqual(["config:B|D:Favs"]);
    bumpDataRev(); // an unrelated save: the page is unchanged, nothing happens
    await settle();
    expect(writes).toHaveLength(1);
  });

  it("does not overwrite an outside edit it has not seen: it adopts it and rolls the change back", async () => {
    disk.set("Favs", { pre_block: "tine/favorites:: true", blocks: [b("[[A]]"), b("Work")], rev: 1 });
    seedFavorites(["A"], "Favs");
    await settle();
    disk.set("Favs", { pre_block: "tine/favorites:: true", blocks: [b("[[A]]"), b("Home")], rev: 2 });
    addFavoriteGroup("Mine");
    await settle();
    expect(shape(disk.get("Favs")!).text).toBe("- [[A]]\n- Home\n");
    expect(md()).toBe("- [[A]]\n- Home\n");
    expect(toasts().some((t) => t.kind === "error")).toBe(true);
  });

  it("kill-and-reopen between the page write and the config write: the next edit recovers the orphan instead of overwriting it", async () => {
    toggleFavorite("A");
    toggleFavorite("B");
    await settle();
    vi.spyOn(backend(), "setFavorites").mockRejectedValueOnce(new Error("killed"));
    addFavoriteGroup("Kept"); // page written, then the process dies
    await settle();
    expect(config).toEqual({ names: ["A", "B"], page: null }); // config never saw the page
    expect(shape(disk.get("Favorites")!).text).toBe("- [[A]]\n- [[B]]\n- Kept\n");
    config.names = ["A"]; // meanwhile Logseq unstarred B: config stays authoritative for membership
    seedFavorites(config.names, config.page); // reopen
    await settle();
    expect(md()).toBe("- [[A]]\n");
    setToasts([]);
    addFavoriteGroup("Work"); // a later grouped edit finds the orphan
    await settle();
    await settle();
    expect(toasts().some((t) => t.message.includes("Recovered the Favorites page"))).toBe(true);
    expect(md()).toBe("- [[A]]\n- Kept\n"); // the orphan's arrangement, config's membership
    expect(config).toEqual({ names: ["A"], page: "Favorites" });
    expect(shape(disk.get("Favorites")!).text).toBe("- [[A]]\n- Kept\n");
    expect(disk.has("Favorites 2")).toBe(false);
    addFavoriteGroup("Work"); // repeating the change now lands over the recovered page
    await settle();
    expect(shape(disk.get("Favorites")!).text).toBe("- [[A]]\n- Kept\n- Work\n");
  });

  it("an orphan recovery scheduled in one graph never lands in the next one (OG-C5 L12, I-20)", async () => {
    toggleFavorite("A");
    await settle();
    vi.spyOn(backend(), "setFavorites").mockRejectedValueOnce(new Error("killed"));
    addFavoriteGroup("Kept"); // page written, config never saw it
    await settle();
    seedFavorites(config.names, config.page); // reopen with the orphan on disk
    await settle();
    setToasts([]);
    addFavoriteGroup("Work"); // finds the orphan and schedules its recovery
    for (let i = 0; i < 60 && !toasts().length; i++) await Promise.resolve();
    expect(toasts().some((t) => t.message.includes("Recovered the Favorites page"))).toBe(true);
    // The next graph binds before the scheduled recovery runs.
    disk.set("Other", { pre_block: "tine/favorites:: true", blocks: [b("[[X]]"), b("[[Y]]")], rev: 1 });
    writes = [];
    seedFavorites(["X"], "Other");
    await settle();
    await settle();
    expect(writes).toEqual([]);
    expect(md()).toBe("- [[X]]\n");
  });

  it("an older page read that lands after a newer one is dropped (I-20)", async () => {
    toggleFavorite("A");
    toggleFavorite("B");
    addFavoriteGroup("G");
    await settle();
    const getPage = backend().getPage as unknown as ReturnType<typeof vi.fn>;
    const real = getPage.getMockImplementation()!;
    const held: { resolve: () => void }[] = [];
    getPage.mockImplementation((...args: unknown[]) => {
      const answer = real(...args); // snapshot the disk as of this read
      return new Promise((resolve) => held.push({ resolve: () => resolve(answer) }));
    });
    const edit = (blocks: BlockDto[]) => {
      const page = disk.get("Favorites")!;
      disk.set("Favorites", { ...page, blocks, rev: page.rev + 1 });
      bumpDataRev();
    };
    edit([b("[[A]]"), b("G")]); // outside edit 1: B removed
    await settle();
    edit([b("[[A]]"), b("[[B]]"), b("[[C]]"), b("G")]); // outside edit 2: C added
    await settle();
    expect(held).toHaveLength(2);
    held[1].resolve(); // the newer read lands first
    await settle();
    held[0].resolve(); // then the older one
    await settle();
    getPage.mockImplementation(real);
    await settle();
    expect(favorites().map((f) => f.name)).toEqual(["A", "B", "C"]);
    expect(config.names).toEqual(["A", "B", "C"]);
  });

  it("renaming the arrangement page moves :tine/favorites-page with it", async () => {
    disk.set("Favorites", { pre_block: "tine/favorites:: true", blocks: [b("[[A]]"), b("G")], rev: 1 });
    config = { names: ["A"], page: "Favorites" };
    seedFavorites(config.names, config.page);
    await settle();
    disk.set("Faves", disk.get("Favorites")!); // the rename moved the file
    disk.delete("Favorites");
    renamePageInNavigation("Favorites", "Faves");
    await settle();
    expect(config).toEqual({ names: ["A"], page: "Faves" });
    addFavoriteGroup("H");
    await settle();
    expect(disk.has("Favorites")).toBe(false); // not recreated under the old name
    expect(shape(disk.get("Faves")!).text).toBe("- [[A]]\n- G\n- H\n");
  });

  it("keeps a label's raw text verbatim across a later write (I-4)", async () => {
    disk.set("Favorites", { pre_block: "tine/favorites:: true", blocks: [b("[[A]]"), b("  Work  ")], rev: 1 });
    config = { names: ["A"], page: "Favorites" };
    seedFavorites(config.names, config.page);
    await settle();
    toggleFavorite("B"); // an unrelated change rewrites the page
    await settle();
    expect(disk.get("Favorites")!.blocks.map((x) => x.raw)).toEqual(["[[A]]", "  Work  ", "[[B]]"]);
  });

  it("in an Org-preferred graph the page is a valid Org page: #+ marker, collapse as a drawer", async () => {
    setGraphMeta({ preferred_format: "org" } as GraphMeta);
    toggleFavorite("A");
    addFavoriteGroup("Work");
    await settle();
    setFavoriteRowCollapsed([1], true);
    await settle();
    const page = disk.get("Favorites")!;
    expect(page.format).toBe("org");
    expect(page.pre_block).toBe("#+tine/favorites: true");
    expect(page.blocks[1].raw).toBe("Work\n:PROPERTIES:\n:collapsed: true\n:END:");
    seedFavorites(config.names, config.page); // reopen: the Org page is recognised, collapse came back
    await settle();
    expect(favoritesLayout()[1]).toMatchObject({ raw: "Work", collapsed: true });
    toggleFavorite("B"); // a later write keeps the page Org, the drawer not doubled
    await settle();
    expect(disk.get("Favorites")!.blocks.map((x) => x.raw)).toEqual(["[[A]]", "Work\n:PROPERTIES:\n:collapsed: true\n:END:", "[[B]]"]);
    expect(toasts()).toEqual([]);
  });
});

it.each(["md", "org"] as const)("arrangement edits preserve the %s page preamble verbatim", async (format) => {
  const pre = format === "md" ? "tine/favorites:: true\nalias:: My links\n\nKEEP THIS NOTE" : "#+tine/favorites: true\n#+alias: My links\n\nKEEP THIS NOTE";
  disk.set("Favs", { pre_block: pre, blocks: [b("[[A]]"), b("Work")], rev: 1, format });
  seedFavorites(["A"], "Favs");
  await settle();
  addFavoriteGroup("Home");
  await settle();
  expect(disk.get("Favs")!.pre_block).toBe(pre);
  expect(disk.get("Favs")!.blocks.map((x) => x.raw)).toEqual(["[[A]]", "Work", "Home"]);
});
