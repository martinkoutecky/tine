import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Show } from "solid-js";
import { render } from "solid-js/web";
import { EditorView } from "@codemirror/view";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { QuickSwitcher } from "./QuickSwitcher";
import { editingId, endEdit, startEditing } from "../editorController";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { closeSwitcher, openCommandPalette, openSwitcher } from "../ui";
import { installKeybindings } from "../keybindings";

let dispose: (() => void) | undefined;
beforeEach(() => {
  vi.spyOn(document, "hasFocus").mockReturnValue(true);
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
});
afterEach(() => {
  dispose?.(); dispose = undefined;
  closeSwitcher(); endEdit("page-navigation"); resetStore();
  vi.restoreAllMocks(); document.body.replaceChildren();
});

it.each(["shortcut", "search", "commands"])("hands native focus to %s before retiring the CM6 editor", async entry => {
  loadSingle({ name: "Focus", title: "Focus", kind: "page", pre_block: null,
    blocks: [{ id: "focus", raw: "Draft", collapsed: false, children: [] }] });
  startEditing("focus", 0);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <>
    <Show when={editingId() === "focus"}><Cm6BlockEditor id="focus" surface="main" scope={null} /></Show>
    <QuickSwitcher />
  </>, root);
  const view = EditorView.findFromDOM(root.querySelector(".cm-content")!)!;
  // jsdom has no EditContext; represent the native input context whose
  // release must wait for focus handoff. Chromium exercises the real API.
  Object.defineProperty(view.contentDOM, "editContext", { value: {}, configurable: true });
  view.focus();
  const focusedAtRetirement: (Element | null)[] = [];
  const destroy = view.destroy.bind(view);
  vi.spyOn(view, "destroy").mockImplementation(() => {
    // Chromium's Android EditContext must retire after the destination owns
    // native focus; the real browser journey also verifies text insertion.
    focusedAtRetirement.push(document.activeElement);
    destroy();
  });
  const uninstall = installKeybindings();
  try {
    if (entry === "shortcut") view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", { key: "k", ctrlKey: true, bubbles: true, cancelable: true }));
    else if (entry === "search") openSwitcher();
    else openCommandPalette();
    await Promise.resolve(); await Promise.resolve();
    const input = root.querySelector<HTMLInputElement>(".switcher-input")!;
    expect(document.activeElement).toBe(input);
    expect(focusedAtRetirement).toEqual([input]);
    input.value = "Toggle code block wrap";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => expect(root.querySelector(".switcher-row")?.textContent).toContain("Toggle code block wrap"));
    expect(input.value).toBe("Toggle code block wrap");
    expect(editingId()).toBeNull();
  } finally { uninstall(); }
});
