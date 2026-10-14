import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { bumpDataRev, bumpGraphEpoch } from "../graphSession";
import { resetPageIndex } from "../pageIndex";
import { resetTabsToJournals, route } from "../router";
import type { PaneRouter } from "../router";
import { resetLeftSidebarSections, setLeftSidebarOpen } from "../ui";
import type { PageInventoryEntry } from "../types";
import { AllPages, ALL_PAGES_ROW_HEIGHT } from "./AllPages";
import { Sidebar } from "./Sidebar";

// GH #335: the full All Pages view. The user outcomes: every page is reachable
// in a sortable, filterable list on a graph of thousands of pages, opening the
// view reads the page inventory once and nothing per row, and a row opens its
// page by the ordinary link gestures.

const TOTAL = 5000;
const existing = (name: string, extra: Partial<PageInventoryEntry> = {}): PageInventoryEntry => ({
  key: name.toLowerCase(), name, is_journal: false, day: null,
  target: { kind: "existing", id: `pages/${name}.md`, others: [] }, ...extra,
});
const journalEntry = (day: number): PageInventoryEntry => ({
  key: String(day), name: `Journal ${day}`, is_journal: true, day,
  target: { kind: "existing", id: `journals/${day}.md`, others: [] },
});
const inventory = (): PageInventoryEntry[] => [
  ...Array.from({ length: TOTAL }, (_, i) => existing(`Page ${String(i).padStart(5, "0")}`)),
  journalEntry(20260101), journalEntry(20260301),
];

const settle = async () => { for (let i = 0; i < 6; i++) await new Promise((resolve) => setTimeout(resolve, 0)); };
const rowNames = (host: HTMLElement) => [...host.querySelectorAll(".all-pages-open")].map((el) => el.textContent);
const column = (host: HTMLElement, label: string) =>
  [...host.querySelectorAll<HTMLElement>(".all-pages-col")].find((el) => el.textContent?.startsWith(label))!;

function stubRouter() {
  return { openPageTarget: vi.fn(), openPageTargetInNewTab: vi.fn() } as unknown as PaneRouter & {
    openPageTarget: ReturnType<typeof vi.fn>; openPageTargetInNewTab: ReturnType<typeof vi.fn>;
  };
}

/** Graph calls apart from the shared inventory/metadata reads and ownership
 * stamp. The router's debounced saveSession may land from test navigation or
 * teardown; it persists tabs and cannot read or write a page row. */
function spyOnEverythingElse() {
  const spies: { name: string; spy: ReturnType<typeof vi.fn> }[] = [];
  const target = backend() as unknown as Record<string, unknown>;
  const names = new Set<string>();
  for (let proto: object | null = target; proto && proto !== Object.prototype; proto = Object.getPrototypeOf(proto))
    for (const key of Object.getOwnPropertyNames(proto)) names.add(key);
  for (const key of names) {
    if (key === "constructor" || key === "pageInventory" || key === "pageModified" || key === "graphBindingGeneration" || key === "saveSession" || typeof target[key] !== "function") continue;
    spies.push({ name: key, spy: vi.spyOn(target as never, key as never) as never });
  }
  return () => spies.flatMap(({ name, spy }) => spy.mock.calls.map(() => name));
}

async function mountAllPages(entries = inventory(), modified: [string, number][] | "fail" = [], backlinks: [string, number][] = []) {
  const read = vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries });
  const readModified = vi.spyOn(backend(), "pageModified");
  if (modified === "fail") readModified.mockRejectedValue(new Error("no times"));
  else readModified.mockResolvedValue({ rev: "1", modified, backlinks });
  const otherCalls = spyOnEverythingElse();
  bumpGraphEpoch();
  const router = stubRouter();
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <AllPages router={router} />, host);
  await vi.waitFor(() => expect(host.querySelector(".all-pages-open")).not.toBeNull());
  return { host, router, read, readModified, otherCalls, dispose };
}

afterEach(() => {
  resetPageIndex();
  resetTabsToJournals();
  resetLeftSidebarSections();
  setLeftSidebarOpen(false);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("the All Pages view", () => {
  it("refreshes backend counts after a graph change while keeping the current sort", async () => {
    const { host, readModified, dispose } = await mountAllPages([existing("Alpha"), existing("Beta")], [], [["pages/Alpha.md", 2], ["pages/Beta.md", 7]]);
    try {
      column(host, "Backlinks").click();
      await vi.waitFor(() => expect(rowNames(host)).toEqual(["Beta", "Alpha"]));
      readModified.mockResolvedValue({ rev: "2", modified: [], backlinks: [["pages/Alpha.md", 9], ["pages/Beta.md", 7]] });
      bumpDataRev();
      await vi.waitFor(() => expect(rowNames(host)).toEqual(["Alpha", "Beta"]));
      expect(readModified).toHaveBeenCalledTimes(2);
    } finally { dispose(); }
  });
  it("shows backend backlink counts and sorts them in either direction", async () => {
    const { host, dispose } = await mountAllPages([existing("Alpha"), existing("Beta")], [], [["pages/Alpha.md", 2], ["pages/Beta.md", 7]]);
    try {
      await vi.waitFor(() => expect([...host.querySelectorAll(".all-pages-backlinks")].map(el => el.textContent)).toEqual(["2", "7"]));
      column(host, "Backlinks").click();
      expect(rowNames(host)).toEqual(["Beta", "Alpha"]);
      column(host, "Backlinks").click();
      expect(rowNames(host)).toEqual(["Alpha", "Beta"]);
    } finally { dispose(); }
  });
  it("lists a graph of thousands of pages with a screenful of rows and one inventory read, whatever the user does", async () => {
    const { host, read, readModified, otherCalls, dispose } = await mountAllPages();
    // The count is the full graph (pages only: the two journals are hidden)...
    expect(host.querySelector(".all-pages-count")!.textContent).toBe(`${TOTAL} pages`);
    // ...but only a window of rows exists. An unwindowed list has TOTAL rows here.
    expect(host.querySelectorAll(".all-pages-row").length).toBeLessThan(60);
    expect(rowNames(host)[0]).toBe("Page 00000");

    // Scrolling to the end reaches the last page, with the same few rows in the DOM.
    const scroller = host.querySelector<HTMLElement>(".all-pages-scroll")!;
    scroller.scrollTop = TOTAL * ALL_PAGES_ROW_HEIGHT;
    scroller.dispatchEvent(new Event("scroll"));
    await settle();
    expect(rowNames(host).at(-1)).toBe(`Page ${String(TOTAL - 1).padStart(5, "0")}`);
    expect(host.querySelectorAll(".all-pages-row").length).toBeLessThan(60);

    // Sorting, filtering and toggling journals are local: still one inventory read, no other call.
    column(host, "Page").click();
    column(host, "Journal date").click();
    const filter = host.querySelector<HTMLInputElement>(".all-pages-filter")!;
    filter.value = "Page 0042";
    filter.dispatchEvent(new Event("input", { bubbles: true }));
    host.querySelector<HTMLInputElement>(".all-pages-journals input")!.click();
    await settle();
    expect(read).toHaveBeenCalledTimes(1);
    expect(readModified).toHaveBeenCalledTimes(1);
    expect(otherCalls()).toEqual([]);
    dispose();
  });

  it("opens with the most recently updated page first and shows each time, a page with none last", async () => {
    const at = (h: number) => new Date(2026, 5, 15, h, 0).getTime() / 1000;
    const { host, dispose } = await mountAllPages(
      [existing("Old"), existing("Never"), existing("New")],
      [["pages/Old.md", at(8)], ["pages/New.md", at(20)]],
    );
    await vi.waitFor(() => expect(rowNames(host)).toEqual(["New", "Old", "Never"]));
    expect(column(host, "Updated").getAttribute("aria-sort")).toBe("descending");
    expect([...host.querySelectorAll(".all-pages-row .all-pages-updated")].map((el) => el.textContent))
      .toEqual(["2026-06-15 20:00", "2026-06-15 08:00", "—"]);
    column(host, "Updated").click();
    await settle();
    expect(rowNames(host)).toEqual(["Old", "New", "Never"]);
    dispose();
  });

  it("still lists every page when the times cannot be read", async () => {
    const { host, readModified, dispose } = await mountAllPages([existing("Beta"), existing("Alpha")], "fail");
    await settle();
    expect(readModified).toHaveBeenCalledTimes(1);
    expect(rowNames(host)).toEqual(["Alpha", "Beta"]);
    expect([...host.querySelectorAll(".all-pages-row .all-pages-updated")].map((el) => el.textContent)).toEqual(["—", "—"]);
    dispose();
  });

  it("sorts by title (either way) and by journal day, and a click on the active column flips it", async () => {
    const { host, dispose } = await mountAllPages([existing("beta"), existing("Alpha"), existing("gamma"), journalEntry(20260101), journalEntry(20260301)]);
    expect(rowNames(host)).toEqual(["Alpha", "beta", "gamma"]);
    column(host, "Page").click();
    await settle();
    expect(rowNames(host)).toEqual(["Alpha", "beta", "gamma"]);
    expect(column(host, "Page").getAttribute("aria-sort")).toBe("ascending");
    column(host, "Page").click();
    await settle();
    expect(rowNames(host)).toEqual(["gamma", "beta", "Alpha"]);
    expect(column(host, "Page").getAttribute("aria-sort")).toBe("descending");

    host.querySelector<HTMLInputElement>(".all-pages-journals input")!.click();
    await settle();
    column(host, "Journal date").click();
    await settle();
    expect(rowNames(host).slice(0, 2)).toEqual(["Journal 20260301", "Journal 20260101"]);
    expect([...host.querySelectorAll(".all-pages-row .all-pages-day")].slice(0, 2).map((el) => el.textContent)).toEqual(["2026-03-01", "2026-01-01"]);
    // Pages with no journal day show a dash, after every journal.
    expect([...host.querySelectorAll(".all-pages-row .all-pages-day")].at(-1)!.textContent).toBe("—");
    column(host, "Journal date").click();
    await settle();
    expect(rowNames(host).slice(0, 2)).toEqual(["Journal 20260101", "Journal 20260301"]);
    dispose();
  });

  it("filters by name, and says so when nothing matches", async () => {
    const { host, dispose } = await mountAllPages([existing("Alpha"), existing("alphabet"), existing("beta")]);
    const filter = host.querySelector<HTMLInputElement>(".all-pages-filter")!;
    filter.value = "ALPHA";
    filter.dispatchEvent(new Event("input", { bubbles: true }));
    await settle();
    expect(rowNames(host)).toEqual(["Alpha", "alphabet"]);
    expect(host.querySelector(".all-pages-count")!.textContent).toBe("2 of 3 pages");
    filter.value = "zzz";
    filter.dispatchEvent(new Event("input", { bubbles: true }));
    await settle();
    expect(rowNames(host)).toEqual([]);
    expect(host.querySelector(".all-pages-empty")!.textContent).toBe("No page names match.");
    dispose();
  });

  it("opens a row's own file with the ordinary link gestures", async () => {
    const { host, router, dispose } = await mountAllPages([existing("Alpha"), existing("beta")]);
    const first = host.querySelector<HTMLElement>(".all-pages-open")!;
    first.click();
    expect(router.openPageTarget).toHaveBeenCalledWith({ name: "Alpha", pageKind: "page", path: "pages/Alpha.md" });
    first.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true }));
    expect(router.openPageTargetInNewTab).toHaveBeenCalledWith({ name: "Alpha", pageKind: "page", path: "pages/Alpha.md" });
    dispose();
  });
});

describe("the sidebar entry", () => {
  it("opens the full view from the All pages section", async () => {
    vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [existing("Alpha")] });
    bumpGraphEpoch();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <Sidebar />, host);
    const header = [...host.querySelectorAll<HTMLElement>(".nav-section-header")].find((el) => el.textContent?.includes("ALL PAGES"))!;
    header.click();
    const entry = await vi.waitFor(() => {
      const el = host.querySelector<HTMLElement>(".nav-all-pages-open");
      expect(el).not.toBeNull();
      return el!;
    });
    entry.click();
    expect(route()).toEqual({ kind: "all-pages" });
    dispose();
  });
});
