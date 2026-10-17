import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { Sidebar } from "./Sidebar";
import { WorkspaceSwitcher } from "./WorkspaceSwitcher";
import { closeSettings, openSettings } from "../ui";
import { commandDefaults, installKeybindings, paletteCommands } from "../keybindings";
import { backend } from "../backend";
import { refreshPageIndex, resetPageIndex } from "../pageIndex";
import { MAIN_WINDOW_ID, mainWindow, registerWindow, setActiveWindowId } from "../windowRealm";
import { setActiveLocale } from "../i18n";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
let dispose = () => {};
afterEach(() => { dispose(); closeSettings(); resetPageIndex(); localStorage.clear(); vi.restoreAllMocks(); document.body.innerHTML = ""; });
function mount(view: () => import("solid-js").JSX.Element) {
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(view, root); return root;
}

it("keeps numbered and resize commands bindable without palette clutter", () => {
  const ids = [...Array.from({ length: 9 }, (_, i) => `tab/activate-${i + 1}`), ...Array.from({ length: 9 }, (_, i) => `pane/focus-${i + 1}`),
    "sidebar/grow-width", "sidebar/shrink-width", "right-sidebar/grow-width", "right-sidebar/shrink-width", "pane/grow-width", "pane/shrink-width", "pane/grow-height", "pane/shrink-height"];
  for (const id of ids) {
    expect(commandDefaults().some((c) => c.id === id)).toBe(true);
    expect(paletteCommands().some((c) => c.id === id)).toBe(false);
  }
  expect(paletteCommands().some((c) => c.id === "pane/focus-left")).toBe(true);
  expect(paletteCommands().some((c) => c.id === "task/carry-n")).toBe(true);
  for (const id of ["task/carry-7", "task/carry-30", "task/carry-365"]) expect(commandDefaults().some((c) => c.id === id)).toBe(false);
});

it("diagnoses ignored unknown shortcut ids including retired carry presets", () => {
  const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  const stop = installKeybindings({ "task/carry-30": "mod+shift+7", "missing/command": "mod+shift+8" });
  try { expect(warn).toHaveBeenCalledWith("[shortcuts] Ignoring unknown command ids; check shortcut configuration."); }
  finally { stop(); }
});

it("starts Appearance with Themes and reveals rare controls through search", async () => {
  const root = mount(() => <Settings />); openSettings("appearance"); await tick();
  expect(root.querySelector(".settings-pane-body")?.firstElementChild?.textContent).toBe("Themes");
  expect(root.textContent).not.toContain("Fine-tuning");
  for (const query of ["custom css", "global css", "developer tools", "embedded blocks", "standard page width"]) {
    const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = query; search.dispatchEvent(new InputEvent("input", { bubbles: true })); await tick();
    const result = [...root.querySelectorAll<HTMLButtonElement>(".settings-search-result")].find((button) => button.textContent?.includes("Advanced"))!;
    expect(result, query).toBeDefined(); result.click(); await tick();
    expect(root.querySelector(".settings-advanced-toggle")?.getAttribute("aria-expanded")).toBe("true");
  }
});

it("hides empty Namespaces and follows live graph inventory", async () => {
  const inventory = vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [] });
  const root = mount(() => <Sidebar />); await refreshPageIndex();
  expect(root.textContent?.toLowerCase()).not.toContain("namespaces");
  inventory.mockResolvedValue({ rev: "2", entries: [{ key: "project/roadmap", name: "Project/Roadmap", is_journal: false, day: null, target: { kind: "absent", id: "pages/Project___Roadmap.md" } }] });
  await refreshPageIndex(); expect(root.textContent?.toLowerCase()).toContain("namespaces");
  inventory.mockResolvedValue({ rev: "3", entries: [] });
  await refreshPageIndex(); expect(root.textContent?.toLowerCase()).not.toContain("namespaces");
});

it.each([false, true])("opens the switcher's same new-workspace form from Commands (compact=%s)", async (compact) => {
  const root = mount(() => <WorkspaceSwitcher compact={compact} />);
  const command = paletteCommands().find((c) => c.id === "workspace/new");
  expect(command?.label).toBe("New workspace…"); command!.run(); await tick();
  expect(root.querySelector('[aria-label="New workspace name"]')).not.toBeNull();
  expect(root.querySelector(".workspace-switcher-mark")?.textContent).toBe("W");
});

it("offers saved-workspace creation in the main window where its switcher lives", () => {
  const unregisterMain = registerWindow(MAIN_WINDOW_ID, mainWindow);
  const frame = document.createElement("iframe"); document.body.append(frame);
  const unregisterPopup = registerWindow("census-popup", frame.contentWindow!);
  try {
    setActiveWindowId("census-popup");
    expect(paletteCommands().some((command) => command.id === "workspace/new")).toBe(false);
    setActiveWindowId(MAIN_WINDOW_ID);
    expect(paletteCommands().some((command) => command.id === "workspace/new")).toBe(true);
  } finally { unregisterPopup(); unregisterMain(); }
});

it("localizes the new workspace command in Simplified Chinese", async () => {
  try {
    await setActiveLocale("zh-CN");
    expect(paletteCommands().find((command) => command.id === "workspace/new")?.label).toBe("新建工作区…");
  } finally { await setActiveLocale("en"); }
});
