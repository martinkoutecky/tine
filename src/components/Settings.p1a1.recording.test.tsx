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
