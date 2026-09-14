import { describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { installKeybindings, setKeybindingsSuspended } from "../keybindings";
import {
  closeSettings,
  closeSwitcher,
  openSettings,
  setShortcutOverrides,
  settingsOpen,
  shortcutOverrides,
  switcherOpen,
} from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";
import { Settings } from "./Settings";

const SHORTCUTS_KEY = "logseq-claude.shortcuts";
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function keydown(init: KeyboardEventInit & { composing?: boolean; keyCode?: number }): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  if (init.composing) Object.defineProperty(event, "isComposing", { value: true });
  if (init.keyCode != null) Object.defineProperty(event, "keyCode", { value: init.keyCode });
  return event;
}

describe("GH #161 P1A1-F2 Settings shortcut recording", () => {
  it("routes the existing Settings search to shortcut commands on that tab (GH #380)", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    try {
      openSettings("shortcuts");
      await tick();
      const input = root.querySelector<HTMLInputElement>(".settings-search-input")!;
      expect(input.placeholder).toBe("Search shortcuts...");
      input.value = "Find in page";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await tick();

      expect(root.querySelector(".settings-search-results")).toBeNull();
      const ids = [...root.querySelectorAll<HTMLElement>(".help-shortcut-id")]
        .map((element) => element.textContent);
      expect(ids).toEqual(["go/find-in-page"]);

      input.value = "no-such-shortcut";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await tick();
      expect(root.querySelector(".help-shortcut-search-empty")?.textContent).toBe("No matching shortcuts");
    } finally {
      dispose();
      clearTransientLayersForTest();
      closeSettings();
      root.remove();
    }
  });

  it("keeps ordinary recording reachable after the earlier global capture listener declines suspended input", async () => {
    const priorOverrides = { ...shortcutOverrides() };
    const priorStorage = localStorage.getItem(SHORTCUTS_KEY);
    const root = document.createElement("div");
    let dispose: (() => void) | undefined;
    let uninstall: (() => void) | undefined;

    try {
      // Exercise and later restore the absent-key persistence case independently
      // of whichever local state an enclosing test run started with.
      setShortcutOverrides({});
      localStorage.removeItem(SHORTCUTS_KEY);
      clearTransientLayersForTest();
      closeSettings();
      closeSwitcher();

      // App installs this listener before a later Settings keycap click starts
      // the recorder, so this is the production capture-listener order.
      uninstall = installKeybindings();
      document.body.append(root);
      dispose = render(() => <Settings />, root);
      openSettings("shortcuts");
      await tick();

      const keycap = () => [...root.querySelectorAll<HTMLElement>(".help-shortcut-row")]
        .find((candidate) => candidate.querySelector(".help-shortcut-id")?.textContent === "go/find-in-page")
        ?.querySelector<HTMLButtonElement>(".help-keycap-button");
      expect(keycap()).toBeDefined();
      expect(localStorage.getItem(SHORTCUTS_KEY)).toBeNull();

      const beginRecording = async () => {
        keycap()!.click();
        await tick();
        expect(keycap()!.textContent).toBe("Press keys...");
      };
      const cancelRecording = async () => {
        keycap()!.click();
        await tick();
        expect(keycap()!.textContent).not.toBe("Press keys...");
      };

      await beginRecording();
      const ordinary = keydown({ key: "k", code: "KeyK", ctrlKey: true });
      keycap()!.dispatchEvent(ordinary);
      await tick();

      expect(switcherOpen()).toBe(false);
      expect(shortcutOverrides()).toEqual({ "go/find-in-page": "mod+k" });
      expect(keycap()!.textContent).toBe("mod+k");
      expect(ordinary.defaultPrevented).toBe(true);
      expect(localStorage.getItem(SHORTCUTS_KEY)).toBe(JSON.stringify({ "go/find-in-page": "mod+k" }));

      const row = keycap()!.closest<HTMLElement>(".help-shortcut-row")!;
      const unbind = [...row.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Unbind");
      expect(unbind).toBeDefined();
      unbind!.click();
      await tick();

      expect(shortcutOverrides()).toEqual({ "go/find-in-page": "false" });
      expect(keycap()!.textContent).toBe("Unbound");
      expect(localStorage.getItem(SHORTCUTS_KEY)).toBe(JSON.stringify({ "go/find-in-page": "false" }));

      const reset = [...keycap()!.closest<HTMLElement>(".help-shortcut-row")!
        .querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.trim() === "Reset");
      expect(reset).toBeDefined();
      reset!.click();
      await tick();
      expect(shortcutOverrides()).toEqual({});
      expect(keycap()!.textContent).toBe("mod+f");
      expect(localStorage.getItem(SHORTCUTS_KEY)).toBe("{}");

      // Restore the remap for the recorder cancellation assertions below.
      keycap()!.click();
      await tick();
      keycap()!.dispatchEvent(keydown({ key: "k", code: "KeyK", ctrlKey: true }));
      await tick();

      const recorded = { ...shortcutOverrides() };
      await beginRecording();
      const modifier = keydown({ key: "Control", code: "ControlLeft", ctrlKey: true });
      keycap()!.dispatchEvent(modifier);
      await tick();
      expect(keycap()!.textContent).toBe("Press keys...");
      expect(shortcutOverrides()).toEqual(recorded);
      expect(modifier.defaultPrevented).toBe(true);
      await cancelRecording();

      await beginRecording();
      const composing = keydown({ key: "j", code: "KeyJ", ctrlKey: true, composing: true });
      keycap()!.dispatchEvent(composing);
      await tick();
      expect(keycap()!.textContent).toBe("Press keys...");
      expect(shortcutOverrides()).toEqual(recorded);
      expect(composing.defaultPrevented).toBe(false);
      await cancelRecording();

      await beginRecording();
      const legacyIme = keydown({ key: "j", code: "KeyJ", ctrlKey: true, keyCode: 229 });
      keycap()!.dispatchEvent(legacyIme);
      await tick();
      expect(keycap()!.textContent).toBe("Press keys...");
      expect(shortcutOverrides()).toEqual(recorded);
      expect(legacyIme.defaultPrevented).toBe(false);
      await cancelRecording();

      await beginRecording();
      const escape = keydown({ key: "Escape", code: "Escape" });
      keycap()!.dispatchEvent(escape);
      await tick();
      expect(escape.defaultPrevented).toBe(true);
      expect(keycap()!.textContent).toBe("mod+k");
      expect(shortcutOverrides()).toEqual(recorded);
      expect(settingsOpen()).toBe(true);
      expect(root.querySelector(".settings-modal")).not.toBeNull();
    } finally {
      // Dispose Settings before test-only transient cleanup: it owns both the
      // recording listener and its registered Settings Escape rung.
      dispose?.();
      clearTransientLayersForTest();
      window.dispatchEvent(new Event("blur"));
      uninstall?.();
      closeSettings();
      closeSwitcher();
      setKeybindingsSuspended(false);
      root.remove();
      setShortcutOverrides(priorOverrides);
      if (priorStorage == null) localStorage.removeItem(SHORTCUTS_KEY);
      else localStorage.setItem(SHORTCUTS_KEY, priorStorage);
    }
  });
});
