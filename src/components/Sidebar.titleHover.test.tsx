import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Sidebar } from "./Sidebar";
import { setFavorites, seedFavorites, setRecentPages } from "../ui";

afterEach(() => { setFavorites([]); setRecentPages([]); document.body.innerHTML = ""; });
it("GH #563: favorites and recents reveal full titles only when hovered and truncated", () => {
  const name = "A long page title with 📚 that must remain identifiable";
  seedFavorites([name]); setFavorites([{ name, kind: "page" }]); setRecentPages([{ name, kind: "page" }]);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Sidebar />, root);
  try {
    for (const list of ["sidebar-favorites-list", "sidebar-recent-list"]) {
      const row = root.querySelector<HTMLElement>(`#${list} .nav-page`)!;
      const label = row.querySelector<HTMLElement>(".nav-page-label") ?? row;
      Object.defineProperties(label, { scrollWidth: { configurable: true, value: 400 }, clientWidth: { configurable: true, value: 160 } });
      label.dispatchEvent(new MouseEvent("mouseenter"));
      expect(label.title).toBe(name);
      label.dispatchEvent(new MouseEvent("mouseleave"));
      expect(label.title).toBe("");
      Object.defineProperty(label, "clientWidth", { configurable: true, value: 500 });
      label.dispatchEvent(new MouseEvent("mouseenter"));
      expect(label.title).toBe("");
    }
  } finally { dispose(); }
});
