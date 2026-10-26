import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { DatePicker } from "./DatePicker";
import { backend } from "../backend";
import { blockRegions, initParser } from "../render/parse";
import { node, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { setDoc } from "../document/model";
import { startEditing, endEdit } from "../editorController";
import { editorIn } from "../tests/editorDriver";
import { focusedEditorCommandBridge } from "../editorCommandBridge";
import { closeDatePicker, datePicker } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";
import { setGraphMeta } from "../graphSession";
import type { PageEntry } from "../types";
import { isAnnotationBlock } from "../editor/annotation";

let dispose: (() => void) | undefined;
beforeAll(() => initParser());
beforeEach(() => {
  // These CM-only tests also run with the default textarea switch. jsdom has
  // no Range layout; use the same empty geometry as the opt-in CM census.
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
});
afterEach(() => {
  dispose?.(); dispose = undefined;
  closeDatePicker(); endEdit("page-navigation"); resetStore();
  clearTransientLayersForTest(); setGraphMeta(null); vi.restoreAllMocks(); document.body.innerHTML = "";
});
const page = (raw: string) => ({ name: "Completion", title: "Completion", kind: "page" as const, pre_block: null,
  // Loaded DTO facets are authoritative: supply the properties a real host
  // sends, including generated annotation metadata.
  blocks: [{ id: "completion", raw, collapsed: false, children: [],
    properties: blockRegions(raw, "md").properties.filter(p => p.primary).map((p): [string, string] => [p.key, p.value]) }] });
function mount(raw = "") {
  loadSingle(page(raw)); startEditing("completion", raw.length);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <><Cm6BlockEditor id="completion" surface="main" scope={null} /><DatePicker /></>, root);
  return editorIn(root);
}
const labels = () => [...document.querySelectorAll(".autocomplete .ac-label")].map(row => row.textContent);
async function pick(label: string) {
  await vi.waitFor(() => expect(labels()).toContain(label));
  [...document.querySelectorAll<HTMLElement>(".autocomplete .ac-item")]
    .find(row => row.querySelector(".ac-label")?.textContent === label)!
    .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
}
it("opens slash and paired reference lifecycles through the focused command bridge", async () => {
  const editor = mount("selected");
  editor.select(0, 8);
  expect(focusedEditorCommandBridge()!.dispatch("editor/open-slash-menu")).toBe(true);
  await vi.waitFor(() => expect(labels()[0]).toBe("Page reference"));
  editor.keydown({ key: "Enter" });
  expect(editor.value()).toBe("[[]]");
  expect(editor.selection()).toEqual([2, 2]);
  expect(document.querySelector(".autocomplete")).toBeNull();
  editor.select(0, 4);
  expect(focusedEditorCommandBridge()!.dispatch("editor/insert-block-ref")).toBe(true);
  expect(editor.value()).toBe("(([[]]))");
  expect(editor.selection()).toEqual([6, 6]);
});
it("accepts a fence language on the opener and maps the caret into its body", async () => {
  const editor = mount();
  editor.input("/code");
  await pick("Code block");
  expect(editor.value()).toBe("```\n\n```");
  editor.type("py");
  await pick("Python");
  expect(node("completion").raw).toBe("```python\n\n```");
  expect(editor.value()).toBe("");
  expect(editor.selection()).toEqual([0, 0]);
  expect(document.querySelector(".autocomplete")).toBeNull();
});
it.each(["js", "calc"])("keeps the own-numbered trigger literal inside a %s body", lang => {
  const editor = mount("```" + lang + "\n\n```");
  for (const ch of "1. ") editor.type(ch);
  expect(editor.value()).toBe("1. ");
  expect(node("completion").raw).toBe("```" + lang + "\n1. \n```");
});
it.each(["unmount", "read-only", "same-id replacement"])("retires page completion after %s even with identical text", async change => {
  let finish!: (entries: PageEntry[]) => void;
  vi.spyOn(backend(), "quickSwitch").mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const editor = mount();
  editor.input("[[Target");
  await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
  if (change === "unmount") { dispose?.(); dispose = undefined; }
  else if (change === "read-only") setDoc("pages", p => p.name === "Completion", "readOnly", true);
  else { resetStore(); loadSingle(page("[[Target")); }
  finish([{ name: "Target", kind: "page", date_key: null, path: "pages/Target.md" }]);
  await Promise.resolve(); await Promise.resolve();
  expect(document.querySelector(".autocomplete")).toBeNull();
  expect(node("completion").raw).toBe("[[Target");
});
it("keeps composition keys out of popup acceptance and opens CJK completion after commit", async () => {
  vi.spyOn(backend(), "quickSwitch").mockResolvedValue([{ name: "中文", kind: "page", date_key: null, path: "pages/CJK.md" }]);
  const editor = mount();
  editor.compositionStart(); editor.input("[[中", 3, { isComposing: true });
  editor.keydown({ key: "Enter" }, { isComposing: true });
  expect(document.querySelector(".autocomplete")).toBeNull();
  expect(editor.value()).toBe("[[中");
  expect(node("completion").raw).toBe("");
  editor.compositionEnd("中");
  await Promise.resolve();
  expect(node("completion").raw).toBe("[[中");
  await vi.waitFor(() => expect(labels()).toContain("中文"));
  await pick("中文");
  expect(node("completion").raw).toBe("[[中文]] ");
});
it("does not insert a captured journal date into a same-id replacement graph", async () => {
  const editor = mount();
  editor.input("/date");
  await pick("Date picker");
  const picker = datePicker()!;
  expect(typeof picker.which).toBe("object");
  resetStore(); loadSingle(page("replacement"));
  if (typeof picker.which === "object" && "insertJournal" in picker.which)
    picker.which.insertJournal("Oct 26th, 2026");
  expect(node("completion").raw).toBe("replacement");
});
it("keeps generated annotation metadata out of the completion lifecycle", async () => {
  const templates = vi.spyOn(backend(), "listTemplates");
  const editor = mount("/today\nls-type:: annotation\nhl-page:: 2");
  expect(isAnnotationBlock(node("completion").raw, "md")).toBe(true);
  editor.input("/today\nls-type:: annotation\nhl-page:: 2", 6);
  expect(node("completion").raw).toBe("/today\nls-type:: annotation\nhl-page:: 2");
  expect(isAnnotationBlock(node("completion").raw, "md")).toBe(true);
  await new Promise(resolve => setTimeout(resolve, 120));
  expect(templates).not.toHaveBeenCalled();
  expect(document.querySelector(".autocomplete")).toBeNull();
  expect(node("completion").raw).toBe("/today\nls-type:: annotation\nhl-page:: 2");
});

