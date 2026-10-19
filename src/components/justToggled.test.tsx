// Discussion #714 (scavi): theme keyframe animations on a task checkbox played on
// page load and whenever a virtualized row remounted. The `just-toggled` class
// exists only on the task chips the user pressed, for a short window, so a theme
// can animate a toggle and nothing else.
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore, setRaw, node as docNode } from "../document";
import { loadSingle } from "../document/workingSet";
import { Block } from "./Block";
import { endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";
import { backend } from "../backend";
import { AstBody } from "../render/body";
import type { BlockDto } from "../types";
import { JUST_TOGGLED_CLASS, JUST_TOGGLED_MS } from "../justToggled";

let disposeKeys: () => void;
beforeAll(async () => { await initParser(); disposeKeys = installKeybindings(); });
afterAll(() => disposeKeys());
afterEach(() => {
  vi.useRealTimers();
  endEdit("page-navigation");
  resetStore();
  document.body.innerHTML = "";
});
const tick = async () => { for (let i = 0; i < 3; i++) await new Promise(r => setTimeout(r, 20)); };
const marked = (root: Element) => root.querySelectorAll(`.${JUST_TOGGLED_CLASS}`);

function mount(raw = "TODO write it") {
  loadSingle({ name: "P", title: "P", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "b", raw, collapsed: false, children: [], marker: raw.split(" ", 1)[0] } as never] });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Block id="b" />, root);
  return { root, dispose };
}
const press = (el: Element) => el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));

describe("just-toggled theme hook (discussion #714)", () => {
  it("is absent on load, appears on the pressed checkbox and its marker, and is gone after the window", async () => {
    const { root, dispose } = mount();
    try {
      await tick();
      expect(root.querySelector(".block-task-checkbox")).not.toBeNull();
      expect(marked(root)).toHaveLength(0);
      vi.useFakeTimers();
      press(root.querySelector(".block-task-checkbox")!);
      expect(docNode("b").raw).toBe("DONE write it");
      const box = root.querySelector(".block-task-checkbox")!;
      expect(box.classList.contains(JUST_TOGGLED_CLASS)).toBe(true);
      expect(box.classList.contains("checked")).toBe(true);
      expect(root.querySelector(".block-marker")!.classList.contains(JUST_TOGGLED_CLASS)).toBe(true);
      vi.advanceTimersByTime(JUST_TOGGLED_MS + 5);
      expect(marked(root)).toHaveLength(0);
    } finally { dispose(); }
  });

  it("is also set by pressing the marker word", async () => {
    const { root, dispose } = mount();
    try {
      await tick();
      vi.useFakeTimers();
      root.querySelector<HTMLElement>(".block-marker")!.click();
      expect(docNode("b").raw).not.toBe("TODO write it");
      expect(root.querySelector(".block-task-checkbox")!.classList.contains(JUST_TOGGLED_CLASS)).toBe(true);
      expect(root.querySelector(".block-marker")!.classList.contains(JUST_TOGGLED_CLASS)).toBe(true);
    } finally { dispose(); }
  });

  it("is not set when the task changes without the user pressing it (sync, external editor), nor on a remount inside the window", async () => {
    const { root, dispose } = mount();
    try {
      await tick();
      setRaw("b", "DONE write it");
      await tick();
      expect(root.querySelector(".block-task-checkbox")!.classList.contains("checked")).toBe(true);
      expect(marked(root)).toHaveLength(0);
      // Press, then remount the row (a virtualized row scrolling back in) inside the window.
      press(root.querySelector(".block-task-checkbox")!);
      expect(marked(root).length).toBeGreaterThan(0);
    } finally { dispose(); }
    document.body.innerHTML = "";
    const again = document.createElement("div"); document.body.append(again);
    const dispose2 = render(() => <Block id="b" />, again);
    try {
      await tick();
      expect(again.querySelector(".block-task-checkbox")).not.toBeNull();
      expect(marked(again)).toHaveLength(0);
    } finally { dispose2(); }
  });

  it("a second toggle inside the window restarts it", async () => {
    const { root, dispose } = mount();
    try {
      await tick();
      vi.useFakeTimers();
      press(root.querySelector(".block-task-checkbox")!);
      vi.advanceTimersByTime(JUST_TOGGLED_MS - 100);
      press(root.querySelector(".block-task-checkbox")!);
      vi.advanceTimersByTime(200);
      expect(root.querySelector(".block-task-checkbox")!.classList.contains(JUST_TOGGLED_CLASS)).toBe(true);
      vi.advanceTimersByTime(JUST_TOGGLED_MS);
      expect(marked(root)).toHaveLength(0);
    } finally { dispose(); }
  });

  it("a task ticked inside a block reference gets it on the reference's checkbox, once the source was toggled", async () => {
    const ID = "71400000-0000-4000-8000-0000000000b1";
    const block: BlockDto = { id: ID, raw: `TODO buy milk\nid:: ${ID}`, collapsed: false, children: [], properties: [["id", ID]], marker: "TODO" };
    loadSingle({ name: "Task source", title: "Task source", kind: "page", format: "md", pre_block: null, blocks: [block] } as never);
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{ page: "Task source", kind: "page", blocks: [block] }]);
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <AstBody raw={`Get ((${ID}))`} />, root);
    try {
      const box = await vi.waitFor(() => {
        const el = root.querySelector(".block-ref .block-task-checkbox");
        expect(el).not.toBeNull();
        return el!;
      });
      expect(marked(root)).toHaveLength(0);
      press(box);
      await vi.waitFor(() => expect(docNode(ID).raw.startsWith("DONE")).toBe(true));
      await vi.waitFor(() => expect(root.querySelector(".block-ref .block-task-checkbox")!.classList.contains(JUST_TOGGLED_CLASS)).toBe(true));
    } finally { dispose(); }
  });
});
