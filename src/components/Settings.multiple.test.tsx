import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { createEffect, onCleanup } from "solid-js";
import { installKeybindings } from "../keybindings";
import { closeSettings, openSettings, setShortcutOverrides, shortcutOverrides } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";
import { Settings } from "./Settings";

let dispose = () => {};
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const row = (id: string) => document.querySelector<HTMLElement>(`[data-shortcut-id="${id}"]`)!;
const press = async (key: string) => {
  window.dispatchEvent(new KeyboardEvent("keydown", { key, code: key, bubbles: true, cancelable: true }));
  await tick();
};
function mount() {
  const root = document.createElement("div");
  document.body.append(root);
  dispose = render(() => {
    createEffect(() => { const cleanup = installKeybindings(shortcutOverrides()); onCleanup(cleanup); });
    return <Settings />;
  }, root);
  openSettings("shortcuts");
}
afterEach(() => {
  dispose(); closeSettings(); clearTransientLayersForTest(); setShortcutOverrides({});
  localStorage.removeItem("logseq-claude.shortcuts"); document.body.innerHTML = "";
});

it("adds without losing the default, replaces one alternative, removes one and resets", async () => {
  setShortcutOverrides({}); mount(); await tick();
  row("go/search").querySelector<HTMLButtonElement>('button[title="Add keybinding"]')!.click(); await tick();
  await press("F8");
  expect(shortcutOverrides()["go/search"]).toEqual(["mod+k", "f8"]);
  expect(JSON.parse(localStorage.getItem("logseq-claude.shortcuts")!)["go/search"]).toEqual(["mod+k", "f8"]);
  row("go/search").querySelectorAll<HTMLButtonElement>(".help-keycap-button")[1].click(); await tick();
  await press("F9");
  expect(shortcutOverrides()["go/search"]).toEqual(["mod+k", "f9"]);
  row("go/search").querySelector<HTMLButtonElement>('button[title="Remove this keybinding"]')!.click(); await tick();
  expect(shortcutOverrides()["go/search"]).toBe("f9");
  row("go/search").querySelector<HTMLButtonElement>('button[title="Reset to default"]')!.click(); await tick();
  expect(shortcutOverrides()["go/search"]).toBeUndefined();
  expect(row("go/search").querySelector(".help-keycap-button")?.textContent).toBe("mod+k");
});

it("shows conflicts in secondary keys and retains default aliases when adding", async () => {
  setShortcutOverrides({ "go/search": ["mod+k", "f8"], "ui/toggle-theme": ["t t", "f8"] });
  mount(); await tick();
  expect(row("go/search").querySelector('[role="status"]')?.textContent).toContain(row("ui/toggle-theme").querySelector(".help-shortcut-label")!.firstChild!.textContent);
  row("ui/open-settings").querySelector<HTMLButtonElement>('button[title="Add keybinding"]')!.click(); await tick();
  await press("F9");
  expect(shortcutOverrides()["ui/open-settings"]).toEqual(["t s", "mod+,", "f9"]);
});

it("a default alias the user bound to another command is not shown, conflicted or kept when adding (GH #1044)", async () => {
  setShortcutOverrides({});
  mount(); await tick();
  expect(row("editor/redo").querySelector(".help-shortcut-alias")?.textContent).toContain("+y");
  setShortcutOverrides({ "editor/insert-link": "mod+y" }); await tick();
  expect(row("editor/redo").querySelector(".help-shortcut-alias")).toBeNull();
  expect(row("editor/insert-link").querySelector('[role="status"]')).toBeNull();
  row("editor/redo").querySelector<HTMLButtonElement>('button[title="Add keybinding"]')!.click(); await tick();
  await press("F9");
  expect(shortcutOverrides()["editor/redo"]).toEqual(["mod+shift+z", "f9"]);
});
