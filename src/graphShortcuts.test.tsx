import { afterEach, expect, it, vi } from "vitest";

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); });

it.each(["MacIntel", "Linux x86_64", "Win32"])("uses OG defaults without another default binding clash on %s and honors overrides", async platform => {
  vi.resetModules();
  vi.stubGlobal("navigator", { platform });
  const keys = await import("./keybindings");
  const defaults = keys.commandDefaults();
  for (const [id, binding] of [["go/all-graphs", "g shift+g"], ["editor/insert-youtube-timestamp", "mod+shift+y"]]) {
    expect(defaults.filter(command => command.binding === binding).map(command => command.id)).toEqual([id]);
  }
  let dispose = keys.installKeybindings();
  const event = (key: string, shiftKey = true) => ({ key, shiftKey, ctrlKey: platform !== "MacIntel", metaKey: platform === "MacIntel", altKey: false } as KeyboardEvent);
  expect(keys.editorCommandFor(event("Y"))).toBe("editor/insert-youtube-timestamp");
  dispose();
  dispose = keys.installKeybindings({ "editor/insert-youtube-timestamp": "mod+shift+u", "go/all-graphs": "g a" });
  expect(keys.editorCommandFor(event("Y"))).toBeNull();
  expect(keys.editorCommandFor(event("U"))).toBe("editor/insert-youtube-timestamp");
  expect(keys.paletteCommands().find(command => command.id === "go/all-graphs")?.binding).toBe("g a");
  dispose();
});

it("All graphs keeps tab history and round-trips through the existing session parser", async () => {
  const { createPaneRouter, routeTitle } = await import("./router");
  const { parsePersistedSession } = await import("./session");
  const router = createPaneRouter();
  router.openAllGraphs({ inPlace: true });
  expect(router.route()).toEqual({ kind: "all-graphs" });
  expect(routeTitle(router.route())).toBe("All graphs");
  router.goBack();
  expect(router.route()).toEqual({ kind: "journals" });
  router.goForward();
  const snapshot = router.snapshot();
  const parsed = parsePersistedSession(JSON.stringify(snapshot));
  expect(parsed?.snapshots.get("main")?.tabs[0].history).toContainEqual({ kind: "all-graphs" });
});
