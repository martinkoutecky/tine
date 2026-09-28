// Family 22 (master 5b3808ce2, d5bb17858, 7b162cb8d): the arrangement page as
// user data. Drives the real entry points against a fake disk behind the
// backend: page reads/writes through the document door, config through
// setFavorites.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "./graph"; // installs the Favorites page door, as at app start
import { backend, type SavePageEntry } from "./backend";
import { bumpDataRev } from "./graphSession";
import {
  addFavoriteGroup, deleteFavoriteGroup, favorites, favoritesLayout, moveFavoriteRow,
  renameFavoriteGroup, seedFavorites, setFavoriteRowCollapsed, toggleFavorite,
} from "./favorites";
import { layoutToMarkdown } from "./favoritesLayout";
import { toasts, setToasts } from "./toasts";
import type { BlockDto, PageRead } from "./types";

type DiskPage = { pre_block: string | null; blocks: BlockDto[]; rev: number };
let disk: Map<string, DiskPage>;
let config: { names: string[]; page: string | null };
let writes: string[];

const b = (raw: string, children: BlockDto[] = []): BlockDto => ({ id: "", raw, collapsed: false, children });
// The real save path writes raw only; a read derives `collapsed` from the raw.
const toDisk = (bs: BlockDto[]): BlockDto[] => bs.map((x) => ({ ...x, collapsed: false, children: toDisk(x.children) }));
const fromDisk = (bs: BlockDto[]): BlockDto[] => bs.map((x) => ({ ...x, collapsed: /^collapsed:: true$/m.test(x.raw), children: fromDisk(x.children) }));
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
    return page ? ({ name, kind: "page", title: name, pre_block: page.pre_block, blocks: fromDisk(page.blocks), rev: String(page.rev), id: `pages/${name}.md` } as PageRead) : null;
  });
  vi.spyOn(api, "resolvePage").mockImplementation(async (name: string) =>
    disk.has(name) ? { kind: "existing", id: `pages/${name}.md`, others: [] } : { kind: "absent", id: `pages/${name}.md` });
  vi.spyOn(api, "savePages").mockImplementation(async (entries: SavePageEntry[]) => {
    const [entry] = entries;
    const current = disk.get(entry.page.name);
    if (String(current?.rev ?? null) !== String(entry.baseRev)) return { failed: { index: 0, family: "conflict", undoFailed: [] } };
    const rev = (current?.rev ?? 0) + 1;
    disk.set(entry.page.name, { pre_block: entry.page.pre_block, blocks: toDisk(entry.page.blocks), rev });
    writes.push(`page:${entry.page.name}:${entry.kinds.join(",")}`);
    return { ok: [String(rev)] };
  });
  vi.spyOn(api, "setFavorites").mockImplementation(async (names: string[], page?: string | null) => {
    config = { names: [...names], page: page ?? config.page };
    writes.push(`config:${names.join("|")}:${page ?? "-"}`);
  });
  seedFavorites([]);
});
afterEach(() => { vi.restoreAllMocks(); setToasts([]); });

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

  it("kill-and-reopen between the page write and the config write reuses the orphaned page", async () => {
    toggleFavorite("A");
    await settle();
    vi.spyOn(backend(), "setFavorites").mockRejectedValueOnce(new Error("killed"));
    addFavoriteGroup();
    await settle();
    expect(config).toEqual({ names: ["A"], page: null }); // config never saw the page
    seedFavorites(config.names, config.page); // reopen
    await settle();
    expect(md()).toBe("- [[A]]\n");
    addFavoriteGroup("Work");
    await settle();
    expect(config.page).toBe("Favorites");
    expect(disk.has("Favorites 2")).toBe(false);
    expect(shape(disk.get("Favorites")!).text).toBe("- [[A]]\n- Work\n");
  });
});
