import { afterAll, expect, it, vi } from "vitest";

vi.hoisted(() => vi.stubGlobal("navigator", { platform: "MacIntel", userAgent: "Mac OS X" }));
import { commandDefaults, eventToBindingString } from "./keybindings";

afterAll(() => vi.unstubAllGlobals());

it("accepts OG's Cmd+[ / Cmd+] as macOS history shortcuts (GH #780 comment 1)", () => {
  const event = (key: string, code: string) => ({ key, code, metaKey: true, ctrlKey: false, altKey: false, shiftKey: false }) as KeyboardEvent;
  const defaults = commandDefaults();
  expect(defaults.find(command => command.id === "go/backward")?.alias).toBe(eventToBindingString(event("[", "BracketLeft")));
  expect(defaults.find(command => command.id === "go/forward")?.alias).toBe(eventToBindingString(event("]", "BracketRight")));
});
