// A failed known-graph list must not read as "you have no other graphs" (master c5279d1865).
import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { GraphSwitcher } from "./Sidebar";

afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });
const tick = async (n = 5) => { for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0)); };

describe("graph switcher failed list", () => {
  it("says it could not load the other graphs instead of an empty list", async () => {
    vi.spyOn(backend(), "listKnownGraphs").mockRejectedValue(new Error("boom"));
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <GraphSwitcher actions={{
      openKnown: async () => ({ kind: "already_current" }) as never,
      openPicked: async () => ({ kind: "already_current" }) as never,
      createNew: async () => ({ kind: "already_current" }) as never,
    }} />, root);
    try {
      await tick();
      root.querySelector<HTMLButtonElement>(".graph-switch-btn")!.click();
      await tick();
      expect(root.querySelector(".graph-switch-menu")).not.toBeNull(); // precondition: menu rendered
      expect(root.textContent ?? "").toMatch(/Couldn.t load/);
    } finally { dispose(); }
  });
});
