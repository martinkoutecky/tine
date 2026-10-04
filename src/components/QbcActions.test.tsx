import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { commandDefaults, installKeybindings } from "../keybindings";
import { focusPane, focusedPaneId, paneRouter, resetPaneLayoutToSingle, splitPane } from "../panes";
import { closeSettings, closeSwitcher, openSwitcher, switcherOpen, openSettings, setSidebarWidth, sidebarWidth, setRightSidebarWidth, rightSidebarWidth, setSidebarOpen, setRightSidebarOpen } from "../ui";
import { Settings } from "./Settings";

const ids = ["sidebar/grow-width", "sidebar/shrink-width", "right-sidebar/grow-width", "right-sidebar/shrink-width", "go/search-tab"];
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
afterEach(() => { closeSettings(); closeSwitcher(); resetPaneLayoutToSingle(); localStorage.clear(); document.body.innerHTML = ""; });
function key(key: string) { window.dispatchEvent(new KeyboardEvent("keydown", { key, altKey: true, bubbles: true, cancelable: true })); }

it("GH #425/#437: Settings exposes five unbound remappable commands", async () => {
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("shortcuts"); await tick();
    for (const id of ids) {
      const command = commandDefaults().find(c => c.id === id);
      expect(command, id).toBeDefined();
      expect(command!.binding).toBe("");
      const row = [...root.querySelectorAll(".help-shortcut-row")].find(el => el.textContent?.includes(command!.label));
      expect(row?.textContent).toContain("Unbound");
    }
  } finally { dispose(); }
});

it("GH #425: remapped keys resize both closed sidebars, clamp, and persist", () => {
  setSidebarOpen(false); setRightSidebarOpen(false);
  setSidebarWidth(246); setRightSidebarWidth(360);
  const dispose = installKeybindings(Object.fromEntries(ids.slice(0, 4).map((id, i) => [id, `alt+${i + 1}`])));
  try {
    key("1"); expect(sidebarWidth()).toBe(294);
    key("2"); expect(sidebarWidth()).toBe(246);
    key("3"); expect(rightSidebarWidth()).toBe(408);
    key("4"); expect(rightSidebarWidth()).toBe(360);
    for (let i = 0; i < 30; i++) { key("1"); key("3"); }
    expect(sidebarWidth()).toBe(500); expect(rightSidebarWidth()).toBe(800);
    expect(localStorage.getItem("logseq-claude.sidebarWidth")).toBe("500");
    expect(localStorage.getItem("logseq-claude.rightSidebarWidth")).toBe("800");
    for (let i = 0; i < 30; i++) { key("2"); key("4"); }
    expect(sidebarWidth()).toBe(180); expect(rightSidebarWidth()).toBe(220);
    expect(localStorage.getItem("logseq-claude.sidebarWidth")).toBe("180");
    expect(localStorage.getItem("logseq-claude.rightSidebarWidth")).toBe("220");
  } finally { dispose(); }
});

it("GH #437: a remapped key opens a foreground persistent search tab in the focused pane", async () => {
  const first = focusedPaneId();
  const other = await splitPane(first, "row"); expect(other).toBeTruthy();
  focusPane(other!);
  const firstCount = paneRouter(first).tabs().length;
  const otherCount = paneRouter(other!).tabs().length;
  const dispose = installKeybindings({ "go/search-tab": "alt+s" });
  try {
    openSwitcher();
    key("s");
    expect(paneRouter(other!).route()).toMatchObject({ kind: "query", source: "", presentation: "search" });
    expect(paneRouter(other!).tabs()).toHaveLength(otherCount + 1);
    expect(paneRouter(first).tabs()).toHaveLength(firstCount);
    expect(switcherOpen()).toBe(false);
  } finally { dispose(); }
});
