import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";

const { setRawSpy } = vi.hoisted(() => ({ setRawSpy: vi.fn() }));

vi.mock("../store", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../store")>();
  return {
    ...actual,
    setRaw: (...args: Parameters<typeof actual.setRaw>) => {
      setRawSpy(...args);
      return actual.setRaw(...args);
    },
  };
});

import { backend } from "../backend";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { doc, isDirty, loadSingle, pageByName, resetStore } from "../store";
import type { BlockDto, PageDto, PageEntry } from "../types";
import { Block } from "./Block";

beforeAll(() => initParser());

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  setRawSpy.mockReset();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(node, root) };
}

function page(raw: string): PageDto {
  const block: BlockDto = { id: "ime-composition", raw, collapsed: false, children: [] };
  return { name: "IME composition", kind: "page", title: "IME composition", pre_block: null, blocks: [block] };
}

function composingInput(textarea: HTMLTextAreaElement, value: string, caret = value.length) {
  textarea.value = value;
  textarea.setSelectionRange(caret, caret);
  const event = new InputEvent("input", {
    bubbles: true,
    inputType: "insertCompositionText",
    data: value[caret - 1] ?? null,
  });
  Object.defineProperty(event, "isComposing", { value: true });
  textarea.dispatchEvent(event);
}

function input(textarea: HTMLTextAreaElement, value: string, caret = value.length) {
  textarea.value = value;
  textarea.setSelectionRange(caret, caret);
  textarea.dispatchEvent(new InputEvent("input", {
    bubbles: true,
    inputType: "insertText",
    data: value[caret - 1] ?? null,
  }));
}

function compositionStart(textarea: HTMLTextAreaElement) {
  textarea.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
}

function compositionEnd(textarea: HTMLTextAreaElement) {
  textarea.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
}

function pageEntry(name: string): PageEntry {
  return { name, kind: "page", date_key: null, path: `pages/${name}.md` };
}

function composingEnter(textarea: HTMLTextAreaElement) {
  const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
  Object.defineProperty(event, "isComposing", { value: true });
  textarea.dispatchEvent(event);
  return event;
}

function legacyImeEnter(textarea: HTMLTextAreaElement) {
  const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
  Object.defineProperty(event, "keyCode", { value: 229 });
  textarea.dispatchEvent(event);
  return event;
}

describe("IME composition", () => {
  it("keeps intermediate page-reference input transaction-local, commits once on end, and ignores the trailing duplicate input", async () => {
    vi.useFakeTimers();
    const quickSwitch = vi.spyOn(backend(), "quickSwitch").mockResolvedValue([]);
    loadSingle(page(""));
    startEditing("ime-composition", 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("IME composition")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      setRawSpy.mockClear();
      const autosize = vi.spyOn(window, "requestAnimationFrame");

      compositionStart(textarea);
      composingInput(textarea, "[[P");
      composingInput(textarea, "[[Pa");

      expect(textarea.value).toBe("[[Pa");
      expect(doc.byId["ime-composition"].raw).toBe("");
      expect(setRawSpy).not.toHaveBeenCalled();
      expect(autosize).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(100);
      expect(quickSwitch).not.toHaveBeenCalled();

      compositionEnd(textarea);
      expect(doc.byId["ime-composition"].raw).toBe("[[Pa");
      expect(setRawSpy).toHaveBeenCalledTimes(1);
      expect(autosize).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(100);
      expect(quickSwitch).toHaveBeenLastCalledWith("Pa", 100);

      input(textarea, "[[Pa");
      expect(setRawSpy).toHaveBeenCalledTimes(1);
      expect(autosize).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(100);
      expect(quickSwitch).toHaveBeenCalledTimes(1);

      input(textarea, "[[Par");
      expect(doc.byId["ime-composition"].raw).toBe("[[Par");
      expect(setRawSpy).toHaveBeenCalledTimes(2);
      expect(autosize).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(100);
      expect(quickSwitch).toHaveBeenLastCalledWith("Par", 100);
    } finally {
      dispose();
    }
  });

  it("normalizes a full-width reference once at composition end and retains it through immediate blur", () => {
    loadSingle(page(""));
    startEditing("ime-composition", 0);
    const { root, dispose } = mount(() => (
      <For each={pageByName("IME composition")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      setRawSpy.mockClear();

      compositionStart(textarea);
      composingInput(textarea, "【");
      composingInput(textarea, "【【");
      expect(setRawSpy).not.toHaveBeenCalled();
      expect(doc.byId["ime-composition"].raw).toBe("");

      compositionEnd(textarea);
      textarea.blur();

      expect(textarea.value).toBe("[[]]");
      expect(textarea.selectionStart).toBe(2);
      expect(doc.byId["ime-composition"].raw).toBe("[[]]");
      expect(setRawSpy).toHaveBeenCalledTimes(1);
    } finally {
      dispose();
    }
  });

  it("keeps a composing Enter out of autocomplete and structural handling until composition end", async () => {
    vi.useFakeTimers();
    vi.spyOn(backend(), "quickSwitch").mockResolvedValue([pageEntry("Parity Target")]);
    loadSingle(page("[[P]]"));
    startEditing("ime-composition", 3);
    const { root, dispose } = mount(() => (
      <For each={pageByName("IME composition")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      input(textarea, "[[P]]", 3);
      await vi.advanceTimersByTimeAsync(100);
      await Promise.resolve();
      expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Parity Target");
      setRawSpy.mockClear();
      expect(isDirty("IME composition")).toBe(false);

      compositionStart(textarea);
      composingInput(textarea, "[[Pa]]", 4);
      const event = composingEnter(textarea);

      expect(event.defaultPrevented).toBe(false);
      expect(textarea.value).toBe("[[Pa]]");
      expect(doc.byId["ime-composition"].raw).toBe("[[P]]");
      expect(pageByName("IME composition")?.roots).toEqual(["ime-composition"]);
      expect(setRawSpy).not.toHaveBeenCalled();
      expect(isDirty("IME composition")).toBe(false);
      expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Parity Target");

      compositionEnd(textarea);

      expect(doc.byId["ime-composition"].raw).toBe("[[Pa]]");
      expect(pageByName("IME composition")?.roots).toEqual(["ime-composition"]);
      expect(setRawSpy).toHaveBeenCalledTimes(1);
      expect(isDirty("IME composition")).toBe(true);
    } finally {
      dispose();
    }
  });

  it("uses legacy keyCode 229 to preserve the IME transaction when compositionstart is absent", async () => {
    vi.useFakeTimers();
    vi.spyOn(backend(), "quickSwitch").mockResolvedValue([pageEntry("Parity Target")]);
    loadSingle(page("[[P]]"));
    startEditing("ime-composition", 3);
    const { root, dispose } = mount(() => (
      <For each={pageByName("IME composition")?.roots ?? []}>{(id) => <Block id={id} />}</For>
    ));

    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      input(textarea, "[[P]]", 3);
      await vi.advanceTimersByTimeAsync(100);
      await Promise.resolve();
      expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Parity Target");
      setRawSpy.mockClear();
      expect(isDirty("IME composition")).toBe(false);

      composingInput(textarea, "[[Pa]]", 4);
      const event = legacyImeEnter(textarea);

      expect(event.defaultPrevented).toBe(false);
      expect(textarea.value).toBe("[[Pa]]");
      expect(doc.byId["ime-composition"].raw).toBe("[[P]]");
      expect(pageByName("IME composition")?.roots).toEqual(["ime-composition"]);
      expect(setRawSpy).not.toHaveBeenCalled();
      expect(isDirty("IME composition")).toBe(false);
      expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Parity Target");

      compositionEnd(textarea);

      expect(doc.byId["ime-composition"].raw).toBe("[[Pa]]");
      expect(pageByName("IME composition")?.roots).toEqual(["ime-composition"]);
      expect(setRawSpy).toHaveBeenCalledTimes(1);
      expect(isDirty("IME composition")).toBe(true);
    } finally {
      dispose();
    }
  });
});
