import { editorIn } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { Block } from "./Block";

beforeAll(async () => { await initParser(); });
let dispose = () => {};
afterEach(() => { dispose(); resetStore(); vi.unstubAllGlobals(); document.body.innerHTML = ""; });

// master 45d1be1cb (GH #515): typing into an editor whose live mirror sits above
// it (an embed of the same block) grows content above the caret. The keystroke's
// autosize frame absorbs that displacement; a user scroll gesture wins.
describe("live growth above the active editor", () => {
  for (const gesture of [null, "wheel"] as const) {
    it(gesture ? `yields to a ${gesture} gesture` : "keeps the editor still", () => {
      const frames: FrameRequestCallback[] = [];
      vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => { frames.push(cb); return frames.length; });
      vi.stubGlobal("cancelAnimationFrame", () => {});
      loadSingle({ name: "Probe", kind: "page", title: "Probe", pre_block: null, blocks: [
        { id: "a", raw: "above", collapsed: false, children: [] },
        { id: "b", raw: "typed", collapsed: false, children: [] },
      ] });
      startEditing("b", 5);
      const scroller = document.createElement("div");
      scroller.style.overflowY = "auto";
      Object.defineProperty(scroller, "scrollHeight", { value: 2000, configurable: true });
      Object.defineProperty(scroller, "clientHeight", { value: 500, configurable: true });
      document.body.append(scroller);
      dispose = render(() => <For each={pageByName("Probe")?.roots ?? []}>{(id) => <Block id={id} />}</For>, scroller);
      const editor = editorIn(scroller);
      editor.focus();
      scroller.scrollTop = 50;
      let top = 300;
      editor.element.getBoundingClientRect = () => ({ top } as DOMRect);
      editor.replaceText("typed!");
      editor.select(6, 6);
      editor.reportInput({ bubbles: true, inputType: "insertText", data: "!" });
      top = 340; // the mirror above grew by 40px during this keystroke
      if (gesture) scroller.dispatchEvent(new Event(gesture));
      while (frames.length) frames.shift()!(0);
      expect(scroller.scrollTop).toBe(gesture ? 50 : 90);
    });
  }
});
