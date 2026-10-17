import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { editorCommandFor, eventToBindingString, installKeybindings, matchesCommand } from "../keybindings";
import { closeSettings, closeSwitcher, openSettings, setShortcutOverrides, shortcutOverrides, switcherOpen } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";
import { Settings } from "./Settings";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
let dispose = () => {};
let keys = () => {};
afterEach(() => {
  dispose(); keys(); clearTransientLayersForTest(); closeSettings(); closeSwitcher();
  setShortcutOverrides({}); localStorage.removeItem("logseq-claude.shortcuts");
  document.body.innerHTML = "";
});
const key = (init: KeyboardEventInit) => new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });

it("opens search from a Cyrillic-layout editor and matches remapped editor commands (GH #709)", () => {
  keys = installKeybindings({ "editor/insert-link": "mod+shift+k" });
  const editor = document.createElement("textarea");
  document.body.append(editor); editor.focus();
  const event = key({ key: "л", code: "KeyK", ctrlKey: true });
  editor.dispatchEvent(event);
  expect(switcherOpen()).toBe(true);
  expect(event.defaultPrevented).toBe(true);
  expect(matchesCommand(event, "go/search")).toBe(true);
  expect(editorCommandFor(key({ key: "Л", code: "KeyK", ctrlKey: true, shiftKey: true }))).toBe("editor/insert-link");
});

it("records a Latin binding in Settings while a Cyrillic layout is selected (GH #709)", async () => {
  keys = installKeybindings();
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <Settings />, root);
  openSettings("shortcuts"); await tick();
  const row = [...root.querySelectorAll<HTMLElement>(".help-shortcut-row")]
    .find((element) => element.querySelector(".help-shortcut-id")?.textContent === "go/find-in-page")!;
  (row.querySelector(".help-keycap-button") as HTMLButtonElement).click(); await tick();
  window.dispatchEvent(key({ key: "л", code: "KeyK", ctrlKey: true })); await tick();
  expect(shortcutOverrides()["go/find-in-page"]).toBe("mod+k");
  const updatedRow = [...root.querySelectorAll<HTMLElement>(".help-shortcut-row")]
    .find((element) => element.querySelector(".help-shortcut-id")?.textContent === "go/find-in-page")!;
  expect(updatedRow.textContent?.toLowerCase()).toContain("k");
  expect(updatedRow.querySelector(".recording")).toBeNull();
  expect(switcherOpen()).toBe(false);
});

it("matches multi-chord remaps on a non-Latin layout and declines composition", () => {
  keys = installKeybindings({ "go/search": "mod+g mod+k" });
  window.dispatchEvent(key({ key: "п", code: "KeyG", ctrlKey: true, isComposing: true }));
  window.dispatchEvent(key({ key: "л", code: "KeyK", ctrlKey: true }));
  expect(switcherOpen()).toBe(false);
  window.dispatchEvent(key({ key: "п", code: "KeyG", ctrlKey: true }));
  window.dispatchEvent(key({ key: "л", code: "KeyK", ctrlKey: true }));
  expect(switcherOpen()).toBe(true);
});

it("keeps Latin layouts and punctuation, falls back for non-Latin digits, and leaves typing alone", () => {
  expect(eventToBindingString(key({ key: "л", code: "KeyK", ctrlKey: true }))).toBe("mod+k");
  expect(eventToBindingString(key({ key: "ש", code: "KeyA", ctrlKey: true }))).toBe("mod+a");
  expect(eventToBindingString(key({ key: "क्ष", code: "KeyK", ctrlKey: true }))).toBe("mod+k");
  expect(eventToBindingString(key({ key: "๑", code: "Digit1", ctrlKey: true }))).toBe("mod+1");
  for (const [character, code] of [["a", "KeyQ"], ["e", "KeyD"], ["é", "Digit2"], ["e\u0301", "KeyE"], ["&", "Digit1"], ["?", "Slash"]]) {
    expect(eventToBindingString(key({ key: character, code, ctrlKey: true }))).toBe(`mod+${character}`);
  }
  expect(eventToBindingString(key({ key: "Dead", code: "KeyK" }))).toBe("dead");
  expect(eventToBindingString(key({ key: "л" }))).toBe("л");
  keys = installKeybindings();
  const editor = document.createElement("textarea"); document.body.append(editor); editor.focus();
  const event = key({ key: "л", code: "KeyK" }); editor.dispatchEvent(event);
  expect(event.key).toBe("л"); expect(event.defaultPrevented).toBe(false);
  editor.value = "л";
  editor.dispatchEvent(new InputEvent("input", { data: "л", inputType: "insertText", bubbles: true }));
  expect(editor.value).toBe("л");
});
