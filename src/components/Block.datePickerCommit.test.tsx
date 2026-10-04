import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { DatePicker } from "./DatePicker";
import { startEditing, endEdit } from "../editorController";
import { journalTitle, localCalendarDate, setJournalTitleFormat } from "../journal";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { readSchedule, resetStore, node } from "../document";
import { closeDatePicker, datePicker } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";

let mounted: (() => void) | undefined;
beforeAll(() => initParser());
afterEach(() => {
  mounted?.(); mounted = undefined;
  closeDatePicker(); endEdit("page-navigation"); resetStore(); clearTransientLayersForTest();
  document.body.innerHTML = ""; setJournalTitleFormat(null); vi.restoreAllMocks();
});
const key = (value: string) => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true }));
async function slash(label: "Scheduled" | "Deadline" | "Date picker" | "Tomorrow" | "Yesterday", existing = "", format: "md" | "org" = "md") {
  const raw = `TODO Task /${label.toLowerCase().split(" ")[0]}${existing}`;
  loadSingle({ name: "P", kind: "page", title: "P", format, pre_block: null,
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

const clickDay = (day: number) => document.querySelector<HTMLButtonElement>(`[data-day="${day}"]`)!.click();
const done = () => [...document.querySelectorAll<HTMLButtonElement>(".dp-btn")].find(button => button.textContent === "Done")!.click();
const outside = () => document.querySelector<HTMLElement>(".dp-overlay")!.click();
const addTime = () => {
  document.querySelector<HTMLButtonElement>(".dp-addtime")?.click();
  const input = document.querySelector<HTMLInputElement>(".dp-time-input")!;
  input.value = "10:00"; input.dispatchEvent(new Event("input", { bubbles: true }));
  const repeat = document.querySelector<HTMLSelectElement>(".dp-rep-unit")!;
  repeat.value = "w"; repeat.dispatchEvent(new Event("change", { bubbles: true }));
};
describe("date picker draft commits (GH #30 / #485)", () => {
  it.each(["Scheduled", "Deadline"].flatMap(label => ["done", "outside", "enter"].map(method => [label, method] as const)))("/%s selects a day then %s writes time and repeat", async (label, method) => {
    await slash(label as "Scheduled" | "Deadline");
    clickDay(12);
    expect(datePicker()).not.toBeNull();
    expect(readSchedule("task", label.toLowerCase() as "scheduled" | "deadline")).toBeNull();
    addTime();
    if (method === "done") done(); else if (method === "outside") outside(); else { document.querySelector<HTMLInputElement>(".dp-time-input")!.focus(); key("Enter"); }
    expect(readSchedule("task", label.toLowerCase() as "scheduled" | "deadline")).toMatchObject({ d: 12, time: "10:00", repeater: "+1w" });
    expect(datePicker()).toBeNull();
  });
  it.each(["Scheduled", "Deadline"] as const)("existing %s chip commits edited time and repeat on outside click", async label => {
    loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null,
      blocks: [{ id: "task", raw: `TODO Task\n${label.toUpperCase()}: <2026-10-12 Mon>`, [label.toLowerCase()]: "2026-10-12 Mon", collapsed: false, children: [] }] });
    const root = document.createElement("div"); document.body.append(root);
    mounted = render(() => <><Block id="task" /><DatePicker /></>, root);
    await vi.waitFor(() => expect(root.querySelector(`.date-chip.${label.toLowerCase()}`)).not.toBeNull());
    root.querySelector<HTMLElement>(`.date-chip.${label.toLowerCase()}`)!.click();
    addTime(); outside();
    expect(readSchedule("task", label.toLowerCase() as "scheduled" | "deadline")).toMatchObject({ y: 2026, m: 9, d: 12, time: "10:00", repeater: "+1w" });
  });
  it("Escape cancels the entire changed planning draft", async () => {
    await slash("Scheduled", "\nSCHEDULED: <2026-10-12 Mon 09:00 +1d>");
    clickDay(15); addTime(); key("Escape");
    expect(readSchedule("task", "scheduled")).toMatchObject({ d: 12, time: "09:00", repeater: "+1d" });
  });
  it.each((["md", "org"] as const).flatMap(format => ["done", "outside", "escape"].map(method => [format, method] as const)))("/Date picker in %s inserts a journal link at the slash caret on %s", async (format, method) => {
    setJournalTitleFormat("do MMM yyyy");
    await slash("Date picker", "", format);
    await vi.waitFor(() => expect(datePicker()).not.toBeNull());
    clickDay(12);
    expect(datePicker()).not.toBeNull();
    if (method === "escape") key("Escape"); else if (method === "done") done(); else outside();
    expect(datePicker()).toBeNull();
    if (method === "escape") expect(node("task").raw).toBe("TODO Task ");
    else {
      const picked = localCalendarDate(new Date().getFullYear(), new Date().getMonth(), 12)!;
      expect(node("task").raw.trimEnd()).toBe(`TODO Task [[${journalTitle(picked)}]]`);
    }
    expect(node("task").raw).not.toContain("SCHEDULED");
  });
  it.each(["Tomorrow", "Yesterday"] as const)("/%s inserts a configured journal link", async label => {
    setJournalTitleFormat("do MMM yyyy");
    await slash(label);
    const date = new Date(); date.setDate(date.getDate() + (label === "Tomorrow" ? 1 : -1));
    expect(node("task").raw.trimEnd()).toBe(`TODO Task [[${journalTitle(date)}]]`);
  });
  it("Today selects a draft and Done applies it without losing time", async () => {
    await slash("Scheduled", "\nSCHEDULED: <2026-10-12 Mon>");
    addTime();
    [...document.querySelectorAll<HTMLButtonElement>(".dp-btn")].find(button => button.textContent === "Today")!.click();
    expect(datePicker()).not.toBeNull(); done();
    const today = new Date();
    expect(readSchedule("task", "scheduled")).toMatchObject({ y: today.getFullYear(), m: today.getMonth(), d: today.getDate(), time: "10:00", repeater: "+1w" });
  });
  it("a changed editor refuses a delayed journal insertion", async () => {
    const { textarea } = await slash("Date picker");
    await vi.waitFor(() => expect(datePicker()).not.toBeNull());
    textarea.value = "Newer text";
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
    outside();
    expect(node("task").raw).toBe("Newer text");
  });
});
