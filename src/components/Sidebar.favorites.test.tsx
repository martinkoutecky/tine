// Family 22 (master fdf2490fc, 7b162cb8d, c3adf171c, GH #211 favorites part):
// the Favorites list renders the arrangement tree, edits groups in place and
// reorders/nests rows by pointer drag, persisting through the real store.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import "../graph"; // installs the Favorites page door, as at app start
import { backend, type SavePageEntry } from "../backend";
import { favorites, seedFavorites, setRecentPages } from "../ui";
import { openJournals, route } from "../router";
import { Sidebar } from "./Sidebar";

const settle = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); await new Promise((r) => setTimeout(r, 0)); };
function setRect(el: Element, top: number) {
  Object.defineProperty(el, "getBoundingClientRect", { configurable: true,
    value: () => ({ x: 0, y: top, left: 0, top, width: 200, height: 30, right: 200, bottom: top + 30, toJSON: () => ({}) }) });
}
const pointer = (type: string, x: number, y: number) =>
  new ((window as { PointerEvent?: typeof PointerEvent }).PointerEvent ?? MouseEvent)(type,
    { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0, buttons: 1 }) as PointerEvent;

let saved: SavePageEntry[];
let configWrites: [string[], string | null | undefined][];
beforeEach(() => {
  saved = [];
  configWrites = [];
  vi.spyOn(backend(), "getPage").mockResolvedValue(null);
  vi.spyOn(backend(), "resolvePage").mockImplementation(async (name: string) => ({ kind: "absent", id: `pages/${name}.md` }));
  vi.spyOn(backend(), "savePages").mockImplementation(async (entries: SavePageEntry[]) => { saved.push(...entries); return { ok: ["r1"] }; });
  vi.spyOn(backend(), "setFavorites").mockImplementation(async (names: string[], page?: string | null) => { configWrites.push([names, page]); });
});
afterEach(async () => {
  await settle();
  seedFavorites([]);
  setRecentPages([]);
  document.body.innerHTML = "";
  openJournals();
  vi.restoreAllMocks();
});

function mount(names = ["Alpha", "Beta", "Gamma"]) {
  seedFavorites(names);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <Sidebar />, root);
  const rows = () => [...root.querySelectorAll<HTMLElement>("#sidebar-favorites-list .nav-page")];
  const layoutRows = () => rows().forEach((row, i) => setRect(row, i * 30));
  return { root, dispose, rows, layoutRows };
}
function drag(from: HTMLElement, to: HTMLElement, start: [number, number], end: [number, number]) {
  from.dispatchEvent(pointer("pointerdown", ...start));
  const prev = document.elementFromPoint;
  document.elementFromPoint = () => to;
  try {
    document.dispatchEvent(pointer("pointermove", ...end));
    document.dispatchEvent(pointer("pointerup", ...end));
  } finally { document.elementFromPoint = prev; }
}

describe("favorites arrangement in the sidebar", () => {
  it("reorders by a vertical drag and persists membership only, for a flat list", async () => {
    const { dispose, rows, layoutRows } = mount();
    layoutRows();
    const [alpha, , gamma] = rows();
    drag(alpha, gamma, [10, 10], [10, 85]);
    await settle();
    expect(favorites().map((f) => f.name)).toEqual(["Beta", "Gamma", "Alpha"]);
    expect(configWrites).toEqual([[["Beta", "Gamma", "Alpha"], null]]);
    expect(saved).toEqual([]);
    dispose();
  });

  it("nests a row under the one above when dragged right of the grab point", async () => {
    const { dispose, rows, layoutRows } = mount();
    layoutRows();
    const [alpha, beta] = rows();
    drag(beta, alpha, [10, 40], [30, 20]);
    await settle();
    expect(rows().map((row) => row.style.paddingLeft)).toEqual(["6px", "22px", "6px"]);
    expect(favorites().map((f) => f.name)).toEqual(["Alpha", "Beta", "Gamma"]);
    expect(saved[0].page.blocks.map((b) => [b.raw, b.children.map((c) => c.raw)])).toEqual([["[[Alpha]]", ["[[Beta]]"]], ["[[Gamma]]", []]]);
    expect(saved[0].kinds).toEqual(["create-page"]);
    expect(configWrites.at(-1)).toEqual([["Alpha", "Beta", "Gamma"], "Favorites"]);
    dispose();
  });

  it("adds, renames, collapses and deletes a group without unfavoriting anything", async () => {
    const { root, dispose, rows, layoutRows } = mount(["Alpha", "Beta"]);
    root.querySelector<HTMLButtonElement>(".nav-fav-add-group")!.click();
    await settle();
    layoutRows();
    const group = rows()[2];
    expect(group.querySelector<HTMLInputElement>(".nav-fav-group-name")!.value).toBe("New group");
    drag(rows()[0], group, [10, 10], [30, 80]); // Alpha into the group
    await settle();
    expect(rows().map((r) => r.style.paddingLeft)).toEqual(["6px", "6px", "22px"]);
    const input = root.querySelector<HTMLInputElement>(".nav-fav-group-name")!;
    input.value = "Work";
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(saved.at(-1)!.page.blocks.map((b) => b.raw)).toEqual(["[[Beta]]", "Work"]);
    root.querySelector<HTMLButtonElement>(".nav-fav-group-toggle")!.click();
    await settle();
    expect(rows()).toHaveLength(2);
    expect(favorites().map((f) => f.name)).toEqual(["Beta", "Alpha"]);
    root.querySelector<HTMLButtonElement>(".nav-fav-group-delete")!.click();
    await settle();
    expect(rows().map((r) => r.textContent)).toEqual([expect.stringContaining("Beta"), expect.stringContaining("Alpha")]);
    expect(favorites().map((f) => f.name)).toEqual(["Beta", "Alpha"]);
    dispose();
  });

  it("a press without movement navigates; the click ending a drag does not", async () => {
    const { dispose, rows, layoutRows } = mount();
    layoutRows();
    rows()[1].dispatchEvent(pointer("pointerdown", 10, 40));
    document.dispatchEvent(pointer("pointerup", 11, 41));
    rows()[1].click();
    expect(route()).toMatchObject({ kind: "page", name: "Beta" });
    openJournals();
    drag(rows()[0], rows()[2], [10, 10], [10, 85]);
    rows()[2].click();
    expect(route().kind).toBe("journals");
    dispose();
  });

  it("suppresses text selection only while a drag runs", () => {
    const { dispose, rows, layoutRows } = mount();
    layoutRows();
    rows()[0].dispatchEvent(pointer("pointerdown", 10, 10));
    const prev = document.elementFromPoint;
    document.elementFromPoint = () => rows()[2];
    try {
      document.dispatchEvent(pointer("pointermove", 10, 70));
      expect(document.documentElement.classList.contains("drag-selection-suppressed")).toBe(true);
      document.dispatchEvent(pointer("pointercancel", 10, 70));
    } finally { document.elementFromPoint = prev; }
    expect(document.documentElement.classList.contains("drag-selection-suppressed")).toBe(false);
    dispose();
  });
});
