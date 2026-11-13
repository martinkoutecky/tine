import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { bumpDataRev, bumpGraphEpoch } from "../graphSession";
import { patchPageIndex, resetPageIndex } from "../pageIndex";
import { resetTabsToJournals, route } from "../router";
import type { PaneRouter } from "../router";
import { resetLeftSidebarSections, setLeftSidebarOpen } from "../ui";
import type { PageInventoryEntry } from "../types";
import { noteQueryInput, QUERY_QUIET_MS } from "../querySubscription";
import { AllPages, ALL_PAGES_ROW_HEIGHT } from "./AllPages";
import { Sidebar } from "./Sidebar";
import { applyGraphAnswers } from "../graphAnswers";

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
// EmojiText may draw an emoji as an image; compare the user's logical label.
const logical = (node: Node): string => node.nodeType === 3 ? node.textContent ?? "" : (node as Element).tagName === "IMG" ? (node as Element).getAttribute("alt") ?? "" : [...node.childNodes].map(logical).join("");
const rowNames = (host: HTMLElement) => [...host.querySelectorAll(".all-pages-open")].map(logical);
const column = (host: HTMLElement, label: string) =>
  [...host.querySelectorAll<HTMLElement>(".all-pages-col")].find((el) => el.textContent?.startsWith(label))!;

function stubRouter() {
  return { openPageTarget: vi.fn(), openPageTargetInNewTab: vi.fn() } as unknown as PaneRouter & {
    openPageTarget: ReturnType<typeof vi.fn>; openPageTargetInNewTab: ReturnType<typeof vi.fn>;
  };
}

/** Graph calls apart from the shared inventory/metadata reads, the ownership
 * stamp and the windowed icon batch (checked separately). The router's
 * debounced saveSession may land from test navigation or teardown; it persists
 * tabs and cannot read or write a page row. */
function spyOnEverythingElse() {
  const spies: { name: string; spy: ReturnType<typeof vi.fn> }[] = [];
  const target = backend() as unknown as Record<string, unknown>;
  const names = new Set<string>();
  for (let proto: object | null = target; proto && proto !== Object.prototype; proto = Object.getPrototypeOf(proto))
    for (const key of Object.getOwnPropertyNames(proto)) names.add(key);
  for (const key of names) {
    if (key === "constructor" || key === "pageInventory" || key === "pageModified" || key === "pageBacklinkCounts" || key === "graphBindingGeneration" || key === "saveSession" || key === "pageIcons" || typeof target[key] !== "function") continue;
    spies.push({ name: key, spy: vi.spyOn(target as never, key as never) as never });
  }
  return () => spies.flatMap(({ name, spy }) => spy.mock.calls.map(() => name));
}

async function mountAllPages(entries = inventory(), modified: [string, number][] | "fail" = [], backlinks: [string, number][] = [], references: [string, number][] = []) {
  const read = vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries });
  const readModified = vi.spyOn(backend(), "pageModified");
  if (modified === "fail") readModified.mockRejectedValue(new Error("no times"));
  else readModified.mockResolvedValue({ rev: "1", modified });
  const readCounts = vi.spyOn(backend(), "pageBacklinkCounts").mockResolvedValue([backlinks, references, "1"]);
  const otherCalls = spyOnEverythingElse();
  const icons = vi.spyOn(backend(), "pageIcons").mockImplementation(async (names: string[]) =>
    Object.fromEntries(names.filter((name) => name === "Page 00001").map((name) => [name, "🚩"])));
  bumpGraphEpoch();
  const router = stubRouter();
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <AllPages router={router} />, host);
  await vi.waitFor(() => expect(host.querySelector(".all-pages-open")).not.toBeNull());
  return { host, router, read, readModified, readCounts, otherCalls, icons, dispose };
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
  it("orders metric publications per target, retaining deletion revisions", async () => {
    const { host, readCounts, readModified, dispose } = await mountAllPages(
      [existing("Alpha"), existing("Beta")], [], [["pages/Alpha.md", 1], ["pages/Beta.md", 1]]);
    try {
      await settle();
      const publish = (rev: string, rows: [string, number | null][]) => applyGraphAnswers({
        rev, inventoryChanged: false, blockRefCounts: {}, pageModified: rows, pageBacklinkCounts: rows,
      });
      publish("5", [["pages/Beta.md", 5]]);
      publish("4", [["pages/Alpha.md", 4], ["pages/Beta.md", 4]]);
      expect(rowNames(host)).toEqual(["Beta", "Alpha"]);
      expect([...host.querySelectorAll(".all-pages-backlinks")].map(el => el.textContent)).toEqual(["5", "4"]);
      publish("7", [["pages/Beta.md", null]]);
      publish("6", [["pages/Alpha.md", 6], ["pages/Beta.md", 6]]);
      expect(rowNames(host)).toEqual(["Alpha", "Beta"]);
      expect([...host.querySelectorAll(".all-pages-backlinks")].map(el => el.textContent)).toEqual(["6", "—"]);
      expect(readCounts).toHaveBeenCalledTimes(1);
      expect(readModified).toHaveBeenCalledTimes(1);
      // An unrelated newer row also cannot hide an older full invalidation.
      readCounts.mockResolvedValue([[["pages/Alpha.md", 6], ["pages/Beta.md", 8]], [], "8"]);
      applyGraphAnswers({ rev: "5", inventoryChanged: false, blockRefCounts: {}, pageBacklinkCounts: null });
      await vi.waitFor(() => expect([...host.querySelectorAll(".all-pages-backlinks")].map(el => el.textContent)).toEqual(["6", "8"]));
      expect(readCounts).toHaveBeenCalledTimes(2);
      expect(readModified).toHaveBeenCalledTimes(1);
    } finally { dispose(); }
  });
  it("repairs the cold seed/publication race when its notification arrives after the seed", async () => {
    const { host, readCounts, readModified, dispose } = await mountAllPages([existing("Alpha")], [], [["pages/Alpha.md", 1]]);
    try {
      await vi.waitFor(() => expect(host.querySelector(".all-pages-backlinks")!.textContent).toBe("1"));
      readCounts.mockResolvedValue([[["pages/Alpha.md", 4]], [], "2"]);
      applyGraphAnswers({ rev: "2", inventoryChanged: false, blockRefCounts: {}, pageModified: [], pageBacklinkCounts: null });
      await vi.waitFor(() => expect(host.querySelector(".all-pages-backlinks")!.textContent).toBe("4"));
      expect(readCounts).toHaveBeenCalledTimes(2);
      expect(readModified).toHaveBeenCalledTimes(1);
    } finally { dispose(); }
  });
  it("rejects previous-graph late metrics and replays rows newer than a seed", async () => {
    let finishTimes!: (value: import("../types").PageModified) => void;
    let finishCounts!: (value: import("../types").PageBacklinkCounts) => void;
    const inventoryRead = vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [existing("Old")] });
    vi.spyOn(backend(), "pageIcons").mockResolvedValue({});
    const times = vi.spyOn(backend(), "pageModified").mockImplementationOnce(() => new Promise(resolve => { finishTimes = resolve; }));
    const counts = vi.spyOn(backend(), "pageBacklinkCounts").mockImplementationOnce(() => new Promise(resolve => { finishCounts = resolve; }));
    bumpGraphEpoch();
    const host = document.createElement("div"); document.body.append(host);
    const dispose = render(() => <AllPages router={stubRouter()} />, host);
    try {
      await vi.waitFor(() => expect(counts).toHaveBeenCalledTimes(1));
      inventoryRead.mockResolvedValue({ rev: "1", entries: [existing("New"), existing("Other")] });
      times.mockResolvedValue({ rev: "1", modified: [["pages/New.md", 1]] });
      let finishNew!: (value: import("../types").PageBacklinkCounts) => void;
      counts.mockImplementationOnce(() => new Promise(resolve => { finishNew = resolve; }));
      bumpGraphEpoch();
      await vi.waitFor(() => expect(counts).toHaveBeenCalledTimes(2));
      applyGraphAnswers({ rev: "2", inventoryChanged: false, blockRefCounts: {},
        pageModified: [["pages/Other.md", 2]], pageBacklinkCounts: [["pages/New.md", 8]] });
      finishNew([[["pages/New.md", 1], ["pages/Other.md", 3]], [], "1"]);
      column(host, "Backlinks").click();
      await vi.waitFor(() => expect(rowNames(host)).toEqual(["New", "Other"]));
      expect([...host.querySelectorAll(".all-pages-backlinks")].map(el => el.textContent)).toEqual(["8", "3"]);
      finishTimes({ rev: "99", modified: [["pages/Other.md", 99]] });
      finishCounts([[["pages/Other.md", 99]], [], "99"]);
      await settle();
      expect(rowNames(host)).toEqual(["New", "Other"]);
      expect([...host.querySelectorAll(".all-pages-backlinks")].map(el => el.textContent)).toEqual(["8", "3"]);
      expect(counts).toHaveBeenCalledTimes(2);
      expect(times).toHaveBeenCalledTimes(2);
    } finally { dispose(); }
  });
  it("shares metrics between panes and patches publications without full reads", async () => {
    const { host, readModified, readCounts, dispose } = await mountAllPages([existing("Alpha"), existing("Beta")], [], [["pages/Alpha.md", 2], ["pages/Beta.md", 7]]);
    const other = document.createElement("div"); document.body.append(other);
    const disposeOther = render(() => <AllPages router={stubRouter()} />, other);
    try {
      await settle();
      expect(readModified).toHaveBeenCalledTimes(1);
      expect(readCounts).toHaveBeenCalledTimes(1);
      column(host, "Backlinks").click(); column(other, "Backlinks").click();
      applyGraphAnswers({ rev: "2", inventoryChanged: false, blockRefCounts: {},
        pageModified: [["pages/Alpha.md", 1781553600]], pageBacklinkCounts: [] } as Parameters<typeof applyGraphAnswers>[0]);
      bumpDataRev();
      await settle();
      expect(readModified).toHaveBeenCalledTimes(1);
      expect(readCounts).toHaveBeenCalledTimes(1);
      expect(host.querySelector(".all-pages-updated")!.textContent).toBe("—"); // Beta still sorts first.
      applyGraphAnswers({ rev: "3", inventoryChanged: false, blockRefCounts: {},
        pageModified: [["pages/Alpha.md", 1781553601]], pageBacklinkCounts: [["pages/Alpha.md", 9]] } as Parameters<typeof applyGraphAnswers>[0]);
      await vi.waitFor(() => expect(rowNames(host)).toEqual(["Alpha", "Beta"]));
      expect(rowNames(other)).toEqual(["Alpha", "Beta"]);
      expect(readModified).toHaveBeenCalledTimes(1);
      expect(readCounts).toHaveBeenCalledTimes(1);
    } finally { disposeOther(); dispose(); }
  });
  it("lists and opens the virtual journal beside a same-title ordinary file", async () => {
    const name = "2026_10_23";
    const { host, router, dispose } = await mountAllPages([
      existing(name),
      { key: name, name, is_journal: true, day: 20261023,
        target: { kind: "absent", id: "journals/2026_10_23.md" },
        link: { name: "2026-10-23", kind: "journal" } },
    ]);
    try {
      expect(rowNames(host)).toEqual([name]);
      host.querySelector<HTMLInputElement>(".all-pages-journals input")!.click();
      await vi.waitFor(() => expect(rowNames(host)).toEqual([name, name]));
      const buttons = [...host.querySelectorAll<HTMLButtonElement>(".all-pages-open")];
      buttons.find(button => button.title === "")!.click();
      expect(router.openPageTarget).toHaveBeenLastCalledWith({ name: "2026-10-23", pageKind: "journal", path: undefined });
      buttons.find(button => button.title !== "")!.click();
      expect(router.openPageTarget).toHaveBeenLastCalledWith({ name, pageKind: "page", path: `pages/${name}.md` });
    } finally { dispose(); }
  });

  // Publications patch the shared answer even while typing. Neither the
  // typing pause nor a disposed pane schedules another graph-wide read.
  it("keeps one metric seed through typing, quiet, and unmount", async () => {
    const { readCounts, readModified, dispose } = await mountAllPages([existing("Alpha")]);
    await settle();
    expect(readCounts).toHaveBeenCalledTimes(1);
    vi.useFakeTimers();
    try {
      // Typing with saves publishing: no count read until the pause.
      for (let i = 0; i < 5; i++) {
        noteQueryInput(); bumpDataRev();
        await vi.advanceTimersByTimeAsync(QUERY_QUIET_MS / 3);
      }
      expect(readCounts).toHaveBeenCalledTimes(1);
      expect(readModified).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(QUERY_QUIET_MS);
      expect(readCounts).toHaveBeenCalledTimes(1);
      bumpDataRev();
      await vi.advanceTimersByTimeAsync(0);
      expect(readCounts).toHaveBeenCalledTimes(1);
      bumpDataRev(); bumpDataRev();
      await vi.advanceTimersByTimeAsync(2 * QUERY_QUIET_MS);
      expect(readCounts).toHaveBeenCalledTimes(1);
      noteQueryInput(); bumpDataRev();
      dispose();
      await vi.advanceTimersByTimeAsync(2 * QUERY_QUIET_MS);
      expect(readCounts).toHaveBeenCalledTimes(1);
    } finally {
      // The typing clock is app-wide and the fake clock ran ahead of the real
      // one: leave the last input far in the past for the tests after this.
      vi.setSystemTime(0); noteQueryInput();
      vi.useRealTimers();
    }
  });
  it("keeps graph-wide backlink reads separate from cheap updated times", () => {
    expect(typeof (backend() as unknown as Record<string, unknown>).pageBacklinkCounts).toBe("function");
  });
  it("refreshes backend counts after a graph change while keeping the current sort", async () => {
    const { host, readModified, readCounts, dispose } = await mountAllPages([existing("Alpha"), existing("Beta")], [], [["pages/Alpha.md", 2], ["pages/Beta.md", 7]]);
    try {
      column(host, "Backlinks").click();
      await vi.waitFor(() => expect(rowNames(host)).toEqual(["Beta", "Alpha"]));
      applyGraphAnswers({ rev: "2", inventoryChanged: false, blockRefCounts: {},
        pageModified: [], pageBacklinkCounts: [["pages/Alpha.md", 9]] });
      bumpDataRev();
      await vi.waitFor(() => expect(rowNames(host)).toEqual(["Alpha", "Beta"]), { timeout: 2000 });
      expect(readModified).toHaveBeenCalledTimes(1);
      expect(readCounts).toHaveBeenCalledTimes(1);
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
    const { host, read, readModified, otherCalls, icons, dispose } = await mountAllPages();
    // GH #346, as OG's all-pages table (page.cljs:1245): a row shows its page's icon.
    await vi.waitFor(() => expect(rowNames(host)[1]).toBe("🚩 Page 00001"));
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
    // The journal-date column exists only while journals are listed, so toggle first.
    host.querySelector<HTMLInputElement>(".all-pages-journals input")!.click();
    await settle();
    column(host, "Journal date").click();
    const filter = host.querySelector<HTMLInputElement>(".all-pages-filter")!;
    filter.value = "Page 0042";
    filter.dispatchEvent(new Event("input", { bubbles: true }));
    host.querySelector<HTMLInputElement>(".all-pages-journals input")!.click();
    await settle();
    expect(read).toHaveBeenCalledTimes(1);
    expect(readModified).toHaveBeenCalledTimes(1);
    expect(otherCalls()).toEqual([]);
    // Icons are one coalesced read per rendered window, never per row or per graph page.
    for (const [names] of icons.mock.calls) expect(names.length).toBeLessThan(60);
    expect(icons.mock.calls.length).toBeLessThanOrEqual(4);
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

  // The grid has one track per cell (allPages.css); a cell without a track wraps into an
  // implicit second grid row and lands on the next row's first letter. jsdom has no layout, so
  // this pins the cell COUNT per header/row (the layout itself is scripts/shot-all-pages.mjs).
  it("renders the journal-date column only while journals are listed, with the same cells in the header and every row", async () => {
    const { host, dispose } = await mountAllPages([existing("beta"), existing("Alpha"), journalEntry(20260101)]);
    const cells = (el: Element) => el.children.length;
    const heads = () => [...host.querySelectorAll(".all-pages-head")];
    const rows = () => [...host.querySelectorAll(".all-pages-row")];
    const table = () => host.querySelector(".all-pages-table")!;

    // Journals off: page, updated, backlinks. No journal-date header, no day cell, no dash.
    expect(heads().map(cells)).toEqual([3]);
    expect(rows().map(cells)).toEqual([3, 3]);
    expect(host.querySelector(".all-pages-day-col")).toBeNull();
    expect(host.querySelector(".all-pages-day")).toBeNull();
    expect(table().classList.contains("all-pages-table--day")).toBe(false);

    host.querySelector<HTMLInputElement>(".all-pages-journals input")!.click();
    await settle();
    expect(heads().map(cells)).toEqual([4]);
    expect(rows().map(cells)).toEqual([4, 4, 4]);
    expect(host.querySelectorAll(".all-pages-row .all-pages-day").length).toBe(3);
    expect(table().classList.contains("all-pages-table--day")).toBe(true);

    // Hiding journals again drops the column, and a sort by the vanished column does not linger.
    column(host, "Journal date").click();
    await settle();
    expect(column(host, "Journal date").getAttribute("aria-sort")).toBe("descending");
    host.querySelector<HTMLInputElement>(".all-pages-journals input")!.click();
    await settle();
    expect(host.querySelector(".all-pages-day-col")).toBeNull();
    expect(heads().map(cells)).toEqual([3]);
    expect(rows().map(cells)).toEqual([3, 3]);
    expect(column(host, "Updated").getAttribute("aria-sort")).toBe("descending");
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

  // GH #886 (fan-out lane review): an alias-only name patch leaves the listed
  // pages equal, so All Pages keeps its rows and the keyboard focus on them.
  it("keeps the focused row and its focus when a patch only adds an alias", async () => {
    const { host, dispose } = await mountAllPages([existing("Alpha"), existing("beta")]);
    const row = [...host.querySelectorAll<HTMLElement>(".all-pages-open")].find((el) => logical(el) === "Alpha")!;
    row.focus();
    expect(document.activeElement).toBe(row);
    expect(patchPageIndex(2n, { keys: ["shortcut"], entries: [{ key: "shortcut", name: "Shortcut", is_journal: false, day: null, target: { kind: "alias", owners: ["pages/Alpha.md"] } }] })).toEqual({ aliases: true });
    await settle();
    expect(rowNames(host)).toEqual(["Alpha", "beta"]);
    expect(row.isConnected).toBe(true);
    expect(document.activeElement).toBe(row);
    dispose();
  });

  // CM step 2 batch 5 (OG handler/page.cljs `get-all-pages`): a page named
  // like an OG built-in page is not listed even when it has a file.
  it("leaves out a page named like a Logseq built-in page, even one with a file", async () => {
    const { host, dispose } = await mountAllPages([existing("Alpha"), existing("TODO"), existing("card"), existing("Todo list")]);
    await settle();
    // OG upper-cases before its lookup, so its mixed-case member `card` never matches.
    expect(rowNames(host).sort()).toEqual(["Alpha", "Todo list", "card"]);
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
  it("opens the full view directly from the All pages entry", async () => {
    const times = vi.spyOn(backend(), "pageModified").mockResolvedValue({ rev: "1", modified: [] });
    const counts = vi.spyOn(backend(), "pageBacklinkCounts").mockResolvedValue([[], [], "1"]);
    vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [existing("Alpha")] });
    bumpGraphEpoch();
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <Sidebar />, host);
    const entry = await vi.waitFor(() => {
      const el = host.querySelector<HTMLElement>(".nav-all-pages-open");
      expect(el).not.toBeNull();
      return el!;
    });
    expect(times).not.toHaveBeenCalled();
    expect(counts).not.toHaveBeenCalled();
    entry.click();
    expect(route()).toEqual({ kind: "all-pages" });
    dispose();
  });
});
