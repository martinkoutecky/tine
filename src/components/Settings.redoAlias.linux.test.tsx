import { afterEach, expect, it, vi } from "vitest";

// The keymap reads the platform once, when it loads: be Linux from the start.
vi.hoisted(() => Object.defineProperty(navigator, "platform", { configurable: true, value: "Linux x86_64" }));

import { render } from "solid-js/web";
import { createEffect, onCleanup } from "solid-js";
import { installKeybindings } from "../keybindings";
import { closeSettings, openSettings, setShortcutOverrides, shortcutOverrides } from "../ui";
import { graphMeta, setGraphMeta } from "../graphSession";
import { clearTransientLayersForTest } from "../transientLayers";
import { Settings } from "./Settings";

// GH #1044 review, off macOS (Settings.redoAlias.mac.test.tsx): the same rule for Ctrl+Y.
// Settings → Shortcuts shows exactly the Redo keys dispatch honours. Dispatch takes the graph's
// config.edn shortcuts merged with the device's overrides (App.tsx), so a Redo binding in either drops the Ctrl+Y alias
// from both.

const tick = () => new Promise(r => setTimeout(r, 0));
let dispose = () => {};
afterEach(() => {
  dispose(); dispose = () => {};
  clearTransientLayersForTest(); closeSettings(); setShortcutOverrides({}); setGraphMeta(null);
  localStorage.removeItem("logseq-claude.shortcuts");
  document.body.innerHTML = "";
});

/** Settings open on Shortcuts, with keybindings installed as App installs them. Returns the times the Redo row shows
 *  ctrl+y (as its binding or as its alias) and whether Ctrl+Y reaches a command. */
async function redoRow(cfg: Record<string, string>, device: Record<string, string> = {}) {
  setGraphMeta({ root: "/graph", pages_dir: "pages", journals_dir: "journals", assets_dir: "assets", shortcuts: cfg } as any);
  setShortcutOverrides(device);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => {
    createEffect(() => { const cleanup = installKeybindings({ ...(graphMeta()?.shortcuts ?? {}), ...shortcutOverrides() }); onCleanup(cleanup); });
    return <Settings />;
  }, root);
  openSettings("shortcuts"); await tick();
  const row = [...root.querySelectorAll<HTMLElement>(".help-shortcut-row")].find(e => e.querySelector(".help-shortcut-id")?.textContent === "editor/redo")!;
  const keys = row.querySelector(".help-shortcut-keys")!.textContent ?? "";
  const shown = keys.split("ctrl+y").length - 1;
  const press = new KeyboardEvent("keydown", { key: "y", code: "KeyY", ctrlKey: true, bubbles: true, cancelable: true });
  document.body.dispatchEvent(press);
  return { shown, dispatched: press.defaultPrevented };
}

it("with no Redo binding, Ctrl+Y is shown once and redoes", async () => {
  expect(await redoRow({})).toEqual({ shown: 1, dispatched: true });
});

it.each([
  ["a remap", "mod+alt+r"],
  ["false", "false"],
  ["an empty binding", ""],
  ["an explicit mod+shift+z", "mod+shift+z"],
])("a graph config.edn Redo binding of %s hides Ctrl+Y, which then does nothing", async (_label, binding) => {
  expect(await redoRow({ "editor/redo": binding })).toEqual({ shown: 0, dispatched: false });
});

it("a graph config.edn Redo binding of ctrl+y shows Ctrl+Y once", async () => {
  expect(await redoRow({ "editor/redo": "ctrl+y" })).toEqual({ shown: 1, dispatched: true });
});

it("a device Redo remap hides Ctrl+Y too", async () => {
  expect(await redoRow({}, { "editor/redo": "mod+alt+r" })).toEqual({ shown: 0, dispatched: false });
});
