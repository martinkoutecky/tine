import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { bumpGraphEpoch } from "../graphSession";
import { resetPageIndex } from "../pageIndex";
import { resetTabsToJournals, route } from "../router";
import type { PageInventoryEntry } from "../types";
import { resetLeftSidebarSections, setLeftSidebarOpen } from "../ui";
import { Sidebar } from "./Sidebar";

// GH #335 (D-20261101-L5): the sidebar Namespaces section KEEPS its inline tree and
// gains one entry that opens the full Namespaces view in the main pane; the phone
// drawer closes after it, like every other successful destination.

const existing = (name: string): PageInventoryEntry => ({
  key: name.toLowerCase(), name, is_journal: false, day: null,
  target: { kind: "existing", id: `pages/${name.replaceAll("/", "___")}.md`, others: [] },
});

afterEach(() => {
  resetPageIndex();
  resetTabsToJournals();
  resetLeftSidebarSections();
  setLeftSidebarOpen(false);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("the sidebar Namespaces section", () => {
  it("keeps its inline tree, offers the full view only when expanded, and completes navigation when it opens", async () => {
    vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [
      ...["Project/Roadmap", "Project/Notes"].map(existing),
      // The namespace parent, as the backend lists it (CM step 2 batch 10).
      { key: "project", name: "Project", is_journal: false, day: null, namespace_parent: true, target: { kind: "absent", id: "pages/Project.md" } },
    ] });
    bumpGraphEpoch();
    const completion = vi.fn();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Sidebar onActiveNavigationComplete={completion} />, root);
    try {
      const header = await vi.waitFor(() => {
        const found = [...root.querySelectorAll<HTMLElement>(".nav-section-header")].find((el) => el.textContent?.includes("Namespaces"));
        expect(found).toBeDefined();
        return found!;
      });
      expect(header.classList.contains("nav-first-level")).toBe(true);
      // At rest: no extra control, no tree.
      expect(root.querySelector(".nav-namespaces-open")).toBeNull();
      header.click();
      await vi.waitFor(() => expect(root.querySelector(".ns-tree")).not.toBeNull());
      // The inline expansion is still there, and the entry sits with it.
      expect([...root.querySelectorAll(".ns-node-label")].map((el) => el.textContent?.trim())).toContain("Project");
      const entry = root.querySelector<HTMLButtonElement>(".nav-namespaces-open")!;
      expect(entry.textContent).toBe("Open the full view…");
      expect(completion).not.toHaveBeenCalled();
      entry.click();
      expect(route()).toEqual({ kind: "namespaces" });
      expect(completion).toHaveBeenCalledTimes(1);
      await vi.waitFor(() => expect(root.querySelector(".nav-namespaces-open")!.classList.contains("active")).toBe(true));
    } finally { dispose(); }
  });
});
