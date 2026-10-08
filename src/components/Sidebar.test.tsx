import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setFavorites, setRecentPages } from "../ui";
import { setToasts, toasts } from "../toasts";
import { GraphSwitcher, Sidebar } from "./Sidebar";

afterEach(() => {
  setFavorites([]);
  setRecentPages([]);
  setToasts([]);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("graph open recovery (master 9a9122b1544d)", () => {
  it("keeps a known graph's open failure sticky and retries its same target", async () => {
    vi.spyOn(backend(), "listKnownGraphs").mockResolvedValue([
      { name: "Shared notes", path: "/graphs/shared-notes" },
    ]);
    const openKnown = vi.fn(async () => {
      throw new Error("graph root is not readable");
    });
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <GraphSwitcher actions={{
      openKnown,
      openPicked: vi.fn(async () => ({ kind: "aborted" as const })),
      createNew: vi.fn(async () => ({ kind: "aborted" as const })),
    }} />, root);

    try {
      (root.querySelector(".graph-switch-btn") as HTMLButtonElement).click();
      const row = await vi.waitFor(() => {
        const current = root.querySelector<HTMLElement>(".graph-switch-row");
        expect(current).not.toBeNull();
        return current!;
      });
      row.click();
      await vi.waitFor(() => expect(toasts()).toHaveLength(1));
      const failure = toasts()[0]!;
      expect(failure.kind).toBe("error");
      expect(failure.message).toContain("graph root is not readable");
      expect(failure.sticky).toBe(true);
      expect(failure.action?.label).toBe("Retry");

      failure.action!.run();
      await vi.waitFor(() => expect(openKnown).toHaveBeenCalledTimes(2));
      expect(openKnown).toHaveBeenLastCalledWith("/graphs/shared-notes", false);
    } finally {
      dispose();
    }
  });
});

describe("left sidebar section disclosures", () => {
  it("collapses Favorites and Recent independently with semantic controls and counts", async () => {
    setFavorites([
      { name: "Favorite one", kind: "page" },
      { name: "Favorite two", kind: "page" },
    ]);
    setRecentPages([{ name: "Recent one", kind: "page" }]);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <Sidebar />, root);

    try {
      const favorites = await vi.waitFor(() => {
        const control = root.querySelector<HTMLButtonElement>('[data-sidebar-section="favorites"]');
        expect(control).not.toBeNull();
        return control!;
      });
      const recent = root.querySelector<HTMLButtonElement>('[data-sidebar-section="recent"]');
      expect(recent).not.toBeNull();
      expect(favorites.tagName).toBe("BUTTON");
      expect(favorites.getAttribute("aria-expanded")).toBe("true");
      expect(favorites.textContent).toContain("2");
      expect(recent!.getAttribute("aria-expanded")).toBe("true");
      expect(recent!.textContent).toContain("1");
      expect(root.textContent).toContain("Favorite one");
      expect(root.textContent).toContain("Recent one");

      favorites.click();
      expect(favorites.getAttribute("aria-expanded")).toBe("false");
      expect(root.textContent).not.toContain("Favorite one");
      expect(root.textContent).toContain("Recent one");

      recent!.click();
      expect(recent!.getAttribute("aria-expanded")).toBe("false");
      expect(root.textContent).not.toContain("Recent one");

      setFavorites([]);
      setRecentPages([]);
      expect(root.querySelector('[data-sidebar-section="favorites"]')).toBeNull();
      expect(root.querySelector('[data-sidebar-section="recent"]')).toBeNull();
    } finally {
      dispose();
    }
  });
});
