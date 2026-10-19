// GH #725: the block context menu. (a) a favorited block offers removal instead
// of "Favourite block", through the Favorites tree's own commit path; (b) Copy
// block ref / embed / link are top-level rows as in OG's block context menu
// (frontend/components/content.cljs `block-context-menu-content`: "Copy block
// ref", "Copy block embed", "Copy block URL" are siblings of Cut and Delete,
// not behind a submenu).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import "../graph"; // installs the Favorites page door, as at app start
import { ContextMenu } from "./ContextMenu";
import { backend } from "../backend";
import { installDiskHost } from "../diskHost.test.support";
import { bindTestHost } from "../document/host/wiring.test.support";
import { initParser } from "../render/parse";
import { ensurePageLoaded, resetStore } from "../document";
import { favoritesLayout, isFavoriteBlock, seedFavorites } from "../favorites";
import { layoutToMarkdown } from "../favoritesLayout";
import { setGraphMeta } from "../graphSession";
import { closeContextMenu, openContextMenu } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";
import { setToasts } from "../toasts";
import type { BlockDto, PageRead } from "../types";

type DiskPage = { pre_block: string | null; blocks: BlockDto[]; rev: number };
let disk: Map<string, DiskPage>;
let dispose: (() => void) | undefined;
const b = (raw: string, children: BlockDto[] = []): BlockDto => ({ id: "", raw, collapsed: false, children });
const settle = async () => { for (let turn = 0; turn < 8; turn++) { for (let i = 0; i < 30; i++) await Promise.resolve(); await new Promise((r) => setTimeout(r, 0)); } };
const labels = () => [...document.querySelectorAll(".ctx-item")].map((el) => el.textContent?.trim() ?? "");
const topLevel = () => [...document.querySelectorAll<HTMLElement>(".ctx-item")]
  .filter((el) => !el.closest(".ctx-submenu-menu")).map((el) => el.textContent?.trim() ?? "");
const choose = (label: string) => {
  // Top level only: a row behind a submenu is not what the user sees first.
  const row = [...document.querySelectorAll<HTMLElement>(".ctx-item")].filter((el) => !el.closest(".ctx-submenu-menu")).find((el) => el.textContent?.trim() === label);
  expect(row, label).toBeDefined();
  row!.click();
};
const reopen = (id: string) => { closeContextMenu(); openContextMenu(10, 10, id); };

beforeAll(() => initParser());
beforeEach(async () => {
  disk = new Map();
  const api = backend();
  vi.spyOn(api, "getPage").mockImplementation(async (name: string) => {
    const page = disk.get(name);
    return page ? ({ name, kind: "page", title: name, pre_block: page.pre_block, blocks: page.blocks, rev: String(page.rev), format: "md", id: `pages/${name}.md` } as PageRead) : null;
  });
  vi.spyOn(api, "resolvePage").mockImplementation(async (name: string) =>
    disk.has(name) ? { kind: "existing", id: `pages/${name}.md`, others: [] } : { kind: "absent", id: `pages/${name}.md` });
  vi.spyOn(api, "setFavorites").mockResolvedValue(undefined as never);
  vi.spyOn(api, "resolveBlocks").mockImplementation(async (ids) => ids.map(() => null));
  vi.spyOn(api, "writeText").mockResolvedValue();
  installDiskHost(await bindTestHost(), {
    read: (_key, name) => {
      const page = disk.get(name);
      let n = 0;
      const ids = (bs: BlockDto[]): BlockDto[] => bs.map((x) => ({ ...x, id: `${name}-${++n}`, children: ids(x.children) }));
      return page ? { name, kind: "page", title: name, pre_block: page.pre_block, blocks: ids(page.blocks), rev: String(page.rev), format: "md" } : null;
    },
    write: (_key, dto) => {
      const rev = (disk.get(dto.name)?.rev ?? 0) + 1;
      disk.set(dto.name, { pre_block: dto.pre_block, blocks: dto.blocks.map((x) => ({ ...x, collapsed: false })), rev });
      return String(rev);
    },
  });
  setGraphMeta({ root: "/fixture" } as never);
  seedFavorites([]);
  const root = document.createElement("div");
  document.body.append(root);
  dispose = render(() => <ContextMenu />, root);
});
afterEach(() => {
  dispose?.(); dispose = undefined;
  closeContextMenu(); clearTransientLayersForTest(); resetStore(); setToasts([]); setGraphMeta(null);
  vi.restoreAllMocks(); document.body.innerHTML = "";
});

function loadSource() {
  disk.set("Source", { pre_block: null, blocks: [b("important block"), b("other block")], rev: 1 });
  ensurePageLoaded({ name: "Source", title: "Source", kind: "page", id: "pages/Source.md", rev: "1", format: "md", pre_block: null,
    blocks: [{ ...b("important block"), id: "Source-1" }, { ...b("other block"), id: "Source-2" }] });
}
const favoritesPageText = () => (disk.get("Favorites")?.blocks ?? []).map((x) => x.raw).join("\n");

describe("GH #725 (a): a favorited block offers removal in its context menu", () => {
  it("favourite, reopen the menu, unfavourite: the same row leaves the Favorites page", async () => {
    loadSource();
    openContextMenu(10, 10, "Source-1");
    expect(topLevel()).toContain("Favourite block");
    expect(topLevel()).not.toContain("Unfavourite block");
    choose("Favourite block");
    await settle();
    const uuid = favoritesLayout()[0].blockId!;
    expect(isFavoriteBlock(uuid)).toBe(true);
    expect(favoritesPageText()).toContain(`((${uuid}))`);

    // The menu for the favorited block removes; the other block still adds.
    reopen("Source-1");
    expect(topLevel()).toContain("Unfavourite block");
    expect(topLevel()).not.toContain("Favourite block");
    reopen("Source-2");
    expect(topLevel()).toContain("Favourite block");
    expect(topLevel()).not.toContain("Unfavourite block");

    reopen("Source-1");
    choose("Unfavourite block");
    await settle();
    expect(isFavoriteBlock(uuid)).toBe(false);
    expect(favoritesLayout().some((node) => node.blockId)).toBe(false);
    expect(favoritesPageText()).not.toContain(uuid);
    reopen("Source-1");
    expect(topLevel()).toContain("Favourite block");
  });

  it("removing one favorited block keeps its group and the other rows", async () => {
    const one = "72500000-0000-4000-8000-000000000001";
    const two = "72500000-0000-4000-8000-000000000002";
    disk.set("Source", { pre_block: null, blocks: [b(`important block\nid:: ${one}`), b(`other block\nid:: ${two}`)], rev: 1 });
    ensurePageLoaded({ name: "Source", title: "Source", kind: "page", id: "pages/Source.md", rev: "1", format: "md", pre_block: null,
      blocks: [{ ...b(`important block\nid:: ${one}`), id: "Source-1" }, { ...b(`other block\nid:: ${two}`), id: "Source-2" }] });
    disk.set("Favorites", { pre_block: "tine/favorites:: true", blocks: [b("Work", [b(`((${one}))`), b(`((${two}))`)]), b("[[A]]")], rev: 1 });
    seedFavorites(["A"], "Favorites");
    await settle();
    expect(isFavoriteBlock(one) && isFavoriteBlock(two)).toBe(true);

    openContextMenu(10, 10, "Source-1");
    choose("Unfavourite block");
    await settle();
    expect(layoutToMarkdown(favoritesLayout())).toBe(`- Work\n\t- ((${two}))\n- [[A]]\n`);
    expect(isFavoriteBlock(one)).toBe(false);
    expect(isFavoriteBlock(two)).toBe(true);
  });
});

describe("GH #725 (b): block copy actions sit at the top level, as in OG", () => {
  it("lists Copy block ref, Copy block embed and Copy link beside Copy and Cut, with no Copy as submenu", () => {
    loadSource();
    openContextMenu(10, 10, "Source-1");
    expect(topLevel()).toEqual(expect.arrayContaining(["Copy block ref", "Copy block embed", "Copy link", "Copy block", "Cut block"]));
    expect(document.querySelector(".ctx-submenu")?.textContent ?? "").not.toMatch(/^Copy as/);
    expect(labels().filter((label) => label === "Copy block ref")).toHaveLength(1);
  });

  it.each([["Copy block ref", (id: string) => `((${id}))`], ["Copy block embed", (id: string) => `{{embed ((${id}))}}`]] as const)(
    "%s copies the reference straight from the top-level row", async (label, text) => {
      loadSource();
      openContextMenu(10, 10, "Source-1");
      choose(label);
      await vi.waitFor(() => expect(backend().writeText).toHaveBeenCalledTimes(1));
      const copied = vi.mocked(backend().writeText).mock.calls[0][0];
      const id = /id:: ([0-9a-f-]{36})/.exec(disk.get("Source")!.blocks[0].raw)![1];
      expect(copied).toBe(text(id));
    });
});
