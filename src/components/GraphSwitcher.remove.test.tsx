// GH #1015: switching graphs is frequent and removing one is rare, so the switcher's rows carry no remove
// control; a graph leaves the remembered list only through its row actions (right-click here, All graphs).
import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { closeContextMenu, contextMenu } from "../ui";
import { dismissTopTransient } from "../transientLayers";
import { GraphSwitcher } from "./Sidebar";

afterEach(() => { closeContextMenu(); vi.restoreAllMocks(); document.body.innerHTML = ""; });

function mountSwitcher() {
  const openKnown = vi.fn(async () => ({ kind: "already_current" as const, root: "/work" }));
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <GraphSwitcher actions={{
    openKnown,
    openPicked: vi.fn(async () => ({ kind: "aborted" as const })),
    createNew: vi.fn(async () => ({ kind: "aborted" as const })),
  }} />, root);
  return { root, dispose, openKnown };
}

async function openRows(root: HTMLElement): Promise<HTMLElement[]> {
  root.querySelector<HTMLButtonElement>(".graph-switch-btn")!.click();
  return vi.waitFor(() => {
    const rows = [...root.querySelectorAll<HTMLElement>(".graph-switch-row")];
    expect(rows).toHaveLength(2);
    return rows;
  });
}

describe("graph switcher rows (GH #1015)", () => {
  it("have no remove control: a click anywhere on a row switches and never forgets the graph", async () => {
    vi.spyOn(backend(), "listKnownGraphs").mockResolvedValue([
      { name: "Work", path: "/work" }, { name: "Home", path: "/home" },
    ]);
    const forget = vi.spyOn(backend(), "forgetKnownGraph").mockResolvedValue(undefined);
    const { root, dispose, openKnown } = mountSwitcher();
    try {
      const rows = await openRows(root);
      for (const row of rows) {
        expect(row.querySelector("button")).toBeNull();
        expect(row.textContent).not.toContain("×");
        expect(row.querySelector("[aria-label]")).toBeNull();
      }
      // The switcher still opens All graphs, where removing lives.
      expect([...root.querySelectorAll(".graph-switch-menu > button")].map(b => b.textContent)).toContain("All graphs");
      // The only control inside a row is the row itself; clicking its last child lands on the row.
      (rows[1]!.lastElementChild as HTMLElement).click();
      await vi.waitFor(() => expect(openKnown).toHaveBeenCalledWith("/home", false));
      expect(forget).not.toHaveBeenCalled();
    } finally { dispose(); }
  });

  it("still removes a graph through the right-click row actions and refreshes the list", async () => {
    const list = vi.spyOn(backend(), "listKnownGraphs")
      .mockResolvedValueOnce([{ name: "Work", path: "/work" }, { name: "Home", path: "/home" }])
      .mockResolvedValue([{ name: "Work", path: "/work" }]);
    const forget = vi.spyOn(backend(), "forgetKnownGraph").mockResolvedValue(undefined);
    const { root, dispose } = mountSwitcher();
    try {
      const rows = await openRows(root);
      rows[1]!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 5, clientY: 6 }));
      const menu = contextMenu();
      expect(menu?.kind).toBe("action-menu");
      const items = menu?.kind === "action-menu" ? menu.items : [];
      const remove = items.find(item => item.label === "Remove from this list");
      expect(remove?.danger).toBe(true);
      remove!.run!();
      await vi.waitFor(() => expect(forget).toHaveBeenCalledWith("/home"));
      await vi.waitFor(() => expect(root.querySelectorAll(".graph-switch-row")).toHaveLength(1));
      expect(list.mock.calls.length).toBeGreaterThanOrEqual(2);
    } finally { dispose(); }
  });

  it("keeps Escape closing the switcher list", async () => {
    vi.spyOn(backend(), "listKnownGraphs").mockResolvedValue([
      { name: "Work", path: "/work" }, { name: "Home", path: "/home" },
    ]);
    const { root, dispose } = mountSwitcher();
    try {
      await openRows(root);
      expect(dismissTopTransient("escape")).toBe(true);
      await vi.waitFor(() => expect(root.querySelector(".graph-switch-menu")).toBeNull());
    } finally { dispose(); }
  });
});
