import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { bumpGraphEpoch } from "../graphSession";
import { resetPageIndex } from "../pageIndex";
import { resetTabsToJournals } from "../router";
import type { PaneRouter } from "../router";
import { resetLeftSidebarSections, setLeftSidebarOpen } from "../ui";
import type { PageInventoryEntry } from "../types";
import { Namespaces, NAMESPACES_ROW_HEIGHT } from "./Namespaces";

// GH #335 (D-20261101-L5): the full Namespaces view. The user outcomes: a namespaced
// graph of thousands of pages is browsable as a tree with a screenful of rows, expand
// all / collapse all and per-namespace counts work, a filter shows each match with its
// full path, a row opens its page by the ordinary link gestures, and none of it reads
// the backend beyond the shared page inventory (and the windowed icon batch).

const existing = (name: string): PageInventoryEntry => ({
  key: name.toLowerCase(), name, is_journal: false, day: null,
  target: { kind: "existing", id: `pages/${name.replaceAll("/", "___")}.md`, others: [] },
});
// A namespace parent as the backend lists it (CM step 2 batch 10, model rule 1):
// an absent entry that opens itself, flagged `namespace_parent`.
const parent = (name: string): PageInventoryEntry => ({
  key: name.toLowerCase(), name, is_journal: false, day: null, namespace_parent: true,
  target: { kind: "absent", id: `pages/${name.replaceAll("/", "___")}.md` },
});
const settle = async () => { for (let i = 0; i < 6; i++) await new Promise((resolve) => setTimeout(resolve, 0)); };
// EmojiText may draw an emoji as an image; compare the user's logical label.
const logical = (node: Node): string => node.nodeType === 3 ? node.textContent ?? "" : (node as Element).tagName === "IMG" ? (node as Element).getAttribute("alt") ?? "" : [...node.childNodes].map(logical).join("");
const rowLabels = (host: HTMLElement) => [...host.querySelectorAll(".ns-view-open")].map(logical);
const counts = (host: HTMLElement) => [...host.querySelectorAll(".ns-view-count")].map((el) => el.textContent);
const button = (host: HTMLElement, label: string) =>
  [...host.querySelectorAll<HTMLButtonElement>(".ns-view-action")].find((el) => el.textContent === label)!;
const setFilter = async (host: HTMLElement, text: string) => {
  const filter = host.querySelector<HTMLInputElement>(".all-pages-filter")!;
  filter.value = text;
  filter.dispatchEvent(new Event("input", { bubbles: true }));
  await settle();
};

const SMALL_FILES = ["Project", "Project/Roadmap", "Project/Roadmap/2026", "Project/Notes", "Archive/Old/Notes", "Archive/Old/Plans", "Other/Notes", "Plain"];
const SMALL = () => [...SMALL_FILES.map(existing), ...["Archive", "Archive/Old", "Other"].map(parent)];
const BIG = 4000;
const big = () => [
  ...Array.from({ length: BIG }, (_, i) => existing(`Area ${String(i % 20).padStart(2, "0")}/Page ${String(i).padStart(5, "0")}`)),
  existing("Plain"),
  ...Array.from({ length: 20 }, (_, i) => parent(`Area ${String(i).padStart(2, "0")}`)),
];

function stubRouter() {
  return { openPageTarget: vi.fn(), openPageTargetInNewTab: vi.fn() } as unknown as PaneRouter & {
    openPageTarget: ReturnType<typeof vi.fn>; openPageTargetInNewTab: ReturnType<typeof vi.fn>;
  };
}

/** Graph calls apart from the shared inventory/metadata reads, the ownership stamp and
 * the windowed icon batch. The router's debounced saveSession persists tabs only. */
function spyOnEverythingElse() {
  const spies: { name: string; spy: ReturnType<typeof vi.fn> }[] = [];
  const target = backend() as unknown as Record<string, unknown>;
  const names = new Set<string>();
  for (let proto: object | null = target; proto && proto !== Object.prototype; proto = Object.getPrototypeOf(proto))
    for (const key of Object.getOwnPropertyNames(proto)) names.add(key);
  for (const key of names) {
    if (key === "constructor" || key === "pageInventory" || key === "graphBindingGeneration" || key === "saveSession" || key === "pageIcons" || typeof target[key] !== "function") continue;
    spies.push({ name: key, spy: vi.spyOn(target as never, key as never) as never });
  }
  return () => spies.flatMap(({ name, spy }) => spy.mock.calls.map(() => name));
}

async function mountNamespaces(entries: PageInventoryEntry[], icons: Record<string, string> = {}) {
  const read = vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries });
  const otherCalls = spyOnEverythingElse();
  const iconRead = vi.spyOn(backend(), "pageIcons").mockImplementation(async (names: string[]) =>
    Object.fromEntries(names.filter((name) => name in icons).map((name) => [name, icons[name]])));
  bumpGraphEpoch();
  const router = stubRouter();
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <Namespaces router={router} />, host);
  await vi.waitFor(() => expect(host.querySelector(".ns-view-open")).not.toBeNull());
  return { host, router, read, otherCalls, iconRead, dispose };
}

afterEach(() => {
  resetPageIndex();
  resetTabsToJournals();
  resetLeftSidebarSections();
  setLeftSidebarOpen(false);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("the Namespaces view", () => {
  it("opens with the top-level namespaces expanded one level and shows each namespace's size", async () => {
    const { host, dispose } = await mountNamespaces(SMALL());
    try {
      expect(rowLabels(host)).toEqual(["Archive", "Old", "Other", "Notes", "Project", "Notes", "Roadmap"]);
      // Archive and Archive/Old have no file of their own but are pages (namespace
      // parents, model rule 1); sizes count the pages below.
      expect(counts(host)).toEqual(["3", "2", "1", "", "3", "", "1"]);
      expect(host.querySelector(".all-pages-count")!.textContent).toBe("10 pages in 5 namespaces");
      const rows = [...host.querySelectorAll(".ns-view-row")];
      expect(rows.map((row) => row.getAttribute("aria-level"))).toEqual(["1", "2", "1", "2", "1", "2", "2"]);
      expect(rows[0].getAttribute("aria-expanded")).toBe("true");
      expect(rows[3].getAttribute("aria-expanded")).toBeNull(); // Other/Notes is a leaf
    } finally { dispose(); }
  });

  it("expands and collapses a namespace from its disclosure, and all at once", async () => {
    const { host, dispose } = await mountNamespaces(SMALL());
    try {
      host.querySelector<HTMLButtonElement>('.ns-view-row[aria-level="1"] .ns-view-toggle')!.click(); // Archive
      await settle();
      expect(rowLabels(host)).toEqual(["Archive", "Other", "Notes", "Project", "Notes", "Roadmap"]);
      button(host, "Collapse all").click();
      await settle();
      expect(rowLabels(host)).toEqual(["Archive", "Other", "Project"]);
      button(host, "Expand all").click();
      await settle();
      expect(rowLabels(host)).toEqual(["Archive", "Old", "Notes", "Plans", "Other", "Notes", "Project", "Notes", "Roadmap", "2026"]);
      const toggle = host.querySelector<HTMLButtonElement>(".ns-view-toggle")!;
      expect(toggle.getAttribute("aria-label")).toBe("Collapse Archive");
    } finally { dispose(); }
  });

  it("filters by full path, lists each match with the path above it, and returns to the remembered tree", async () => {
    const { host, dispose } = await mountNamespaces(SMALL());
    try {
      button(host, "Collapse all").click();
      await settle();
      await setFilter(host, "notes");
      // Three different "Notes" pages, told apart by where they live; no tree toggles meanwhile.
      expect([...host.querySelectorAll(".ns-view-name")].map((el) => el.textContent?.replace(/\s+/g, ""))).toEqual(["Archive/Old/Notes", "Other/Notes", "Project/Notes"]);
      expect(host.querySelector(".ns-view-toggle")).toBeNull();
      expect(button(host, "Expand all").disabled).toBe(true);
      expect(host.querySelector(".all-pages-count")!.textContent).toBe("3 matches");
      await setFilter(host, "ROADMAP");
      expect(host.querySelector(".all-pages-count")!.textContent).toBe("2 matches");
      await setFilter(host, "no such name");
      expect(host.querySelector(".all-pages-empty")!.textContent).toBe("No namespace or page name matches.");
      await setFilter(host, "");
      expect(rowLabels(host)).toEqual(["Archive", "Other", "Project"]); // collapse all survived the filter
      expect(button(host, "Expand all").disabled).toBe(false);
    } finally { dispose(); }
  });

  it("opens a namespace by the ordinary link gestures", async () => {
    const { host, router, dispose } = await mountNamespaces(SMALL());
    try {
      const rows = [...host.querySelectorAll<HTMLElement>(".ns-view-open")];
      const project = rows.find((el) => el.title === "Project")!;
      project.click();
      expect(router.openPageTarget).toHaveBeenCalledWith({ name: "Project", pageKind: "page" });
      const notes = rows.find((el) => el.title === "Project/Notes")!;
      notes.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true }));
      expect(router.openPageTargetInNewTab).toHaveBeenCalledWith({ name: "Project/Notes", pageKind: "page" });
    } finally { dispose(); }
  });

  it("shows a page's icon before its name", async () => {
    const { host, dispose } = await mountNamespaces(SMALL(), { Project: "🚩" });
    try {
      await vi.waitFor(() => expect(rowLabels(host)).toContain("🚩 Project"));
    } finally { dispose(); }
  });

  it("says so when the graph has no namespaced page", async () => {
    const { host, dispose } = await (async () => {
      vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [existing("Plain")] });
      bumpGraphEpoch();
      const host = document.createElement("div");
      document.body.append(host);
      return { host, dispose: render(() => <Namespaces router={stubRouter()} />, host) };
    })();
    try {
      await vi.waitFor(() => expect(host.querySelector(".all-pages-empty")).not.toBeNull());
      expect(host.querySelector(".all-pages-empty")!.textContent).toContain("No namespaced pages yet");
    } finally { dispose(); }
  });

  it("draws a screenful of rows for thousands of pages and reads the inventory once, whatever the user does", async () => {
    const { host, read, otherCalls, iconRead, dispose } = await mountNamespaces(big());
    try {
      await settle();
      button(host, "Expand all").click();
      await settle();
      const total = 20 + BIG; // twenty areas and their pages
      expect(host.querySelectorAll(".ns-view-row").length).toBeLessThan(60);
      const spacer = host.querySelector<HTMLElement>(".all-pages-spacer")!;
      expect(spacer.style.height).toBe(`${total * NAMESPACES_ROW_HEIGHT}px`);
      // Scrolling to the end reaches the last page with the same few rows in the DOM.
      const scroller = host.querySelector<HTMLElement>(".all-pages-scroll")!;
      scroller.scrollTop = total * NAMESPACES_ROW_HEIGHT;
      scroller.dispatchEvent(new Event("scroll"));
      await settle();
      expect(rowLabels(host).at(-1)).toBe(`Page ${String(BIG - 1).padStart(5, "0")}`);
      expect(host.querySelectorAll(".ns-view-row").length).toBeLessThan(60);
      // Collapse, expand, filter and clear are local: no further inventory read and no other call.
      button(host, "Collapse all").click();
      await setFilter(host, "page 0042");
      expect(rowLabels(host).length).toBe(10); // Page 00420..00429
      await setFilter(host, "");
      expect(read).toHaveBeenCalledTimes(1);
      expect(otherCalls()).toEqual([]);
      for (const [names] of iconRead.mock.calls) expect(names.length).toBeLessThan(60);
      expect(iconRead.mock.calls.length).toBeLessThanOrEqual(8);
    } finally { dispose(); }
  });
});
