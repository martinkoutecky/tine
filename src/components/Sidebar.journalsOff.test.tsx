// F9 (D-20261028-UILP; OG container.cljs `home-nav`): `:feature/enable-journals? false`
// swaps the sidebar's Journals entry for the configured home page.
import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Sidebar } from "./Sidebar";
import { setGraphMeta } from "../graphSession";
import { openJournals, route } from "../router";
import { backend } from "../backend";
import type { GraphMeta, PageInventory } from "../types";
import { refreshPageIndex, resetPageIndex } from "../pageIndex";
import { pageIdentityKey } from "../ui";

const inventory = (...names: string[]): PageInventory => ({ rev: "1", entries: names.map((name) => ({
  key: pageIdentityKey(name), name, is_journal: false, day: null, target: { kind: "existing", id: `pages/${name}.md`, others: [] },
})) } as unknown as PageInventory);
async function loadPages(...names: string[]) {
  resetPageIndex();
  vi.spyOn(backend(), "pageInventory").mockResolvedValue(inventory(...names));
  await refreshPageIndex();
}

const meta = (patch: Partial<GraphMeta>) => setGraphMeta({ ...patch } as GraphMeta);
function mount(): { root: HTMLElement; dispose: () => void } {
  const root = document.body.appendChild(document.createElement("div"));
  return { root, dispose: render(() => <Sidebar />, root) };
}
const navTitles = (root: HTMLElement) => [...root.querySelectorAll<HTMLElement>(".nav-contents > .nav-item")].map((el) => el.textContent?.trim());

afterEach(() => {
  resetPageIndex();
  setGraphMeta(null);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("journals switched off", () => {
  it("keeps Journals when the graph has journals on, whether or not it names a home page", () => {
    meta({ enable_journals: true, default_home: "Directory" });
    const { root, dispose } = mount();
    try { expect(navTitles(root)[0]).toBe("Journals"); expect(root.querySelector(".home-nav")).toBeNull(); } finally { dispose(); }
  });
  it("shows the home page instead of Journals, and opens it", async () => {
    meta({ enable_journals: false, default_home: "Directory" });
    await loadPages("Directory");
    vi.spyOn(backend(), "getPage").mockResolvedValue({ id: "h", name: "Directory", kind: "page", title: "Directory", pre_block: null, blocks: [], rev: "r", format: "md" } as never);
    openJournals();
    const { root, dispose } = mount();
    try {
      expect(navTitles(root)[0]).toBe("Directory");
      expect(navTitles(root)).not.toContain("Journals");
      root.querySelector<HTMLElement>(".home-nav")!.click();
      await vi.waitFor(() => expect(route()).toMatchObject({ kind: "page", name: "Directory" }));
    } finally { dispose(); }
  });
  it("keeps Journals when the configured home page does not exist (OG get-default-home-if-valid)", async () => {
    meta({ enable_journals: false, default_home: "Missing" });
    await loadPages("Other");
    const { root, dispose } = mount();
    try { expect(navTitles(root)[0]).toBe("Journals"); } finally { dispose(); }
  });
  it("leaves Journals reachable when journals are off but no home page is configured (OG)", () => {
    meta({ enable_journals: false, default_home: null });
    const { root, dispose } = mount();
    try { expect(navTitles(root)[0]).toBe("Journals"); } finally { dispose(); }
  });
});
