import { editorIn, type EditorDriver } from "../tests/editorDriver";
// GH #53 follow-up: typing through full-width closing brackets. `【【` becomes
// `[[]]` with the caret inside; a Chinese IME's `】` typed before the ASCII `]`
// steps over it like `]` does, so `【【name】】` ends as `[[name]]` with the caret
// after it and no `】` left in the text. Both delivery paths of an IME character
// (plain input, and composition end) and auto-pairing on and off.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { PageDto } from "../types";
import { Block } from "./Block";
import { autoPairing, setAutoPairing } from "../ui";
import { fullWidthCloserEdit } from "../editor/autocomplete";

beforeAll(() => initParser());
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(node, root) };
}
const page = (raw: string): PageDto => ({ name: "FW", kind: "page", title: "FW", pre_block: null,
  blocks: [{ id: "fw", raw, collapsed: false, children: [] }] });

function composing(textarea: EditorDriver, value: string, caret: number) {
  textarea.input(value, caret, { inputType: "insertCompositionText", isComposing: true });
}
function input(textarea: EditorDriver, value: string, caret: number) {
  textarea.input(value, caret, { inputType: "insertText", data: value[caret - 1] ?? null });
}
const compositionStart = (t: EditorDriver) => t.compositionStart();
const compositionEnd = (t: EditorDriver) => t.compositionEnd();

describe("typing through full-width closers (GH #53)", () => {
  for (const pairing of [false, true]) {
    for (const composition of [false, true]) {
      for (const [open, close, ascii] of [["【【", "】", "[["], ["（（", "）", "(("]] as const) {
        if (open === "（（" && !pairing) continue; // an ASCII `)` does not type through without auto-pairing either
        const closer = ascii === "[[" ? "]" : ")";
        it(`${open}name${close}${close} ends as ${ascii}name${closer}${closer}, composition=${composition}, autoPairing=${pairing}`, async () => {
          vi.useFakeTimers();
          vi.spyOn(backend(), "search").mockResolvedValue([]);
          vi.spyOn(backend(), "quickSwitch").mockResolvedValue([]);
          const previous = autoPairing();
          setAutoPairing(pairing);
          loadSingle(page(""));
          startEditing("fw", 0);
          const { root, dispose } = mount(() => <Block id="fw" />);
          try {
            const t = editorIn(root);
            const type = async (ch: string) => {
              const at = t.selection()[0];
              const value = t.value().slice(0, at) + ch + t.value().slice(at);
              if (composition) { compositionStart(t); composing(t, value, at + ch.length); compositionEnd(t); await Promise.resolve(); }
              else input(t, value, at + ch.length);
            };
            if (composition) { compositionStart(t); composing(t, open, 2); compositionEnd(t); await Promise.resolve(); }
            else { input(t, open[0], 1); input(t, open, 2); }
            expect(t.value()).toBe(`${ascii}${closer}${closer}`);
            for (const ch of "name") await type(ch);
            expect(t.value()).toBe(`${ascii}name${closer}${closer}`);
            await type(close);
            expect(t.value()).toBe(`${ascii}name${closer}${closer}`);
            expect(t.selection()[0]).toBe(ascii.length + 4 + 1);
            await type(close);
            expect(t.value()).toBe(`${ascii}name${closer}${closer}`);
            expect(t.selection()[0]).toBe(ascii.length + 4 + 2);
            expect(doc.byId.fw.raw).toBe(`${ascii}name${closer}${closer}`);
            // The text after the finished reference is ordinary again.
            await type(close);
            expect(t.value()).toBe(`${ascii}name${closer}${closer}${close}`);
          } finally {
            dispose();
            setAutoPairing(previous);
          }
        });
      }
    }
  }

  it("a full-width closer with no matching ASCII closer after the caret stays text", () => {
    expect(fullWidthCloserEdit("a】", 2, false)).toBeNull();
    expect(fullWidthCloserEdit("[[x】", 5, false)).toBeNull();
    expect(fullWidthCloserEdit("a】b]", 2, false)).toBeNull();
    expect(fullWidthCloserEdit("", 0, false)).toBeNull();
  });
  it("steps over exactly one ASCII closer", () => {
    expect(fullWidthCloserEdit("[[x】]]", 4, false)).toEqual({ value: "[[x]]", caret: 4 });
    expect(fullWidthCloserEdit("(a）)", 3, true)).toEqual({ value: "(a)", caret: 3 });
    expect(fullWidthCloserEdit("(a）)", 3, false)).toBeNull();
  });
});
