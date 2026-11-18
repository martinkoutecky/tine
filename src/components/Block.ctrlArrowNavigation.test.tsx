import { editorIn } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { startEditing, endEdit, editingId } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, readSchedule, resetStore, selectBlock, selectedIds } from "../document";
import { loadSingle } from "../document/workingSet";
import { closeDatePicker, datePicker } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";
import { installKeybindings } from "../keybindings";
import { Block } from "./Block";
import { DatePicker } from "./DatePicker";

// GH #986: Logseq binds Ctrl+P / Ctrl+N beside Up / Down (editor, autocomplete, date picker).
beforeAll(() => initParser());
let dispose = () => {};
let keys = () => {};
afterEach(() => {
  dispose(); keys(); closeDatePicker(); endEdit("page-navigation"); resetStore(); clearTransientLayersForTest();
  document.body.innerHTML = ""; vi.restoreAllMocks();
});

const UP = { key: "p", code: "KeyP", ctrlKey: true } as const;
const DOWN = { key: "n", code: "KeyN", ctrlKey: true } as const;
const press = (init: KeyboardEventInit) =>
  document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));

function mountPage(raws: string[], editing: number, caret: number) {
  loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null,
    blocks: raws.map((raw, i) => ({ id: `b${i}`, raw, collapsed: false, children: [] })) });
  startEditing(`b${editing}`, caret);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <><For each={pageByName("P")?.roots ?? []}>{(id) => <Block id={id} />}</For><DatePicker /></>, root);
  return root;
}

describe("Ctrl+P / Ctrl+N move between blocks like Up / Down (GH #986)", () => {
  it("leaves a single-line block for the previous and the next one", () => {
    const root = mountPage(["foo", "bar"], 1, 0);
    let editor = editorIn(root);
    editor.select(0, 0);
    expect(editor.keydown(UP).defaultPrevented).toBe(true);
    editor = editorIn(root);
    expect(editor.value()).toBe("foo");
    expect(editor.keydown(DOWN).defaultPrevented).toBe(true);
    expect(editorIn(root).value()).toBe("bar");
  });

  it("stays inside a multi-line block until its first or last line, as Up / Down do", () => {
    const root = mountPage(["above", "one\ntwo", "below"], 1, 5);
    let editor = editorIn(root);
    editor.select(5, 5); // on the last line: Ctrl+P must not leave (the caret moves up a line instead)
    editor.keydown(UP);
    expect(editorIn(root).value()).toBe("one\ntwo");
    editor.select(1, 1); // first line: Ctrl+P leaves upward
    editor.keydown(UP);
    expect(editorIn(root).value()).toBe("above");
    editor = editorIn(root);
    editor.select(5, 5);
    editor.keydown(DOWN);
    expect(editorIn(root).value()).toBe("one\ntwo");
    editor = editorIn(root);
    editor.select(5, 5); // last line: Ctrl+N leaves downward
    editor.keydown(DOWN);
    expect(editorIn(root).value()).toBe("below");
  });

  it("does not take Ctrl+Shift+P, Cmd+P or Alt chords", () => {
    const root = mountPage(["foo", "bar"], 1, 0);
    const editor = editorIn(root);
    editor.select(0, 0);
    for (const extra of [{ shiftKey: true }, { metaKey: true }, { altKey: true }]) {
      editor.keydown({ ...UP, ...extra });
      expect(editorIn(root).value()).toBe("bar");
    }
  });

  it("moves the autocomplete highlight", async () => {
    const root = mountPage(["x"], 0, 0);
    const editor = editorIn(root);
    editor.focus(); editor.input("/", 1, { inputType: "insertText" });
    await vi.waitFor(() => expect(document.querySelectorAll(".ac-item").length).toBeGreaterThan(2));
    const active = () => [...document.querySelectorAll(".ac-item")].findIndex(el => el.classList.contains("active"));
    expect(active()).toBe(0);
    editor.keydown(DOWN);
    expect(active()).toBe(1);
    editor.keydown(DOWN);
    expect(active()).toBe(2);
    editor.keydown(UP);
    expect(active()).toBe(1);
    expect(editorIn(root).value()).toBe("/"); // the popup took the keys; no block navigation
  });

  it("moves the date picker by a week", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const root = mountPage(["TODO Task /scheduled\nSCHEDULED: <2026-12-10 Thu>"], 0, 19);
    const editor = editorIn(root);
    editor.focus(); editor.input("TODO Task /scheduled\nSCHEDULED: <2026-12-10 Thu>", 19, { inputType: "insertText" });
    await vi.waitFor(() => expect([...document.querySelectorAll(".ac-label")].some(el => el.textContent === "Scheduled")).toBe(true));
    [...document.querySelectorAll<HTMLElement>(".ac-item")].find(el => el.querySelector(".ac-label")?.textContent === "Scheduled")!
      .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(document.activeElement?.closest(".date-picker")).not.toBeNull());
    press(DOWN);
    await Promise.resolve();
    press(DOWN);
    await Promise.resolve();
    press(UP);
    await Promise.resolve();
    press({ key: "Enter" });
    expect(readSchedule("b0", "scheduled")).toMatchObject({ y: 2026, m: 11, d: 17 });
    expect(datePicker()).toBeNull();
    expect(editingId()).toBe("b0");
  });

  it("moves the block selection", () => {
    loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null,
      blocks: ["a", "b", "c"].map(id => ({ id, raw: id, collapsed: false, children: [] })) });
    const root = document.createElement("div"); document.body.append(root);
    dispose = render(() => <For each={pageByName("P")?.roots ?? []}>{(id) => <Block id={id} />}</For>, root);
    keys = installKeybindings();
    selectBlock("b");
    document.body.dispatchEvent(new KeyboardEvent("keydown", { ...DOWN, bubbles: true, cancelable: true }));
    expect(selectedIds()).toEqual(["c"]);
    document.body.dispatchEvent(new KeyboardEvent("keydown", { ...UP, bubbles: true, cancelable: true }));
    document.body.dispatchEvent(new KeyboardEvent("keydown", { ...UP, bubbles: true, cancelable: true }));
    expect(selectedIds()).toEqual(["a"]);
  });
});
