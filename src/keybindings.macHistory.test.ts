import { afterAll, expect, it, vi } from "vitest";

vi.hoisted(() => vi.stubGlobal("navigator", { platform: "MacIntel", userAgent: "Mac OS X" }));
import { commandDefaults, eventToBindingString } from "./keybindings";

afterAll(() => vi.unstubAllGlobals());

it("binds OG's Cmd+[ / Cmd+] as the macOS history shortcuts, leaving Option+arrows to the editor (GH #780 comment 1, GH #1040)", () => {
  const event = (key: string, code: string) => ({ key, code, metaKey: true, ctrlKey: false, altKey: false, shiftKey: false }) as KeyboardEvent;
  const defaults = commandDefaults();
  expect(defaults.find(command => command.id === "go/backward")).toMatchObject({ binding: eventToBindingString(event("[", "BracketLeft")), alias: undefined });
  expect(defaults.find(command => command.id === "go/forward")).toMatchObject({ binding: eventToBindingString(event("]", "BracketRight")), alias: undefined });
});
