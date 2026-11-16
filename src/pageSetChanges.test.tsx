import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
vi.mock("./warmCache", () => ({ waitForWarmCache: async () => true }));
import { backend, type GraphAnswersChange, type GraphChange } from "./backend";
import { allPageNames, allPages, installPageIndex, resetPageIndex } from "./pageIndex";
import { applyGraphChange, applyGraphChangesBulk, resetStore } from "./document";
import { bumpGraphEpoch } from "./graphSession";
import { resetTabsToJournals } from "./router";
import type { PaneRouter } from "./router";
import type { PageInventoryEntry } from "./types";
import { AllPages } from "./components/AllPages";

// GH #543 R15-10 (queue card g8baTc): the frontend page lists (All pages, the
// quick switcher / search name list, the left-sidebar lists) are views of ONE
// cache, `pageIndex.ts`, refetched on a graph binding and on a page-inventory
// revision. Every way the page SET can change must reach it, driven through the
// entry points App wires to the native events (`applyGraphChange`,
// `applyGraphChangesBulk`) and the graph-bind path (`resetPageIndex` + epoch).
// The backend side of the same cases is crates/tine-store/tests/page_set_changes.rs.

type Disk = { files: string[]; aliases: string[]; refs: string[]; unreadable: string[] };
let disk: Disk;
let rev = 0;
const entry = (name: string, kind: "existing" | "alias" | "absent"): PageInventoryEntry => ({
  key: name.toLowerCase(), name, is_journal: false, day: null,
  target: kind === "existing" ? { kind, id: `pages/${name}.md`, others: [] }
    : kind === "alias" ? { kind, owners: ["pages/A.md"] } : { kind, id: `pages/${name}.md` },
});
const answers = (inventoryChanged: boolean): GraphAnswersChange => ({ rev: String(++rev + 1000), inventoryChanged, blockRefCounts: {} });
const outside = (name: string, extra: Partial<GraphChange> = {}): GraphChange =>
  ({ name, kind: "page", path: `pages/${name}.md`, created: false, removed: false, ...extra });
const settle = async () => { for (let i = 0; i < 8; i++) await new Promise((resolve) => setTimeout(resolve, 0)); };
const listed = () => (allPages() ?? []).map((p) => p.name);

beforeEach(async () => {
  resetStore(); vi.restoreAllMocks(); rev = 0;
  disk = { files: ["A"], aliases: [], refs: [], unreadable: [] };
  vi.spyOn(backend(), "pageInventory").mockImplementation(async () => ({
    rev: String(++rev),
    entries: [...disk.files.map((n) => entry(n, "existing")), ...disk.aliases.map((n) => entry(n, "alias")),
      ...disk.refs.map((n) => entry(n, "absent"))],
    unreadable: disk.unreadable.length ? [...disk.unreadable] : undefined,
  }));
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
  vi.spyOn(backend(), "pageModified").mockResolvedValue({ rev: "1", modified: [] });
  vi.spyOn(backend(), "pageBacklinkCounts").mockResolvedValue([]);
  installPageIndex(); bumpGraphEpoch();
  await vi.waitFor(() => expect(listed()).toEqual(["A"]));
  vi.mocked(backend().pageInventory).mockClear();
});
afterEach(() => { resetPageIndex(); resetTabsToJournals(); vi.restoreAllMocks(); document.body.innerHTML = ""; });

describe("the page set changes under a bound graph", () => {
  it("M1: a page deleted while Tine was closed is not listed after the graph binds", async () => {
    disk.files = ["A", "Gone"]; resetPageIndex(); bumpGraphEpoch();
    await vi.waitFor(() => expect(listed()).toEqual(["A", "Gone"]));
    // Quit; the page is deleted outside; launch binds the graph again (graph.ts: resetPageIndex, then the epoch).
    disk.files = ["A"]; resetPageIndex(); bumpGraphEpoch();
    await vi.waitFor(() => expect(listed()).toEqual(["A"]));
    expect(allPageNames()).toEqual(["A"]);
  });

  it("M1b: a rebind that keeps the old list until the new one lands never keeps the deleted page", async () => {
    disk.files = ["A", "Gone"]; bumpGraphEpoch();
    await vi.waitFor(() => expect(listed()).toEqual(["A", "Gone"]));
    disk.files = ["A"]; bumpGraphEpoch();
    await vi.waitFor(() => expect(listed()).toEqual(["A"]));
  });

  it("M2: an unreadable page repaired by an outside editor appears (a modify, not a create)", async () => {
    disk.unreadable = ["pages/Other.md"]; disk.files = ["A"];
    resetPageIndex(); bumpGraphEpoch();
    await vi.waitFor(() => expect(backend().pageInventory).toHaveBeenCalled());
    await settle();
    expect(listed()).toEqual(["A"]);
    disk.unreadable = []; disk.files = ["A", "Target"];
    // The watcher reports the repaired file as an ordinary modification whose
    // answers say the inventory changed (store `unreadable != old.unreadable`).
    await applyGraphChange(outside("Target", { path: "pages/Other.md", answers: answers(true) }));
    await vi.waitFor(() => expect(listed()).toEqual(["A", "Target"]));
  });

  it("M2b: the repair arrives with no page event at all (answers-only bulk)", async () => {
    disk.files = ["A", "Target"];
    await applyGraphChangesBulk({ changes: [], answers: answers(true) });
    await vi.waitFor(() => expect(listed()).toEqual(["A", "Target"]));
  });

  it("an outside rename (a removed and a created event, answers on the first) swaps the name", async () => {
    disk.files = ["Old"]; resetPageIndex(); bumpGraphEpoch();
    await vi.waitFor(() => expect(listed()).toEqual(["Old"]));
    disk.files = ["New"];
    const a = answers(true);
    await applyGraphChange(outside("Old", { removed: true, answers: a }));
    await applyGraphChange(outside("New", { created: true }));
    await vi.waitFor(() => expect(listed()).toEqual(["New"]));
  });

  it("an outside rename whose answers were lost still swaps the name (created/removed bump)", async () => {
    disk.files = ["Old"]; resetPageIndex(); bumpGraphEpoch();
    await vi.waitFor(() => expect(listed()).toEqual(["Old"]));
    disk.files = ["New"];
    await applyGraphChange(outside("Old", { removed: true }));
    await vi.waitFor(() => expect(listed()).toEqual(["New"]));
  });

  it("a page delivered by sync while open is listed", async () => {
    disk.files = ["A", "Synced"];
    await applyGraphChange(outside("Synced", { created: true, answers: answers(true) }));
    await vi.waitFor(() => expect(listed()).toEqual(["A", "Synced"]));
  });

  it("a sync checkout (bulk, over 32 pages) lists every created page", async () => {
    const names = Array.from({ length: 40 }, (_, i) => `Bulk ${String(i).padStart(2, "0")}`);
    disk.files = ["A", ...names];
    await applyGraphChangesBulk({ changes: names.map((n) => outside(n, { created: true })), answers: answers(true) });
    await vi.waitFor(() => expect(listed()).toEqual(["A", ...names]));
  });

  it("an alias-only edit lists the alias in the name list, not in All pages", async () => {
    disk.aliases = ["Nick"];
    await applyGraphChange(outside("A", { answers: answers(true) }));
    await vi.waitFor(() => expect(allPageNames()).toEqual(["A", "Nick"]));
    expect(listed()).toEqual(["A"]);
    disk.aliases = [];
    await applyGraphChange(outside("A", { answers: answers(true) }));
    await vi.waitFor(() => expect(allPageNames()).toEqual(["A"]));
  });

  it("a namespace-only (reference-only) edit lists the new name", async () => {
    disk.refs = ["ns/child"];
    await applyGraphChange(outside("A", { answers: answers(true) }));
    await vi.waitFor(() => expect(allPageNames()).toEqual(["A", "ns/child"]));
    expect(listed()).toEqual(["A"]);
    disk.refs = [];
    await applyGraphChange(outside("A", { answers: answers(true) }));
    await vi.waitFor(() => expect(allPageNames()).toEqual(["A"]));
  });

  it("a text-only modification costs no inventory read (I-25)", async () => {
    await applyGraphChange(outside("A", { answers: answers(false) }));
    await applyGraphChangesBulk({ changes: [outside("A")], answers: answers(false) });
    await settle();
    expect(backend().pageInventory).not.toHaveBeenCalled();
  });

  it("the All pages view shows the right set after each kind of change", async () => {
    const router = { openPageTarget: vi.fn(), openPageTargetInNewTab: vi.fn() } as unknown as PaneRouter;
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <AllPages router={router} />, host);
    const rows = () => [...host.querySelectorAll(".all-pages-open")].map((n) => n.textContent);
    try {
      await vi.waitFor(() => expect(rows()).toEqual(["A"]));
      disk.files = ["A", "Synced"];
      await applyGraphChange(outside("Synced", { created: true, answers: answers(true) }));
      await vi.waitFor(() => expect(rows()).toEqual(["A", "Synced"]));
      disk.files = ["Synced", "Renamed"];
      await applyGraphChange(outside("A", { removed: true, answers: answers(true) }));
      await applyGraphChange(outside("Renamed", { created: true }));
      await vi.waitFor(() => expect(rows()).toEqual(["Renamed", "Synced"]));
      disk.files = ["Renamed"]; disk.unreadable = ["pages/Bad.md"];
      await applyGraphChange(outside("Synced", { removed: true, answers: answers(true) }));
      await vi.waitFor(() => expect(rows()).toEqual(["Renamed"]));
    } finally { dispose(); }
  });
});
