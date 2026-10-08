import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { DatePicker } from "./DatePicker";
import { startEditing, endEdit, editingId } from "../editorController";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { readSchedule, resetStore } from "../document";
import { closeDatePicker, datePicker, openDatePicker } from "../ui";
import { dismissTopTransient, clearTransientLayersForTest } from "../transientLayers";

let mounted: (() => void) | undefined;
beforeAll(() => initParser());
afterEach(() => {
  mounted?.(); mounted = undefined;
  closeDatePicker(); endEdit("page-navigation"); resetStore(); clearTransientLayersForTest();
  document.body.innerHTML = ""; vi.restoreAllMocks();
});
const key = (value: string) => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true }));
async function slash(label: "Scheduled" | "Deadline", existing = "") {
  const raw = `TODO Task /${label.toLowerCase()}${existing}`;
  loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null,
    blocks: [{ id: "task", raw, collapsed: false, children: [] }] });
  const caret = raw.indexOf("\n") < 0 ? raw.length : raw.indexOf("\n");
  startEditing("task", caret);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><Block id="task" /><DatePicker /></>, root);
  mounted = dispose;
  const textarea = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
  textarea.focus(); textarea.value = raw; textarea.setSelectionRange(caret, caret);
  textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
  await vi.waitFor(() => expect([...document.body.querySelectorAll(".ac-label")].some(el => el.textContent === label)).toBe(true));
  [...document.body.querySelectorAll<HTMLElement>(".ac-item")].find(el => el.querySelector(".ac-label")?.textContent === label)!
    .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
  return { textarea, dispose };
}
describe("slash calendar keyboard ownership (GH #596)", () => {
  it.each(["Scheduled", "Deadline"] as const)("/%s focuses the picker, arrows cross month/year and Enter applies the date", async (label) => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const { textarea, dispose } = await slash(label, `\n${label.toUpperCase()}: <2026-12-31 Thu 09:00 +1w>`);
    try {
      await vi.waitFor(() => expect(document.activeElement?.closest(".date-picker")).not.toBeNull());
      expect(editingId()).toBe("task");
      for (const arrow of ["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp", "ArrowRight", "ArrowRight"]) {
        key(arrow); await Promise.resolve();
      }
      key("Enter");
      expect(readSchedule("task", label.toLowerCase() as "scheduled" | "deadline")).toMatchObject({ y: 2027, m: 0, d: 2, time: "09:00", repeater: "+1w" });
      expect(datePicker()).toBeNull();
      await vi.waitFor(() => expect(document.activeElement).toBe(textarea));
    } finally { dispose(); }
  });
  it.each(["Scheduled", "Deadline"] as const)("/%s Escape dismisses and restores the editor caret without writing a date", async (label) => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const { textarea, dispose } = await slash(label);
    try {
      await vi.waitFor(() => expect(document.activeElement?.closest(".date-picker")).not.toBeNull());
      const caret = textarea.selectionStart;
      key("Escape");
      await vi.waitFor(() => expect(document.activeElement).toBe(textarea));
      expect(textarea.selectionStart).toBe(caret);
      expect(readSchedule("task", label.toLowerCase() as "scheduled" | "deadline")).toBeNull();
    } finally { dispose(); }
  });
  it("sheet property entry transfers focus and restores its exact opener on cancellation", async () => {
    const trigger = document.createElement("button"); document.body.append(trigger); trigger.focus();
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <DatePicker />, root);
    try {
      openDatePicker("missing", { field: "prop:date", fieldType: "date" }, 10, 10);
      await vi.waitFor(() => expect(document.activeElement?.closest(".date-picker")).not.toBeNull());
      dismissTopTransient("escape");
      await vi.waitFor(() => expect(document.activeElement).toBe(trigger));
    } finally { dispose(); }
  });
});
