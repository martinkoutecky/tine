import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { For } from "solid-js";
import { Block } from "./Block";
import { MobileKeyboardToolbar, NATIVE_IME_EVENT, revealFocusedEditorAboveToolbar } from "./MobileKeyboardToolbar";
import { DatePicker } from "./DatePicker";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { node, pageByName, readSchedule, resetStore } from "../document";
import { setDoc } from "../document/model";
import { editingId, endEdit, startEditing } from "../editorController";
import { dispatchFocusedEditorCommand } from "../editorCommandBridge";
import { installKeybindings } from "../keybindings";
import { editorIn } from "../tests/editorDriver";
import { closeDatePicker, datePicker, setWorkflow } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";

vi.mock("../inputCapability", async original => ({
  ...await original<typeof import("../inputCapability")>(), touchPrimary: () => true,
}));

let dispose: (() => void) | undefined;
let uninstall: (() => void) | undefined;
beforeAll(() => initParser());
beforeEach(() => {
  uninstall = installKeybindings();
  setWorkflow("todo");
  Object.defineProperty(window, "visualViewport", { configurable: true, value: undefined });
});
afterEach(() => {
  dispose?.(); uninstall?.();
  closeDatePicker(); clearTransientLayersForTest(); endEdit("page-navigation"); resetStore();
  vi.restoreAllMocks(); document.body.innerHTML = "";
});

function mount(raw = "selected text") {
  loadSingle({ name: "Dock", title: "Dock", kind: "page", pre_block: null, blocks: [
    { id: "before", raw: "previous", collapsed: false, children: [] },
    { id: "task", raw, collapsed: false, children: [] },
  ] });
  startEditing("task", 0);
  const root = document.createElement("main"); root.className = "main-content"; document.body.append(root);
  dispose = render(() => <>
    <For each={pageByName("Dock")?.roots ?? []}>{id => <Block id={id} />}</For>
    <MobileKeyboardToolbar /><DatePicker />
  </>, root);
  return root;
}
const tap = (label: string) => {
  const button = document.querySelector<HTMLButtonElement>(`[data-mobile-keyboard-toolbar] button[aria-label="${label}"]`)!;
  expect(button).not.toBeNull(); button.click();
};

it("keeps the hardware-keyboard dock for either editor and retires it after native IME hide", async () => {
  mount();
  await vi.waitFor(() => expect(document.documentElement.hasAttribute("data-mobile-editing-dock")).toBe(true));
  window.dispatchEvent(new CustomEvent(NATIVE_IME_EVENT, { detail: true }));
  expect(document.documentElement.hasAttribute("data-mobile-editing-dock")).toBe(true);
  window.dispatchEvent(new CustomEvent(NATIVE_IME_EVENT, { detail: false }));
  expect(document.documentElement.hasAttribute("data-mobile-editing-dock")).toBe(false);
  expect(document.activeElement).toBe(editorIn().element);
});

it("cycles TODO from the mounted dock and keyboard with the same caret delta", async () => {
  mount();
  const editor = editorIn(); editor.select(3);
  await Promise.resolve(); tap("TODO");
  await vi.waitFor(() => expect(editor.value()).toBe("TODO selected text"));
  expect(editor.selection()).toEqual([8, 8]);
  editor.keydown({ key: "Enter", ctrlKey: true });
  await vi.waitFor(() => expect(editor.value()).toBe("DOING selected text"));
  expect(editor.selection()).toEqual([9, 9]);
  expect(node("task").raw.split("\n")[0]).toBe("DOING selected text");
  expect(node("task").raw).toContain(":LOGBOOK:");
});

it("preserves the selected range through dock indent and outdent", async () => {
  mount();
  editorIn().select(2, 8, "backward"); await Promise.resolve();
  tap("Indent");
  await vi.waitFor(() => expect(node("task").parent).toBe("before"));
  expect(editorIn().selection()).toEqual([2, 8]); expect(editorIn().direction()).toBe("backward");
  tap("Outdent");
  await vi.waitFor(() => expect(node("task").parent).toBeNull());
  expect(editorIn().selection()).toEqual([2, 8]); expect(editorIn().direction()).toBe("backward");
});

it("uses one document timeline for keyboard and dock undo/redo across a structural edit", async () => {
  mount();
  editorIn().select(13); editorIn().type("!"); await Promise.resolve();
  tap("Indent");
  await vi.waitFor(() => expect(node("task").parent).toBe("before"));
  editorIn().keydown({ key: "z", ctrlKey: true });
  await vi.waitFor(() => expect(node("task").parent).toBeNull());
  expect(node("task").raw).toBe("selected text!");
  tap("Undo");
  await vi.waitFor(() => expect(node("task").raw).toBe("selected text"));
  expect(editingId()).toBe("task");
  tap("Redo");
  expect(node("task").raw).toBe("selected text!");
  editorIn().keydown({ key: "z", ctrlKey: true, shiftKey: true });
  await vi.waitFor(() => expect(node("task").parent).toBe("before"));
});

it("honors remapped history without retaining a private Mod+Z undo stack", async () => {
  uninstall?.(); uninstall = installKeybindings({ "editor/undo": "alt+shift+u", "editor/redo": "alt+shift+r" });
  mount(); editorIn().select(13); editorIn().type("!");
  editorIn().keydown({ key: "z", ctrlKey: true });
  expect(node("task").raw).toBe("selected text!");
  editorIn().keydown({ key: "U", altKey: true, shiftKey: true });
  await vi.waitFor(() => expect(node("task").raw).toBe("selected text"));
  editorIn().keydown({ key: "R", altKey: true, shiftKey: true });
  expect(node("task").raw).toBe("selected text!");
});

it("applies the dock planning picker to its mounted editor and restores focus", async () => {
  mount("TODO task"); await Promise.resolve(); tap("Date picker");
  expect(datePicker()?.blockId).toBe("task");
  document.querySelector<HTMLButtonElement>('[data-day="12"]')!.click();
  [...document.querySelectorAll<HTMLButtonElement>(".dp-btn")].find(button => button.textContent === "Done")!.click();
  await vi.waitFor(() => expect(readSchedule("task", "scheduled")?.d).toBe(12));
  expect(datePicker()).toBeNull();
  expect(document.activeElement).toBe(editorIn().element);
});

it("reveals either focused editor above the docked selection row", async () => {
  const root = mount(); await Promise.resolve();
  const editor = editorIn(); editor.selectByUser(0, 8);
  vi.spyOn(editor.element, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 470, 300, 30));
  vi.spyOn(root.querySelector<HTMLElement>("[data-mobile-selection-toolbar]")!, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 480, 300, 32));
  expect(revealFocusedEditorAboveToolbar(520)).toBe(true);
  expect(root.scrollTop).toBe(28);
});

it("refuses dock text commands after read-only replacement", async () => {
  mount(); await Promise.resolve();
  setDoc("pages", page => page.name === "Dock", "readOnly", true);
  dispatchFocusedEditorCommand("editor/cycle-todo");
  expect(node("task").raw).toBe("selected text");
});

it("keeps CM's native IME preedit out of dock text commands", async () => {
  mount(); const editor = editorIn(); await Promise.resolve();
  // CM owns a native contenteditable composition; the textarea's mobile
  // command adapter does not expose that guard.
  if (editor.kind !== "cm6") return;
  editor.compositionStart(); dispatchFocusedEditorCommand("editor/cycle-todo");
  expect(node("task").raw).toBe("selected text");
  editor.compositionEnd();
});
