import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { decodeToolbarChoice, placeToolbarActions, saveToolbarChoice, toolbarActions } from "./toolbarChoice";

afterEach(async () => { saveToolbarChoice(null); await Promise.resolve(); vi.restoreAllMocks(); vi.doUnmock("./backend"); });
const available = toolbarActions.map(action => action.id);
const options = { available, tabs: false, windowControls: false, workspace: false, budget: 64, costs: {} };

describe("device toolbar choice (#926)", () => {
  it("ignores unknown ids, gives bar priority, preserves both orders, appends new actions", () => {
    expect(decodeToolbarChoice('{"bar":["settings","future","theme","settings"],"menu":["theme","journals","calendar"]}')).toEqual({
      bar: ["settings", "theme"], menu: ["journals", "calendar", "share", "right-sidebar", "workspaces"],
    });
    expect(decodeToolbarChoice("")).toBeNull();
  });
  it("overflows the rightmost chosen buttons first and never duplicates a visible button", () => {
    const choice = decodeToolbarChoice('{"bar":["settings","theme","calendar"],"menu":["journals","share","right-sidebar","workspaces"]}')!;
    const wide = placeToolbarActions(choice, 900, { ...options, budget: 96 });
    const narrow = placeToolbarActions(choice, 390, options);
    expect(wide.bar).toEqual(["settings", "theme", "calendar"]);
    expect(narrow.bar).toEqual(["settings", "theme"]);
    expect(narrow.menu).toEqual(["calendar", "journals", "share", "right-sidebar", "workspaces"]);
    expect(narrow.bar.filter(id => narrow.menu.includes(id))).toEqual([]);
  });
  it("saves once, restores after a reload, and Reset deletes the device record", async () => {
    const stored = new Map<string, string>();
    const write = vi.spyOn(backend(), "setAppString").mockImplementation(async (key, value) => { if (value === null) stored.delete(key); else stored.set(key, value); });
    const read = vi.spyOn(backend(), "getAppString").mockImplementation(async (key, fallback) => stored.get(key) ?? fallback);
    const choice = decodeToolbarChoice('{"bar":["settings"],"menu":["theme","journals","share","calendar","right-sidebar","workspaces"]}')!;
    saveToolbarChoice(choice);
    await vi.waitFor(() => expect(write).toHaveBeenCalledTimes(1));
    const api = backend();
    vi.doMock("./backend", () => ({ backend: () => api }));
    vi.resetModules();
    const restarted = await import("./toolbarChoice");
    expect(restarted.toolbarChoice()).toBeNull();
    await restarted.initToolbarChoice();
    expect(restarted.toolbarChoice()).toEqual(choice);
    expect(read).toHaveBeenCalledTimes(1);
    restarted.saveToolbarChoice(null);
    await vi.waitFor(() => expect(write).toHaveBeenLastCalledWith("toolbar_actions", null));
    expect(stored.has("toolbar_actions")).toBe(false);
    await restarted.initToolbarChoice();
    expect(restarted.toolbarChoice()).toBeNull();
  });
  it("resolves the seven-button placement in bounded time", () => {
    const start = performance.now();
    for (let i = 0; i < 1000; i++) placeToolbarActions(null, 362, options);
    expect(performance.now() - start).toBeLessThan(50);
  });
});
