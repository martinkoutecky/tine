import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { backend } from "../backend";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { toasts, setToasts } from "../toasts";
import type { RefGroup } from "../types";

beforeAll(() => initParser());
afterEach(() => { resetStore(); vi.restoreAllMocks(); setToasts([]); document.body.innerHTML = ""; });
function mount() {
  loadSingle({ name: "Picker", title: "Picker", kind: "page", pre_block: null,
    blocks: [{ id: "picker", raw: "", children: [], collapsed: false }] });
  startEditing("picker", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Block id="picker" />, root);
  const textarea = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
  const input = (value: string) => {
    textarea.focus(); textarea.value = value; textarea.setSelectionRange(value.length, value.length);
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value.at(-1) }));
  };
  return { dispose, textarea, input };
}
function deferred() {
  let resolve!: (groups: RefGroup[]) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<RefGroup[]>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
it("keeps pending, failure, retry and empty block-picker answers visibly distinct without toasts", async () => {
  setToasts([]);
  const work = deferred();
  const search = vi.spyOn(backend(), "search").mockReturnValueOnce(work.promise).mockResolvedValue([]);
  const m = mount();
  try {
    m.input("((target");
    await vi.waitFor(() => expect(search).toHaveBeenCalled());
    expect(document.querySelector(".autocomplete")?.textContent).toContain("Searching…");
    expect(document.querySelector(".autocomplete")?.textContent).not.toContain("No matched blocks");
    work.reject(new Error("private failure text"));
    await vi.waitFor(() => expect(document.querySelector(".autocomplete")?.textContent).toContain("Couldn’t load blocks"));
    expect(document.body.textContent).not.toContain("private failure text");
    document.querySelector<HTMLButtonElement>(".autocomplete button")!.click();
    await vi.waitFor(() => expect(search).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(document.querySelector(".autocomplete")?.textContent).toContain("No matched blocks"));
    expect(m.textarea.value).toBe("((target");
    expect(toasts()).toEqual([]);
    m.textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(document.querySelector(".autocomplete")).toBeNull());
  } finally { work.resolve([]); m.dispose(); }
});
it("never lands an old picker failure over a newer answer or a closed editor", async () => {
  const work = deferred();
  const search = vi.spyOn(backend(), "search").mockReturnValueOnce(work.promise).mockResolvedValue([]);
  const m = mount();
  try {
    m.input("((old");
    await vi.waitFor(() => expect(search).toHaveBeenCalledOnce());
    m.input("((new");
    await vi.waitFor(() => expect(document.querySelector(".autocomplete")?.textContent).toContain("No matched blocks"));
    work.reject(new Error("old search failed"));
    await Promise.resolve(); await Promise.resolve();
    expect(document.querySelector(".autocomplete")?.textContent).not.toContain("Couldn’t load");
    m.input("ordinary text");
    await vi.waitFor(() => expect(document.querySelector(".autocomplete")).toBeNull());
  } finally { work.resolve([]); m.dispose(); }
});
