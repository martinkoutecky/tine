import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { setFavorites, setRecentPages } from "../ui";
import { Sidebar } from "./Sidebar";

afterEach(() => {
  setFavorites([]);
  setRecentPages([]);
  document.body.innerHTML = "";
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
