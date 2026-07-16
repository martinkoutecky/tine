import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { editingId, startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore, undo } from "../store";
import type { BlockDto } from "../types";
import {
  clearTransientLayersForTest,
  dismissTopTransient,
  registerTransientLayer,
} from "../transientLayers";
import { Block, Editor } from "./Block";

beforeAll(() => initParser());

afterEach(() => {
  clearTransientLayersForTest();
  installKeybindings()();
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function mountEditor(raw = "before selected after", format: "md" | "org" = "md") {
  const block: BlockDto = { id: "selection-wrap", raw, collapsed: false, children: [] };
  loadSingle({ name: "Wrap", kind: "page", title: "Wrap", pre_block: null, blocks: [block], format });
  startEditing(block.id, 0);
  const mounted = mount(() => (
    <For each={pageByName("Wrap")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ));
  return {
    ...mounted,
    textarea: mounted.root.querySelector("textarea.block-editor") as HTMLTextAreaElement,
  };
}

function keydown(textarea: HTMLTextAreaElement, init: KeyboardEventInit) {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  textarea.dispatchEvent(event);
  return event;
}

describe("selection wrapping with Alt-modified literal delimiters (GH #83)", () => {
  it("wraps twice with a literal Alt+[ and opens page completion", async () => {
    const { textarea, dispose } = mountEditor();
    try {
      textarea.setSelectionRange(7, 15);

      const first = keydown(textarea, { key: "[", code: "BracketLeft", altKey: true });
      expect(first.defaultPrevented).toBe(true);
      expect(textarea.value).toBe("before [selected] after");
      await vi.waitFor(() =>
        expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([8, 16])
      );

      const second = keydown(textarea, { key: "[", code: "BracketLeft", altKey: true });
      expect(second.defaultPrevented).toBe(true);
      expect(textarea.value).toBe("before [[selected]] after");
      await vi.waitFor(() =>
        expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([17, 17])
      );
      await vi.waitFor(() => expect(document.body.querySelector(".autocomplete")).not.toBeNull());
    } finally {
      dispose();
    }
  });

  it("does not reinterpret a layout-produced non-delimiter by physical key code", () => {
    const { textarea, dispose } = mountEditor();
    try {
      textarea.setSelectionRange(7, 15);
      const event = keydown(textarea, { key: "“", code: "BracketLeft", altKey: true });
      expect(event.defaultPrevented).toBe(false);
      expect(textarea.value).toBe("before selected after");
    } finally {
      dispose();
    }
  });

  it("gives an explicit Alt+[ editor binding precedence over incidental wrapping", () => {
    const disposeKeys = installKeybindings({ "editor/bold": "alt+[" });
    const { textarea, dispose } = mountEditor("selected");
    try {
      textarea.setSelectionRange(0, 8);
      const event = keydown(textarea, { key: "[", code: "BracketLeft", altKey: true });
      expect(event.defaultPrevented).toBe(true);
      expect(textarea.value).toBe("**selected**");
    } finally {
      dispose();
      disposeKeys();
    }
  });
});

describe("selection toolbar actions (GH #142)", () => {
  it("toggles page-link and inline-code wrappers while retaining the selection", async () => {
    const { textarea, root, dispose } = mountEditor("alpha beta");
    try {
      textarea.setSelectionRange(0, 5);
      textarea.dispatchEvent(new Event("select", { bubbles: true }));
      const pageLink = root.querySelector<HTMLButtonElement>('[data-selection-action="page-link"]');
      const inlineCode = root.querySelector<HTMLButtonElement>('[data-selection-action="inline-code"]');
      expect(pageLink).not.toBeNull();
      expect(inlineCode).not.toBeNull();

      pageLink!.click();
      await vi.waitFor(() => expect(textarea.value).toBe("[[alpha]] beta"));
      expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([2, 7]);
      pageLink!.click();
      await vi.waitFor(() => expect(textarea.value).toBe("alpha beta"));
      expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([0, 5]);

      inlineCode!.click();
      await vi.waitFor(() => expect(textarea.value).toBe("`alpha` beta"));
      expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([1, 6]);
    } finally {
      dispose();
    }
  });

  it("records a toolbar edit as one undoable editor change", async () => {
    const { textarea, root, dispose } = mountEditor("alpha beta");
    try {
      textarea.setSelectionRange(0, 5);
      textarea.dispatchEvent(new Event("select", { bubbles: true }));
      root.querySelector<HTMLButtonElement>('[data-selection-action="inline-code"]')!.click();
      await vi.waitFor(() => expect(doc.byId["selection-wrap"].raw).toBe("`alpha` beta"));
      undo();
      expect(doc.byId["selection-wrap"].raw).toBe("alpha beta");
    } finally {
      dispose();
    }
  });

  it.each([
    ["md", "bold", "**"],
    ["md", "italic", "*"],
    ["md", "strikethrough", "~~"],
    ["md", "highlight", "=="],
    ["org", "bold", "*"],
    ["org", "italic", "/"],
    ["org", "strikethrough", "+"],
    ["org", "highlight", "^^"],
  ] as const)("applies %s %s from the toolbar without wrapping the trailing space", async (format, action, delimiter) => {
    const { textarea, root, dispose } = mountEditor("before selected after", format);
    try {
      textarea.focus();
      textarea.setSelectionRange(7, 16, "backward");
      textarea.dispatchEvent(new Event("select", { bubbles: true }));
      root.querySelector<HTMLButtonElement>(`[data-selection-action="${action}"]`)!.click();
      await vi.waitFor(() => expect(textarea.value).toBe(`before ${delimiter}selected${delimiter} after`));
      expect([textarea.selectionStart, textarea.selectionEnd, textarea.selectionDirection]).toEqual([
        7 + delimiter.length,
        15 + delimiter.length,
        "backward",
      ]);
    } finally {
      dispose();
    }
  });
});

describe("selection formatting keyboard commands (GH #178)", () => {
  it.each([
    ["md", "b", false, "**"],
    ["md", "i", false, "*"],
    ["md", "s", true, "~~"],
    ["md", "h", true, "=="],
    ["org", "b", false, "*"],
    ["org", "i", false, "/"],
    ["org", "s", true, "+"],
    ["org", "h", true, "^^"],
  ] as const)("applies %s %s without moving the selected trailing space inside its delimiter", async (format, key, shiftKey, delimiter) => {
    const { textarea, dispose } = mountEditor("before selected after", format);
    try {
      textarea.focus();
      textarea.setSelectionRange(7, 16, "backward");
      const event = keydown(textarea, { key, code: `Key${key.toUpperCase()}`, ctrlKey: true, shiftKey });
      expect(event.defaultPrevented).toBe(true);
      await vi.waitFor(() => expect(textarea.value).toBe(`before ${delimiter}selected${delimiter} after`));
      expect([textarea.selectionStart, textarea.selectionEnd, textarea.selectionDirection]).toEqual([
        7 + delimiter.length,
        15 + delimiter.length,
        "backward",
      ]);
      expect(doc.byId["selection-wrap"].raw).toBe(`before ${delimiter}selected${delimiter} after`);
    } finally {
      dispose();
    }
  });
});

function openSelectionOverflow(
  root: ParentNode,
  textarea: HTMLTextAreaElement,
  start: number,
  end: number,
) {
  textarea.focus();
  textarea.setSelectionRange(start, end);
  textarea.dispatchEvent(new Event("select", { bubbles: true }));
  const more = root.querySelector<HTMLButtonElement>('.sel-toolbar-more');
  expect(more).not.toBeNull();
  more!.click();
  expect(root.querySelector('.sel-toolbar-overflow')).not.toBeNull();
}

function registerLower(id: string) {
  let dismissals = 0;
  const unregister = registerTransientLayer({
    id,
    dismiss: () => { dismissals += 1; return true; },
  });
  return { unregister, dismissals: () => dismissals };
}

describe("selection toolbar overflow transient ownership", () => {
  it.each(["escape", "back"] as const)(
    "owns one %s rung without losing the live editor selection",
    async (reason) => {
      const lower = registerLower(`selection-overflow-lower-${reason}`);
      const { textarea, root, dispose } = mountEditor("alpha beta gamma");
      try {
        openSelectionOverflow(root, textarea, 6, 10);

        expect(dismissTopTransient(reason)).toBe(true);
        await vi.waitFor(() => expect(root.querySelector('.sel-toolbar-overflow')).toBeNull());
        expect(lower.dismissals()).toBe(0);
        expect(document.activeElement).toBe(textarea);
        expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([6, 10]);
        expect(textarea.value).toBe("alpha beta gamma");
        expect(doc.byId["selection-wrap"].raw).toBe("alpha beta gamma");
        expect(editingId()).toBe("selection-wrap");

        expect(dismissTopTransient(reason)).toBe(true);
        expect(lower.dismissals()).toBe(1);
      } finally {
        lower.unregister();
        dispose();
      }
    },
  );

  it("drops a stale overflow when the selection empties and unregisters on unmount", async () => {
    const lower = registerLower("selection-overflow-cleanup-lower");
    const { textarea, root, dispose } = mountEditor("alpha beta gamma");
    openSelectionOverflow(root, textarea, 0, 5);

    textarea.setSelectionRange(5, 5);
    textarea.dispatchEvent(new Event("select", { bubbles: true }));
    await vi.waitFor(() => expect(root.querySelector('.sel-toolbar')).toBeNull());

    textarea.setSelectionRange(6, 10);
    textarea.dispatchEvent(new Event("select", { bubbles: true }));
    await vi.waitFor(() => expect(root.querySelector('.sel-toolbar')).not.toBeNull());
    expect(root.querySelector('.sel-toolbar-overflow')).toBeNull();
    expect(dismissTopTransient("escape")).toBe(true);
    expect(lower.dismissals()).toBe(1);

    openSelectionOverflow(root, textarea, 6, 10);
    dispose();
    expect(dismissTopTransient("back")).toBe(true);
    expect(lower.dismissals()).toBe(2);
    lower.unregister();
  });

  it("keeps two directly mounted editors as independent overflow owners", async () => {
    const blocks: BlockDto[] = [
      { id: "selection-a", raw: "alpha one", collapsed: false, children: [] },
      { id: "selection-b", raw: "beta two", collapsed: false, children: [] },
    ];
    loadSingle({ name: "Wrap", kind: "page", title: "Wrap", pre_block: null, blocks });
    const lower = registerLower("selection-overflow-instances-lower");
    const mounted = mount(() => <><Editor id="selection-a" /><Editor id="selection-b" /></>);
    try {
      const textareas = Array.from(mounted.root.querySelectorAll<HTMLTextAreaElement>("textarea.block-editor"));
      const wraps = Array.from(mounted.root.querySelectorAll<HTMLElement>(".editor-wrap"));
      openSelectionOverflow(wraps[0], textareas[0], 0, 5);
      openSelectionOverflow(wraps[1], textareas[1], 0, 4);
      expect(mounted.root.querySelectorAll('.sel-toolbar-overflow')).toHaveLength(2);

      expect(dismissTopTransient("escape")).toBe(true);
      await vi.waitFor(() => expect(mounted.root.querySelectorAll('.sel-toolbar-overflow')).toHaveLength(1));
      expect(lower.dismissals()).toBe(0);
      expect(document.activeElement).toBe(textareas[1]);
      expect([textareas[1].selectionStart, textareas[1].selectionEnd]).toEqual([0, 4]);

      expect(dismissTopTransient("back")).toBe(true);
      await vi.waitFor(() => expect(mounted.root.querySelectorAll('.sel-toolbar-overflow')).toHaveLength(0));
      expect(lower.dismissals()).toBe(0);
      expect(document.activeElement).toBe(textareas[0]);
      expect([textareas[0].selectionStart, textareas[0].selectionEnd]).toEqual([0, 5]);

      expect(dismissTopTransient("escape")).toBe(true);
      expect(lower.dismissals()).toBe(1);
    } finally {
      lower.unregister();
      mounted.dispose();
    }
  });
});
